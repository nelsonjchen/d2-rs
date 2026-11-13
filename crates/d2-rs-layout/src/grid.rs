// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Measured-graph grid layout corresponding to D2's `d2grid` package.

use d2_rs_core::stable_sort::StableSort;
use d2_rs_core::{
    CandidateReport, Graph, Insets, LabelPosition, LayoutError, LayoutOptions, LayoutReport,
    LayoutResult, NodeId, NodeLabelState, Point, Rect, Score, Size,
};
use std::collections::BTreeMap;

/// Default distance between grid cells in either axis.
pub const DEFAULT_GAP: f64 = 40.0;
/// Default padding between the grid and its container border.
pub const CONTAINER_PADDING: f64 = 60.0;
const LABEL_PADDING: f64 = 5.0;
const MAX_ICON_SIZE: f64 = 64.0;
const OVAL_ASPECT_LIMIT: f64 = 3.0;
const PERSON_ASPECT_LIMIT: f64 = 1.5;
const C4_PERSON_ASPECT_LIMIT: f64 = 1.5;
const CLOUD_WIDE_INNER_X: f64 = 0.085;
const CLOUD_WIDE_INNER_Y: f64 = 0.409;
const CLOUD_WIDE_INNER_WIDTH: f64 = 0.819;
const CLOUD_WIDE_INNER_HEIGHT: f64 = 0.548;
const CLOUD_TALL_INNER_X: f64 = 0.228;
const CLOUD_TALL_INNER_Y: f64 = 0.179;
const CLOUD_TALL_INNER_WIDTH: f64 = 0.549;
const CLOUD_TALL_INNER_HEIGHT: f64 = 0.820;
const CLOUD_SQUARE_INNER_X: f64 = 0.167;
const CLOUD_SQUARE_INNER_Y: f64 = 0.335;
const CLOUD_SQUARE_INNER_WIDTH: f64 = 0.663;
const CLOUD_SQUARE_INNER_HEIGHT: f64 = 0.663;
const CLOUD_WIDE_ASPECT_BOUNDARY: f64 =
    (1.0 + CLOUD_WIDE_INNER_WIDTH / CLOUD_WIDE_INNER_HEIGHT) / 2.0;
const CLOUD_TALL_ASPECT_BOUNDARY: f64 =
    (1.0 + CLOUD_TALL_INNER_WIDTH / CLOUD_TALL_INNER_HEIGHT) / 2.0;
const DOCUMENT_PATH_HEIGHT: f64 = 18.925;
const DOCUMENT_INNER_BOTTOM: f64 = 14.0;
const PAGE_CORNER_WIDTH: f64 = 20.8164;
const PAGE_CORNER_HEIGHT: f64 = 20.348;
const PACKAGE_TOP_MAX_HEIGHT: f64 = 55.0;
const PACKAGE_VERTICAL_SCALAR: f64 = 0.2;
const STEP_WEDGE_WIDTH: f64 = 35.0;
const PARALLELOGRAM_WEDGE_WIDTH: f64 = 26.0;
const STORED_DATA_WEDGE_WIDTH: f64 = 15.0;
const DEFAULT_ARC_DEPTH: f64 = 24.0;
const CALLOUT_TIP_HEIGHT: f64 = 45.0;
const C4_HEAD_RADIUS_FACTOR: f64 = 0.22;
const C4_BODY_TOP_FACTOR: f64 = 0.8;
// These bounds are part of d2grid's observable search behavior.  Keep them
// here, rather than replacing the bounded search with an unbounded optimizer:
// the first acceptable division and the attempt budget affect the chosen
// layout for large or uneven diagrams.
const GRID_STARTING_THRESHOLD: f64 = 1.2;
const GRID_THRESHOLD_STEP: f64 = 0.25;
const GRID_MIN_THRESHOLD_ATTEMPTS: usize = 1;
const GRID_MAX_THRESHOLD_ATTEMPTS: usize = 3;
const GRID_ATTEMPT_LIMIT: usize = 100_000;
const GRID_SKIP_LIMIT: usize = 10_000_000;

#[derive(Clone, Copy, Debug)]
struct GridScopeConfig {
    rows: usize,
    columns: usize,
    row_directed: bool,
    dynamic_axis: Option<DynamicAxis>,
    grid_gap: Option<f64>,
    vertical_gap: Option<f64>,
    horizontal_gap: Option<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DynamicAxis {
    Rows,
    Columns,
}

/// Applies every nested grid scope to an already laid-out graph.
///
/// D2 lays out nested diagrams before their containing grid.  The shared Rust
/// graph keeps all scopes in one arena, so this pass walks grid containers from
/// deepest to shallowest, moves their direct children and descendants as one
/// unit, and refreshes routes touched by each move.  The flat adapter remains
/// the cheaper path for a graph that consists of one grid scope only.
pub fn apply_scopes(
    graph: &Graph,
    result: LayoutResult,
    options: &LayoutOptions,
) -> Result<LayoutResult, LayoutError> {
    apply_scopes_except(graph, result, options, |_| false)
}

/// [`apply_scopes`] without the grid scopes for which `settled` holds.
///
/// A grid nested inside an extracted grid-cell container is laid out by that
/// cell's own `LayoutNested` call, together with the cell's edges; the
/// enclosing grid only moves the cell's graph as one unit (`PositionNested`).
pub(crate) fn apply_scopes_except(
    graph: &Graph,
    mut result: LayoutResult,
    _options: &LayoutOptions,
    settled: impl Fn(NodeId) -> bool,
) -> Result<LayoutResult, LayoutError> {
    validate(graph)?;
    let mut scopes = graph
        .nodes()
        .filter_map(|(id, node)| {
            (node.grid_rows.is_some() || node.grid_columns.is_some())
                .then_some(id)
                .filter(|_| !belongs_to_sequence_diagram(graph, id) && !settled(id))
        })
        .collect::<Vec<_>>();
    scopes.stable_sort_by_key(|id| std::cmp::Reverse(depth(graph, *id)));
    // Sequence cells have already supplied their fitted geometry, but Source
    // leaves their root attributes detached until after the parent's grid
    // pass. Preserve that pending state for InjectNested instead of exposing
    // it to the placeholder's label/icon defaults during packing.
    let mut detached_root_labels = graph
        .nodes()
        .filter(|(id, node)| {
            graph.is_sequence_diagram(*id)
                && graph.nodes().any(|(_, child)| child.parent == Some(*id))
                && node.parent.map_or_else(
                    || graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some(),
                    |parent| {
                        graph.node(parent).is_some_and(|parent| {
                            parent.grid_rows.is_some() || parent.grid_columns.is_some()
                        })
                    },
                )
        })
        .filter_map(|(id, _)| {
            result
                .node_labels
                .get(&id)
                .copied()
                .map(|label| (id, label))
        })
        .collect::<BTreeMap<_, _>>();
    for scope in scopes {
        let children = ordered_children(graph, Some(scope));
        if children.is_empty() {
            continue;
        }
        // LayoutNested extracts each nonempty object grid before laying out
        // the containing graph. A synthetic root grid itself is not extracted.
        result.source_edge_storage.record_prior_extraction();
        let node = graph.node(scope).expect("grid scope exists");
        let spacing = graph.grid_spacing(scope);
        let row_directed = graph
            .grid_row_directed
            .get(scope.0 as usize)
            .and_then(|value| *value)
            .unwrap_or(true);
        let (rows, columns) = grid_dimensions(
            node.grid_rows,
            node.grid_columns,
            children.len(),
            row_directed,
        )?;
        let dynamic_axis = match (node.grid_rows, node.grid_columns) {
            (Some(_), None) => Some(DynamicAxis::Rows),
            (None, Some(_)) => Some(DynamicAxis::Columns),
            _ => None,
        };
        apply_scope(
            graph,
            &mut result,
            Some(scope),
            children.clone(),
            GridScopeConfig {
                rows,
                columns,
                row_directed,
                dynamic_axis,
                grid_gap: spacing.grid_gap,
                vertical_gap: spacing.vertical_gap,
                horizontal_gap: spacing.horizontal_gap,
            },
        );
        if let Some(label) = result.node_labels.get(&scope).copied() {
            detached_root_labels.insert(scope, label);
        }
        #[cfg(feature = "diagnostic-traces")]
        trace_grid_snapshot(graph, &result, Some(scope), "grid");
        #[cfg(feature = "diagnostic-traces")]
        if super::diagnostic_env_enabled("DAGRE_DEBUG_NESTED") {
            eprintln!("GRID_SCOPE {:?} boxes={:?}", scope, result.boxes);
        }
    }

    // The source document root is implicit in Graph.  Apply its grid after
    // object scopes so each nested diagram has an accurate cell size first.
    if !graph.root_sequence_diagram
        && (graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some())
    {
        let children = ordered_children(graph, None);
        if !children.is_empty() {
            let row_directed = graph.root_grid_row_directed.unwrap_or(true);
            let (rows, columns) = grid_dimensions(
                graph.root_grid_rows,
                graph.root_grid_columns,
                children.len(),
                row_directed,
            )?;
            let dynamic_axis = match (graph.root_grid_rows, graph.root_grid_columns) {
                (Some(_), None) => Some(DynamicAxis::Rows),
                (None, Some(_)) => Some(DynamicAxis::Columns),
                _ => None,
            };
            apply_scope(
                graph,
                &mut result,
                None,
                children,
                GridScopeConfig {
                    rows,
                    columns,
                    row_directed,
                    dynamic_axis,
                    grid_gap: graph.root_grid_gap,
                    vertical_gap: graph.root_vertical_gap,
                    horizontal_gap: graph.root_horizontal_gap,
                },
            );
            #[cfg(feature = "diagnostic-traces")]
            trace_grid_snapshot(graph, &result, None, "grid");
            #[cfg(feature = "diagnostic-traces")]
            if super::diagnostic_env_enabled("DAGRE_DEBUG_NESTED") {
                eprintln!("GRID_SCOPE root boxes={:?}", result.boxes);
            }
        }
    }

    // Nested d2grid calls return their child dimensions before the enclosing
    // graph is fitted. Recompute every ordinary container from the committed
    // child rectangles so a grid's growth propagates through non-grid parents
    // exactly like d2graph's SizeToContent pass.
    // InjectNested restores the detached root label after parent-grid packing.
    result.node_labels.extend(detached_root_labels);
    fit_grid_containers(graph, &mut result);

    // D2's grid engine owns only direct edges between the cells of the grid.
    // Routes inside a cell were produced by that cell's layout pass and move
    // with the cell; replacing them here with center-to-center lines loses
    // the compact cubic representation emitted by Dagre.
    Ok(result)
}

/// Emits bit-exact grid-scope state for comparison with the instrumented
/// D2 v0.9.0 adapter. This code and its argument construction are absent from
/// ordinary builds.
#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_grid_snapshot(
    graph: &Graph,
    result: &LayoutResult,
    scope: Option<NodeId>,
    stage: &str,
) {
    if !super::diagnostic_env_enabled("D2_TRACE_GRID_STATE") {
        return;
    }
    let scope_id = scope
        .and_then(|id| {
            graph
                .node(id)
                .map(|node| normalized_grid_id(&node.external_id))
        })
        .unwrap_or_else(|| "root".to_owned());
    let scope_hex = hex_bytes(scope_id.as_bytes());
    let frame_origin = scope
        .and_then(|id| result.boxes.get(&id).map(|rect| rect.origin))
        .unwrap_or_default();
    let in_scope = |id: NodeId| {
        scope.map_or_else(
            || {
                stage.starts_with("layout-final")
                    || graph.node(id).is_some_and(|node| node.parent.is_none())
            },
            |scope| {
                graph
                    .node(id)
                    .is_some_and(|node| node.parent == Some(scope))
            },
        )
    };

    let mut nodes = graph
        .nodes()
        .filter(|(id, _)| in_scope(*id))
        .filter_map(|(id, node)| {
            result.boxes.get(&id).map(|rect| {
                (
                    normalized_grid_node_id(graph, &node.external_id, scope),
                    (rect.origin.x - frame_origin.x).to_bits(),
                    (rect.origin.y - frame_origin.y).to_bits(),
                    rect.size.width.to_bits(),
                    rect.size.height.to_bits(),
                )
            })
        })
        .collect::<Vec<_>>();
    nodes.stable_sort_by(|left, right| left.0.cmp(&right.0));
    for (id, x, y, width, height) in nodes {
        eprintln!(
            "GRID_STATE stage={stage} scope={scope_hex} node={} rect={x:016x},{y:016x},{width:016x},{height:016x}",
            hex_bytes(id.as_bytes())
        );
    }

    let mut labels = graph
        .nodes()
        .filter(|(id, _)| in_scope(*id))
        .filter_map(|(id, node)| {
            (has_label(graph, id) || node.has_icon)
                .then(|| result.node_labels.get(&id))
                .flatten()
                .filter(|label| label.position != LabelPosition::Unset)
                .map(|label| {
                    (
                        normalized_grid_node_id(graph, &node.external_id, scope),
                        label.position,
                    )
                })
        })
        .collect::<Vec<_>>();
    labels.stable_sort_by(|left, right| left.0.cmp(&right.0));
    for (id, position) in labels {
        eprintln!(
            "GRID_LABEL stage={stage} scope={scope_hex} node={} position={}",
            hex_bytes(id.as_bytes()),
            normalized_label_position(position)
        );
    }

    let mut edge_ordinals = BTreeMap::<(String, String), usize>::new();
    let mut routes = Vec::new();
    for (edge_id, edge) in graph.edges() {
        if !in_scope(edge.source) || !in_scope(edge.target) {
            continue;
        }
        let (Some(source), Some(target)) = (graph.node(edge.source), graph.node(edge.target))
        else {
            continue;
        };
        let source_id = normalized_grid_node_id(graph, &source.external_id, scope);
        let target_id = normalized_grid_node_id(graph, &target.external_id, scope);
        let pair = (source_id.clone(), target_id.clone());
        let ordinal = edge_ordinals.entry(pair).or_default();
        let Some(points) = result.routes.get(&edge_id) else {
            *ordinal += 1;
            continue;
        };
        let encoded_points = points
            .iter()
            .map(|point| {
                format!(
                    "{:016x},{:016x}",
                    (point.x - frame_origin.x).to_bits(),
                    (point.y - frame_origin.y).to_bits()
                )
            })
            .collect::<Vec<_>>()
            .join(";");
        routes.push((source_id, target_id, *ordinal, encoded_points));
        *ordinal += 1;
    }
    for line in &result.sequence_lifelines {
        if !in_scope(line.actor) {
            continue;
        }
        let Some(actor) = graph.node(line.actor) else {
            continue;
        };
        let source = normalized_grid_node_id(graph, &actor.external_id, scope);
        let points = line
            .route
            .iter()
            .map(|point| {
                format!(
                    "{:016x},{:016x}",
                    (point.x - frame_origin.x).to_bits(),
                    (point.y - frame_origin.y).to_bits()
                )
            })
            .collect::<Vec<_>>()
            .join(";");
        routes.push((source, line.target_id.clone(), 0, points));
    }
    routes.stable_sort();
    for (source, target, ordinal, points) in routes {
        eprintln!(
            "GRID_ROUTE stage={stage} scope={scope_hex} edge={}>{}#{ordinal} points={points}",
            hex_bytes(source.as_bytes()),
            hex_bytes(target.as_bytes())
        );
    }
}

#[cfg(feature = "diagnostic-traces")]
fn normalized_label_position(position: LabelPosition) -> String {
    let mut normalized = String::new();
    for character in format!("{position:?}").chars() {
        if character.is_ascii_uppercase() && !normalized.is_empty() {
            normalized.push('_');
        }
        normalized.push(character.to_ascii_uppercase());
    }
    normalized
}

/// Emits the final routes that cross a nested grid extraction boundary. The
/// pinned Go adapter observes these after the extracted-edge callback; keep this hook at the
/// selected-edge callback boundary, before local routes move into their parent.
#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_external_routes(
    graph: &Graph,
    result: &LayoutResult,
    selected: &[d2_rs_tala::EdgeId],
    scope_prefix: Option<&str>,
    callback_site: &str,
) {
    if !super::diagnostic_env_enabled("D2_TRACE_GRID_STATE") {
        return;
    }
    // The extracted graph has a detached root. Match that actual identity view
    // while retaining the callback's coordinates, before parent reinjection.
    if std::env::var("D2_TRACE_GRID_CALLBACK").as_deref() == Ok("1") {
        eprintln!(
            "GRID_CALLBACK site={} prefix={:?} nodes={} edges={} selected={:?}",
            callback_site,
            scope_prefix,
            graph.nodes.len(),
            graph.edges.len(),
            selected
        );
    }
    let prefix = scope_prefix.map(|scope| format!("{scope}."));
    let local = |id: &str| {
        normalized_grid_id(
            prefix
                .as_ref()
                .and_then(|prefix| id.strip_prefix(prefix))
                .unwrap_or(id),
        )
    };
    let mut ordinals = BTreeMap::<(String, String, String), usize>::new();
    let mut records = Vec::new();
    for edge_id in selected {
        let Some(edge) = graph.edges.get(edge_id.0 as usize) else {
            continue;
        };
        let (Some(source), Some(target), Some(points)) = (
            graph.node(edge.source),
            graph.node(edge.target),
            result.routes.get(edge_id),
        ) else {
            continue;
        };
        let mut scope = "root".to_owned();
        let mut parent = source.parent;
        while let Some(id) = parent {
            let Some(node) = graph.node(id) else {
                break;
            };
            if node.grid_rows.is_some() || node.grid_columns.is_some() {
                scope = local(&node.external_id);
                if scope.is_empty() {
                    scope = "root".to_owned();
                }
                break;
            }
            parent = node.parent;
        }
        let source_id = local(&source.external_id);
        let target_id = local(&target.external_id);
        let ordinal = ordinals
            .entry((scope.clone(), source_id.clone(), target_id.clone()))
            .or_default();
        let points = points
            .iter()
            .map(|point| format!("{:016x},{:016x}", point.x.to_bits(), point.y.to_bits()))
            .collect::<Vec<_>>()
            .join(";");
        records.push((scope, source_id, target_id, *ordinal, points));
        *ordinal += 1;
    }
    records.stable_sort();
    for (scope, source, target, ordinal, points) in records {
        eprintln!(
            "GRID_ROUTE stage=external-route scope={} edge={}>{}#{ordinal} points={points}",
            hex_bytes(scope.as_bytes()),
            hex_bytes(source.as_bytes()),
            hex_bytes(target.as_bytes())
        );
    }
}

#[cfg(feature = "diagnostic-traces")]
fn normalized_grid_id(id: &str) -> String {
    id.strip_prefix("b.").unwrap_or(id).to_owned()
}

#[cfg(feature = "diagnostic-traces")]
fn normalized_grid_node_id(graph: &Graph, id: &str, scope: Option<NodeId>) -> String {
    let normalized = normalized_grid_id(id);
    let Some(scope) = scope.and_then(|id| graph.node(id).map(|node| node.external_id.as_str()))
    else {
        return normalized;
    };
    let normalized_scope = normalized_grid_id(scope);
    normalized
        .strip_prefix(&format!("{normalized_scope}."))
        .unwrap_or(&normalized)
        .to_owned()
}

#[cfg(feature = "diagnostic-traces")]
fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn fit_grid_containers(graph: &Graph, result: &mut LayoutResult) {
    let mut containers = graph
        .nodes()
        .filter_map(|(id, _)| {
            graph
                .nodes()
                .any(|(_, child)| child.parent == Some(id))
                .then_some(id)
        })
        .collect::<Vec<_>>();
    containers.sort_by_key(|id| std::cmp::Reverse(depth(graph, *id)));
    for container in containers {
        let Some(node) = graph.node(container) else {
            continue;
        };
        // A grid scope was sized by apply_scope from its cells plus its own
        // padding. Recomputing it from child rectangles here adds that
        // padding a second time (the nested_layout_bug witness grows 306 to
        // 342). Only ordinary containers that contain a grid descendant need
        // this propagation pass.
        if node.grid_rows.is_some() || node.grid_columns.is_some() {
            continue;
        }
        // Ordinary containers keep TALA's own fitted dimensions. Only a
        // parent that contains a grid scope needs this propagation pass; the
        // grid's temporary cells have already replaced that descendant's
        // dimensions and must be reflected upward.
        if !contains_grid_scope(graph, container) {
            continue;
        }
        // Ordinary grid cells are extracted and fitted by LayoutNested before
        // this enclosing grid runs. Their descendant containers already have
        // engine-owned dimensions; fitting them again adds padding twice.
        let mut owner = Some(container);
        let mut extracted_cell = false;
        while let Some(id) = owner {
            if super::is_grid_cell_container(graph, id) {
                extracted_cell = true;
                break;
            }
            owner = graph.node(id).and_then(|node| node.parent);
        }
        if extracted_cell {
            continue;
        }
        let Some(parent_rect) = result.boxes.get(&container).copied() else {
            continue;
        };
        let children = graph
            .nodes()
            .filter_map(|(id, child)| (child.parent == Some(container)).then_some(id));
        let mut max_right = 0.0_f64;
        let mut max_bottom = 0.0_f64;
        for child in children {
            let Some(rect) = result.boxes.get(&child).copied() else {
                continue;
            };
            max_right = max_right.max(rect.right() - parent_rect.origin.x);
            max_bottom = max_bottom.max(rect.bottom() - parent_rect.origin.y);
        }
        let desired = Size {
            width: max_right + node.content_insets.right,
            height: max_bottom + node.content_insets.bottom,
        };
        if let Some(rect) = result.boxes.get_mut(&container) {
            if !node.fixed_width && desired.width > 0.0 {
                rect.size.width = desired.width;
            }
            if !node.fixed_height && desired.height > 0.0 {
                rect.size.height = desired.height;
            }
        }
    }
}

fn contains_grid_scope(graph: &Graph, id: NodeId) -> bool {
    graph.nodes().any(|(child_id, child)| {
        child.parent == Some(id)
            && !belongs_to_sequence_diagram(graph, child_id)
            // LayoutNested leaves childless grids in the parent engine
            // (d2layouts.go:211-216). They never resize a nested scope, so
            // their ancestors must keep the engine's fitted dimensions.
            && (((child.grid_rows.is_some() || child.grid_columns.is_some())
                && graph.nodes().any(|(_, node)| node.parent == Some(child_id)))
                || contains_grid_scope(graph, child_id))
    })
}

fn belongs_to_sequence_diagram(graph: &Graph, mut id: NodeId) -> bool {
    loop {
        if graph.is_sequence_diagram(id) {
            return true;
        }
        let Some(parent) = graph.node(id).and_then(|node| node.parent) else {
            return false;
        };
        id = parent;
    }
}

fn depth(graph: &Graph, id: NodeId) -> usize {
    let mut depth = 0;
    let mut parent = graph.node(id).and_then(|node| node.parent);
    while let Some(id) = parent {
        depth += 1;
        parent = graph.node(id).and_then(|node| node.parent);
    }
    depth
}

fn ordered_children(graph: &Graph, parent: Option<NodeId>) -> Vec<NodeId> {
    if let Some(order) = graph.hierarchy_children_order.get(&parent) {
        return order.clone();
    }
    graph
        .nodes()
        .filter_map(|(id, node)| (node.parent == parent).then_some(id))
        .collect()
}

fn grid_dimensions(
    configured_rows: Option<usize>,
    configured_columns: Option<usize>,
    child_count: usize,
    row_directed: bool,
) -> Result<(usize, usize), LayoutError> {
    // d2grid/grid_diagram.go:newGridDiagram validates before multiplying or
    // allocating. Language validation does not protect direct Graph callers.
    for (name, dimension) in [
        ("grid-rows", configured_rows),
        ("grid-columns", configured_columns),
    ] {
        if let Some(value) = dimension {
            if value == 0 {
                return Err(LayoutError::D2GraphValidation(format!(
                    "invalid {name} \"0\""
                )));
            }
            if value > d2_rs_core::grid::MAX_GRID_DIMENSION as usize {
                return Err(LayoutError::D2GraphValidation(format!(
                    "{name} {value} exceeds the maximum of {}",
                    d2_rs_core::grid::MAX_GRID_DIMENSION,
                )));
            }
        }
    }
    if child_count > d2_rs_core::grid::MAX_GRID_CELLS as usize {
        return Err(LayoutError::D2GraphValidation(format!(
            "grid object count {child_count} exceeds the limit of {} cells",
            d2_rs_core::grid::MAX_GRID_CELLS,
        )));
    }
    let mut rows = configured_rows.unwrap_or(0);
    let mut columns = configured_columns.unwrap_or(0);
    if rows == 0 && columns == 0 {
        columns = child_count.max(1);
        rows = child_count.div_ceil(columns);
    } else if rows == 0 {
        columns = columns.max(1);
        rows = child_count.div_ceil(columns);
    } else if columns == 0 {
        rows = rows.min(child_count.max(1));
        columns = child_count.div_ceil(rows);
    } else {
        let capacity = d2_rs_core::grid::grid_capacity(rows as i64, columns as i64)
            .map_err(LayoutError::D2GraphValidation)? as usize;
        if capacity < child_count {
            if row_directed {
                rows = child_count.div_ceil(columns);
            } else {
                columns = child_count.div_ceil(rows);
            }
            d2_rs_core::grid::grid_capacity(rows as i64, columns as i64).map_err(|error| {
                LayoutError::D2GraphValidation(format!("expanded grid: {error}"))
            })?;
        }
    }
    Ok((rows.max(1), columns.max(1)))
}

// d2grid/layout.go:occupiedDimensions. Empty configured cells contribute
// gaps to the diagram extent, but need no temporary row/column storage.
fn grid_storage_dimensions(
    rows: usize,
    columns: usize,
    count: usize,
    row_directed: bool,
) -> (usize, usize) {
    if count == 0 {
        (0, 0)
    } else if row_directed {
        (count.div_ceil(columns), count.min(columns))
    } else {
        (count.min(rows), count.div_ceil(rows))
    }
}

#[cfg(test)]
#[path = "grid_capacity_api_tests.rs"]
mod capacity_api_tests;

#[cfg(test)]
#[path = "grid_search_api_tests.rs"]
mod search_api_tests;

#[cfg(test)]
#[path = "grid_measurement_api_tests.rs"]
mod measurement_api_tests;

fn apply_scope(
    graph: &Graph,
    result: &mut LayoutResult,
    scope: Option<NodeId>,
    children: Vec<NodeId>,
    config: GridScopeConfig,
) {
    let margins = prepare_grid_objects(graph, result, scope, &children);
    if let Some(axis) = config.dynamic_axis {
        apply_dynamic_scope(graph, result, scope, children, config, axis, margins);
        return;
    }
    // The document root is synthetic in D2 and does not draw a container
    // border. Object grid scopes retain the 60px content inset, while the
    // root grid starts its first cell at the canvas origin.
    // d2grid reuses the configured gap as the container padding whenever a
    // horizontal/vertical gap (or grid-gap) is explicitly present. This is
    // why the zero-gap nested label-position fixture has no 60px inset.
    let horizontal_padding = scope.map_or(0.0, |_| {
        config
            .horizontal_gap
            .or(config.grid_gap)
            .unwrap_or(CONTAINER_PADDING)
            .max(0.0)
    });
    let vertical_padding = scope.map_or(0.0, |_| {
        config
            .vertical_gap
            .or(config.grid_gap)
            .unwrap_or(CONTAINER_PADDING)
            .max(0.0)
    });
    let horizontal_gap = config
        .horizontal_gap
        .or(config.grid_gap)
        .unwrap_or(DEFAULT_GAP)
        .max(0.0);
    let vertical_gap = config
        .vertical_gap
        .or(config.grid_gap)
        .unwrap_or(DEFAULT_GAP)
        .max(0.0);
    let container = scope.and_then(|id| result.boxes.get(&id).copied());
    // d2layouts extracts a sole root grid as a diagram before invoking the
    // core layout engine. Its container is therefore anchored at the
    // synthetic canvas origin even if the preliminary Dagre pass placed the
    // placeholder at its normal 10/20 source margin. Nested grids retain the
    // parent-relative origin from that pass.
    let root_grid_diagram = scope.is_some_and(|id| {
        graph.node(id).is_some_and(|node| node.parent.is_none())
            && graph
                .nodes()
                .filter(|(_, node)| node.parent.is_none())
                .count()
                == 1
    });
    let origin = if root_grid_diagram {
        Point::default()
    } else {
        container.map_or_else(Point::default, |rect| rect.origin)
    };
    if root_grid_diagram
        && let Some(scope) = scope
        && let Some(rect) = result.boxes.get_mut(&scope)
    {
        rect.origin = Point::default();
    }
    let (storage_rows, storage_columns) = grid_storage_dimensions(
        config.rows,
        config.columns,
        children.len(),
        config.row_directed,
    );
    let mut column_widths = vec![0.0_f64; storage_columns];
    let mut row_heights = vec![0.0_f64; storage_rows];
    for (index, child) in children.iter().copied().enumerate() {
        let Some(rect) = result.boxes.get(&child).copied() else {
            continue;
        };
        let (row, column) = grid_cell(index, config.rows, config.columns, config.row_directed);
        column_widths[column] = column_widths[column].max(rect.size.width);
        row_heights[row] = row_heights[row].max(rect.size.height);
    }
    let content_width = column_widths.iter().sum::<f64>()
        + horizontal_gap * config.columns.saturating_sub(1) as f64;
    let content_height =
        row_heights.iter().sum::<f64>() + vertical_gap * config.rows.saturating_sub(1) as f64;
    let content_padding = scope
        .map(|id| {
            grid_scope_padding(
                graph,
                result,
                id,
                horizontal_padding,
                vertical_padding,
                Size {
                    width: content_width,
                    height: content_height,
                },
            )
        })
        .unwrap_or_else(|| Insets::uniform(0.0));
    let desired_size = Size {
        width: content_width + content_padding.left + content_padding.right,
        height: content_height + content_padding.top + content_padding.bottom,
    };
    let existing_size = scope
        .and_then(|id| result.boxes.get(&id).map(|rect| rect.size))
        .unwrap_or(desired_size);
    let fitted_size = scope
        .and_then(|id| graph.node(id))
        .map_or(desired_size, |node| {
            grid_shape_size(node, desired_size, existing_size)
        });
    if let Some(scope) = scope
        && let Some(rect) = result.boxes.get_mut(&scope)
    {
        // Keep authored fixed dimensions explicit.  The preceding parent
        // TALA pass may have fitted a shaped placeholder around its
        // temporary children; leaving that box untouched would leak the
        // pre-extraction size into the nested grid result.
        rect.size.width = fitted_size.width;
        rect.size.height = fitted_size.height;
    }

    let mut deltas = BTreeMap::new();
    let mut y = origin.y + content_padding.top;
    for (row, row_height) in row_heights.iter().copied().enumerate() {
        let mut x = origin.x + content_padding.left;
        for (column, column_width) in column_widths.iter().copied().enumerate() {
            let index = grid_index(
                row,
                column,
                config.rows,
                config.columns,
                config.row_directed,
            );
            let Some(child) = children.get(index).copied() else {
                x += column_width + horizontal_gap;
                continue;
            };
            let Some(previous) = result.boxes.get(&child).copied() else {
                x += column_width + horizontal_gap;
                continue;
            };
            let node = graph.node(child).expect("grid child exists");
            let mut target = Point { x, y };
            if let Some(locked) = node.locked_position {
                target = locked;
            }
            if let Some(constrained_x) = node.constrained_x {
                target.x = constrained_x;
            }
            if let Some(constrained_y) = node.constrained_y {
                target.y = constrained_y;
            }
            // d2grid's even layout gives every cell the maximum width and
            // height of its row/column. This is observable for a short object
            // beside a tall nested grid (the source nested-layout witness).
            if let Some(rect) = result.boxes.get_mut(&child) {
                rect.size.width = column_width;
                rect.size.height = row_height;
            }
            let delta = Point {
                x: target.x - previous.origin.x,
                y: target.y - previous.origin.y,
            };
            if delta.x != 0.0 || delta.y != 0.0 {
                shift_subtree(graph, result, child, delta, &mut deltas);
            }
            x += column_width + horizontal_gap;
        }
        y += row_height + vertical_gap;
    }

    // `d2grid` lays cells out in the rectangular grid first, then asks the
    // container shape where that rectangle belongs in its actual inner box.
    // The additional translation is observable for every non-rectangular
    // shape (and for cloud's content-aspect-dependent inner box).
    if let Some(scope) = scope.and_then(|id| graph.node(id)) {
        let shape_shift = grid_shape_shift(scope.shape, fitted_size, desired_size);
        if shape_shift.x != 0.0 || shape_shift.y != 0.0 {
            for child in children.iter().copied() {
                shift_subtree(graph, result, child, shape_shift, &mut deltas);
            }
        }
    }

    if scope.is_none() {
        let (state, shift) = super::root_state::grid_root(
            graph,
            Size {
                width: content_width,
                height: content_height,
            },
            config
                .horizontal_gap
                .or(config.grid_gap)
                .unwrap_or(CONTAINER_PADDING)
                .max(0.0),
            config
                .vertical_gap
                .or(config.grid_gap)
                .unwrap_or(CONTAINER_PADDING)
                .max(0.0),
        );
        result.synthetic_root = Some(state);
        if shift.x != 0.0 || shift.y != 0.0 {
            for child in children.iter().copied() {
                shift_subtree(graph, result, child, shift, &mut deltas);
            }
        }
    }
    finish_grid_objects(graph, result, &children, &margins, &mut deltas);
    apply_grid_label_states(graph, result, scope, &children);
    if scope.is_none() {
        update_grid_routes(graph, result, scope, &deltas);
    }

    for (edge_id, edge) in graph.edges() {
        let direct_child_edge = scope.is_some_and(|scope| {
            graph.node(edge.source).and_then(|node| node.parent) == Some(scope)
                && graph.node(edge.target).and_then(|node| node.parent) == Some(scope)
        });
        if direct_child_edge {
            if let (Some(source), Some(target)) = (
                result.boxes.get(&edge.source),
                result.boxes.get(&edge.target),
            ) {
                result.routes.insert(
                    edge_id,
                    grid_edge_route(graph, edge_id, *source, *target, &result.node_labels),
                );
                set_grid_edge_label(graph, result, edge_id);
            }
            continue;
        }
        // Refresh inter-cell routes after grid placement so the result does
        // not retain a pre-grid corridor at stale rank coordinates. TALA's
        // orchestration replaces nested-container crossings with its selected
        // edge router after the scope has been assembled.
        let Some(scope) = scope else {
            continue;
        };
        let source_cell = direct_grid_child(graph, edge.source, scope);
        let target_cell = direct_grid_child(graph, edge.target, scope);
        if source_cell.is_none() || source_cell == target_cell {
            continue;
        }
        if let (Some(source), Some(target)) = (
            result.boxes.get(&edge.source),
            result.boxes.get(&edge.target),
        ) {
            result.routes.insert(
                edge_id,
                grid_edge_route(graph, edge_id, *source, *target, &result.node_labels),
            );
        }
    }
}

pub(crate) fn direct_grid_child(graph: &Graph, id: NodeId, scope: NodeId) -> Option<NodeId> {
    let mut current = id;
    while let Some(node) = graph.node(current) {
        if node.parent == Some(scope) {
            return Some(current);
        }
        current = node.parent?;
    }
    None
}

/// Applies D2's one-axis grid search. When only rows or only columns is
/// supplied, d2grid partitions the ordered children into groups whose measured
/// widths/heights are closest to an even target; it does not simply fill a
/// rectangular row-major matrix.
fn apply_dynamic_scope(
    graph: &Graph,
    result: &mut LayoutResult,
    scope: Option<NodeId>,
    children: Vec<NodeId>,
    config: GridScopeConfig,
    axis: DynamicAxis,
    margins: BTreeMap<NodeId, Insets>,
) {
    let horizontal_gap = config
        .horizontal_gap
        .or(config.grid_gap)
        .unwrap_or(DEFAULT_GAP)
        .max(0.0);
    let vertical_gap = config
        .vertical_gap
        .or(config.grid_gap)
        .unwrap_or(DEFAULT_GAP)
        .max(0.0);
    let horizontal_padding = scope.map_or(0.0, |_| {
        config
            .horizontal_gap
            .or(config.grid_gap)
            .unwrap_or(CONTAINER_PADDING)
            .max(0.0)
    });
    let vertical_padding = scope.map_or(0.0, |_| {
        config
            .vertical_gap
            .or(config.grid_gap)
            .unwrap_or(CONTAINER_PADDING)
            .max(0.0)
    });
    let container = scope.and_then(|id| result.boxes.get(&id).copied());
    let root_grid_diagram = scope.is_some_and(|id| {
        graph.node(id).is_some_and(|node| node.parent.is_none())
            && graph
                .nodes()
                .filter(|(_, node)| node.parent.is_none())
                .count()
                == 1
    });
    let origin = if root_grid_diagram {
        Point::default()
    } else {
        container.map_or_else(Point::default, |rect| rect.origin)
    };
    if root_grid_diagram
        && let Some(scope) = scope
        && let Some(rect) = result.boxes.get_mut(&scope)
    {
        rect.origin = Point::default();
    }
    let sizes = children
        .iter()
        .map(|id| {
            result
                .boxes
                .get(id)
                .map_or(Size::default(), |rect| rect.size)
        })
        .collect::<Vec<_>>();
    let (groups, row_directed) = match axis {
        DynamicAxis::Rows => (
            partition_children(&children, &sizes, config.rows, horizontal_gap, false),
            true,
        ),
        DynamicAxis::Columns => (
            partition_children(&children, &sizes, config.columns, vertical_gap, true),
            false,
        ),
    };
    #[cfg(feature = "diagnostic-traces")]
    if super::diagnostic_env_enabled("DAGRE_DEBUG_NESTED") {
        let child_state = children
            .iter()
            .map(|id| {
                (
                    *id,
                    graph.node(*id).map(|node| node.external_id.as_str()),
                    result.boxes.get(id).map(|rect| rect.size),
                )
            })
            .collect::<Vec<_>>();
        eprintln!(
            "GRID_DYNAMIC scope={scope:?} axis={axis:?} groups={groups:?} children={child_state:?}"
        );
    }
    let mut deltas = BTreeMap::new();
    let (content_width, content_height) = if row_directed {
        let mut row_widths = groups
            .iter()
            .map(|group| {
                group
                    .iter()
                    .map(|id| result.boxes[id].size.width)
                    .sum::<f64>()
                    + horizontal_gap * group.len().saturating_sub(1) as f64
            })
            .collect::<Vec<_>>();
        let max_width = row_widths.iter().copied().fold(0.0, f64::max);
        let mut row_heights = Vec::with_capacity(groups.len());
        // d2grid/layout.go lays out from zero, then shifts the finished grid.
        // Adding the origin during accumulation changes floating-point rounding.
        let mut y = 0.0;
        for (row_index, group) in groups.iter().enumerate() {
            expand_group_widths(
                result,
                group,
                &mut row_widths[row_index],
                max_width,
                horizontal_gap,
            );
            let row_height = group
                .iter()
                .map(|id| result.boxes[id].size.height)
                .fold(0.0, f64::max);
            row_heights.push(row_height);
            let mut x = 0.0;
            for id in group {
                move_grid_child(
                    graph,
                    result,
                    *id,
                    Point {
                        x: x + (origin.x + horizontal_padding),
                        y: y + (origin.y + vertical_padding),
                    },
                    Size {
                        width: result.boxes[id].size.width,
                        height: row_height,
                    },
                    &mut deltas,
                );
                x += result.boxes[id].size.width + horizontal_gap;
            }
            y += row_height + vertical_gap;
        }
        (
            max_width,
            row_heights.iter().sum::<f64>()
                + vertical_gap * row_heights.len().saturating_sub(1) as f64,
        )
    } else {
        let mut column_heights = groups
            .iter()
            .map(|group| {
                group
                    .iter()
                    .map(|id| result.boxes[id].size.height)
                    .sum::<f64>()
                    + vertical_gap * group.len().saturating_sub(1) as f64
            })
            .collect::<Vec<_>>();
        let max_height = column_heights.iter().copied().fold(0.0, f64::max);
        let mut column_widths = Vec::with_capacity(groups.len());
        let mut x = 0.0;
        for (column_index, group) in groups.iter().enumerate() {
            expand_group_heights(
                result,
                group,
                &mut column_heights[column_index],
                max_height,
                vertical_gap,
            );
            let column_width = group
                .iter()
                .map(|id| result.boxes[id].size.width)
                .fold(0.0, f64::max);
            column_widths.push(column_width);
            let mut y = 0.0;
            for id in group {
                move_grid_child(
                    graph,
                    result,
                    *id,
                    Point {
                        x: x + (origin.x + horizontal_padding),
                        y: y + (origin.y + vertical_padding),
                    },
                    Size {
                        width: column_width,
                        height: result.boxes[id].size.height,
                    },
                    &mut deltas,
                );
                y += result.boxes[id].size.height + vertical_gap;
            }
            x += column_width + horizontal_gap;
        }
        (
            column_widths.iter().sum::<f64>()
                + horizontal_gap * column_widths.len().saturating_sub(1) as f64,
            max_height,
        )
    };
    if let Some(scope) = scope {
        let content_size = Size {
            width: content_width,
            height: content_height,
        };
        let padding = grid_scope_padding(
            graph,
            result,
            scope,
            horizontal_padding,
            vertical_padding,
            content_size,
        );
        let desired_width = content_width + padding.left + padding.right;
        let desired_height = content_height + padding.top + padding.bottom;
        let node = graph.node(scope);
        let existing_size = result
            .boxes
            .get(&scope)
            .map(|rect| rect.size)
            .unwrap_or(Size {
                width: desired_width,
                height: desired_height,
            });
        let fitted_size = node.map_or(
            Size {
                width: desired_width,
                height: desired_height,
            },
            |node| {
                grid_shape_size(
                    node,
                    Size {
                        width: desired_width,
                        height: desired_height,
                    },
                    existing_size,
                )
            },
        );
        if let Some(rect) = result.boxes.get_mut(&scope) {
            rect.size.width = fitted_size.width;
            rect.size.height = fitted_size.height;
        }
        let mut shift = Point {
            x: padding.left - horizontal_padding,
            y: padding.top - vertical_padding,
        };
        if let Some(node) = node {
            let shape_shift = grid_shape_shift(
                node.shape,
                fitted_size,
                Size {
                    width: desired_width,
                    height: desired_height,
                },
            );
            shift.x += shape_shift.x;
            shift.y += shape_shift.y;
        }
        if shift.x != 0.0 || shift.y != 0.0 {
            for child in children.iter().copied() {
                shift_subtree(graph, result, child, shift, &mut deltas);
            }
        }
    }
    if scope.is_none() {
        let (state, shift) = super::root_state::grid_root(
            graph,
            Size {
                width: content_width,
                height: content_height,
            },
            config
                .horizontal_gap
                .or(config.grid_gap)
                .unwrap_or(CONTAINER_PADDING)
                .max(0.0),
            config
                .vertical_gap
                .or(config.grid_gap)
                .unwrap_or(CONTAINER_PADDING)
                .max(0.0),
        );
        result.synthetic_root = Some(state);
        if shift.x != 0.0 || shift.y != 0.0 {
            for child in children.iter().copied() {
                shift_subtree(graph, result, child, shift, &mut deltas);
            }
        }
    }
    finish_grid_objects(graph, result, &children, &margins, &mut deltas);
    apply_grid_label_states(graph, result, scope, &children);
    update_grid_routes(graph, result, scope, &deltas);
}

/// Apply the shape's `GetDimensionsToFit`/aspect-ratio rules to the total
/// grid rectangle.  D2 performs this through `Object.SizeToContent` before it
/// asks the shape for its inner box.  Keeping the formulas here avoids making
/// the layout crate depend on the renderer's path implementation.
pub(super) fn grid_shape_size(node: &d2_rs_core::Node, content: Size, existing: Size) -> Size {
    let mut size = shape_dimensions_to_fit(node.shape, content);
    if node.fixed_width {
        // The first parent TALA pass can resize a shaped grid placeholder
        // while it still contains its authored children.  D2's nested-grid
        // extraction keeps the declared fixed dimension when the grid is
        // later fitted, so use that source value instead of the temporary
        // pre-extraction box.
        size.width = node
            .declared_size
            .map_or(existing.width, |declared| declared.width);
    }
    if node.fixed_height {
        size.height = node
            .declared_size
            .map_or(existing.height, |declared| declared.height);
    }
    if matches!(
        node.shape,
        d2_rs_core::ShapeKind::Circle | d2_rs_core::ShapeKind::RealSquare
    ) {
        let side = size.width.max(size.height);
        size.width = side;
        size.height = side;
    } else if !node.fixed_width && !node.fixed_height {
        match node.shape {
            d2_rs_core::ShapeKind::Oval => {
                size = limit_shape_aspect(size.width, size.height, OVAL_ASPECT_LIMIT)
            }
            d2_rs_core::ShapeKind::Person => {
                size = limit_shape_aspect(size.width, size.height, PERSON_ASPECT_LIMIT)
            }
            d2_rs_core::ShapeKind::C4Person => {
                size = limit_shape_aspect(size.width, size.height, C4_PERSON_ASPECT_LIMIT)
            }
            _ => {}
        }
    }
    size
}

fn shape_dimensions_to_fit(shape: d2_rs_core::ShapeKind, content: Size) -> Size {
    let width = content.width;
    let height = content.height;
    let ceil_size = |width: f64, height: f64| Size {
        width: width.ceil(),
        height: height.ceil(),
    };
    match shape {
        d2_rs_core::ShapeKind::RealSquare => {
            let side = width.max(height).ceil();
            Size {
                width: side,
                height: side,
            }
        }
        d2_rs_core::ShapeKind::Circle => {
            let side = (2.0_f64).sqrt() * width.max(height);
            let side = side.ceil();
            Size {
                width: side,
                height: side,
            }
        }
        d2_rs_core::ShapeKind::Oval => {
            let fitted = Size {
                // D2's oval implementation only uses the angle when an
                // explicit padding value is supplied. Grid calls
                // SizeToContent with zero shape padding, so the dimensions
                // are simply the diagonal multiplier of each content axis.
                width: ((2.0_f64).sqrt() * width).ceil(),
                height: ((2.0_f64).sqrt() * height).ceil(),
            };
            limit_shape_aspect(fitted.width, fitted.height, OVAL_ASPECT_LIMIT)
        }
        d2_rs_core::ShapeKind::Diamond => ceil_size(2.0 * width, 2.0 * height),
        d2_rs_core::ShapeKind::Hexagon => ceil_size(1.5 * width, 1.5 * height),
        d2_rs_core::ShapeKind::Cloud => {
            let aspect = width / height.max(f64::MIN_POSITIVE);
            if aspect > CLOUD_WIDE_ASPECT_BOUNDARY {
                ceil_size(
                    width / CLOUD_WIDE_INNER_WIDTH,
                    height / CLOUD_WIDE_INNER_HEIGHT,
                )
            } else if aspect < CLOUD_TALL_ASPECT_BOUNDARY {
                ceil_size(
                    width / CLOUD_TALL_INNER_WIDTH,
                    height / CLOUD_TALL_INNER_HEIGHT,
                )
            } else {
                ceil_size(
                    width / CLOUD_SQUARE_INNER_WIDTH,
                    height / CLOUD_SQUARE_INNER_HEIGHT,
                )
            }
        }
        d2_rs_core::ShapeKind::Cylinder => ceil_size(width, height + 3.0 * DEFAULT_ARC_DEPTH),
        d2_rs_core::ShapeKind::Queue => ceil_size(width + 3.0 * DEFAULT_ARC_DEPTH, height),
        d2_rs_core::ShapeKind::Parallelogram => {
            ceil_size(width + 2.0 * PARALLELOGRAM_WEDGE_WIDTH, height)
        }
        d2_rs_core::ShapeKind::Document => {
            ceil_size(width, height * DOCUMENT_PATH_HEIGHT / DOCUMENT_INNER_BOTTOM)
        }
        d2_rs_core::ShapeKind::Page => {
            let mut fitted_width = width;
            let fitted_height = height;
            if fitted_height < 3.0 * PAGE_CORNER_HEIGHT {
                fitted_width += PAGE_CORNER_WIDTH;
            }
            ceil_size(
                fitted_width.max(2.0 * PAGE_CORNER_WIDTH),
                fitted_height.max(PAGE_CORNER_HEIGHT),
            )
        }
        d2_rs_core::ShapeKind::Package => {
            let inner_height = height;
            let top_height =
                inner_height * PACKAGE_VERTICAL_SCALAR / (1.0 - PACKAGE_VERTICAL_SCALAR);
            ceil_size(width, inner_height + top_height.min(PACKAGE_TOP_MAX_HEIGHT))
        }
        d2_rs_core::ShapeKind::Step => ceil_size(width + 2.0 * STEP_WEDGE_WIDTH, height),
        d2_rs_core::ShapeKind::StoredData => {
            ceil_size(width + 2.0 * STORED_DATA_WEDGE_WIDTH, height)
        }
        d2_rs_core::ShapeKind::Callout => {
            let base_height = if height < CALLOUT_TIP_HEIGHT {
                2.0 * height
            } else {
                height + CALLOUT_TIP_HEIGHT
            };
            ceil_size(width, base_height)
        }
        d2_rs_core::ShapeKind::Person => Size { width, height },
        d2_rs_core::ShapeKind::C4Person => {
            let total_width = width / 0.9;
            let head_radius = total_width * C4_HEAD_RADIUS_FACTOR;
            let body_top = head_radius + head_radius * C4_BODY_TOP_FACTOR;
            let vertical_padding = total_width * 0.06;
            let mut total_height = height + body_top + vertical_padding;
            total_height = total_height.max(total_width * 0.95);
            let fitted = limit_shape_aspect(total_width, total_height, C4_PERSON_ASPECT_LIMIT);
            ceil_size(fitted.width, fitted.height)
        }
        _ => Size {
            width: width.ceil(),
            height: height.ceil(),
        },
    }
}

fn limit_shape_aspect(mut width: f64, mut height: f64, limit: f64) -> Size {
    if width > limit * height {
        height = (width / limit).round();
    } else if height > limit * width {
        width = (height / limit).round();
    }
    Size { width, height }
}

/// Return the extra translation applied after the rectangular grid has been
/// positioned.  This is the Rust equivalent of d2grid's
/// `-horizontalPadding + innerTL + padding + resize` after the cells already
/// sit at `padding` inside the object.
pub(super) fn grid_shape_shift(
    shape: d2_rs_core::ShapeKind,
    box_size: Size,
    content: Size,
) -> Point {
    let inside = shape_inside_placement(shape, box_size, content);
    let inner = shape_inner_box(shape, box_size, content);
    Point {
        x: inside.x + ((inner.width - content.width).max(0.0) / 2.0),
        y: inside.y + ((inner.height - content.height).max(0.0) / 2.0),
    }
}

fn shape_inside_placement(shape: d2_rs_core::ShapeKind, box_size: Size, content: Size) -> Point {
    match shape {
        d2_rs_core::ShapeKind::Circle => {
            let radius = box_size.width / 2.0;
            let offset = (radius - radius * (2.0_f64).sqrt() / 2.0).ceil();
            Point {
                x: offset,
                y: offset,
            }
        }
        d2_rs_core::ShapeKind::Oval => {
            let rx = box_size.width / 2.0;
            let ry = box_size.height / 2.0;
            let theta = (ry / rx.max(f64::MIN_POSITIVE)).atan();
            let sin = theta.sin();
            let cos = theta.cos();
            let radius = rx * ry / ((rx * sin).powi(2) + (ry * cos).powi(2)).sqrt();
            Point {
                x: (rx - cos * radius).ceil(),
                y: (ry - sin * radius).ceil(),
            }
        }
        d2_rs_core::ShapeKind::Diamond => Point {
            x: box_size.width / 4.0,
            y: box_size.height / 4.0,
        },
        d2_rs_core::ShapeKind::Hexagon => Point {
            x: box_size.width / 6.0,
            y: box_size.height / 6.0,
        },
        d2_rs_core::ShapeKind::Cloud => {
            let (x, y) = cloud_inner_factors(content);
            Point {
                x: (box_size.width * x).ceil(),
                y: (box_size.height * y).ceil(),
            }
        }
        d2_rs_core::ShapeKind::Cylinder => Point {
            x: 0.0,
            y: 2.0 * cylinder_arc_height(box_size.height),
        },
        d2_rs_core::ShapeKind::Queue => Point {
            x: queue_arc_width(box_size.width),
            y: 0.0,
        },
        d2_rs_core::ShapeKind::Parallelogram => Point {
            x: PARALLELOGRAM_WEDGE_WIDTH,
            y: 0.0,
        },
        d2_rs_core::ShapeKind::Step => Point {
            x: STEP_WEDGE_WIDTH,
            y: 0.0,
        },
        d2_rs_core::ShapeKind::StoredData => Point {
            x: STORED_DATA_WEDGE_WIDTH,
            y: 0.0,
        },
        d2_rs_core::ShapeKind::Package => Point {
            x: 0.0,
            y: package_top_height(box_size),
        },
        d2_rs_core::ShapeKind::Person => Point {
            x: box_size.width * (20.2 / 68.3),
            y: 0.0,
        },
        d2_rs_core::ShapeKind::C4Person => {
            let head_radius = box_size.width * C4_HEAD_RADIUS_FACTOR;
            Point {
                x: box_size.width * 0.05,
                y: head_radius + head_radius * C4_BODY_TOP_FACTOR + box_size.height * 0.03,
            }
        }
        _ => Point::default(),
    }
}

fn shape_inner_box(shape: d2_rs_core::ShapeKind, box_size: Size, content: Size) -> Size {
    match shape {
        d2_rs_core::ShapeKind::Circle => {
            let radius = box_size.width / 2.0;
            let offset = (radius - radius * (2.0_f64).sqrt() / 2.0).ceil();
            Size {
                width: box_size.width - 2.0 * offset,
                height: box_size.height - 2.0 * offset,
            }
        }
        d2_rs_core::ShapeKind::Oval => {
            let inside = shape_inside_placement(shape, box_size, content);
            Size {
                width: box_size.width - 2.0 * inside.x,
                height: box_size.height - 2.0 * inside.y,
            }
        }
        d2_rs_core::ShapeKind::Diamond => Size {
            width: box_size.width / 2.0,
            height: box_size.height / 2.0,
        },
        d2_rs_core::ShapeKind::Hexagon => Size {
            width: box_size.width / 1.5,
            height: box_size.height / 1.5,
        },
        d2_rs_core::ShapeKind::Cloud => {
            let (x, y, width, height) = cloud_inner_factors_and_size(content);
            let _ = (x, y);
            Size {
                width: box_size.width * width,
                height: box_size.height * height,
            }
        }
        d2_rs_core::ShapeKind::Cylinder => Size {
            width: box_size.width,
            height: box_size.height - 3.0 * cylinder_arc_height(box_size.height),
        },
        d2_rs_core::ShapeKind::Queue => Size {
            width: box_size.width - 3.0 * queue_arc_width(box_size.width),
            height: box_size.height,
        },
        d2_rs_core::ShapeKind::Parallelogram => Size {
            width: box_size.width - 2.0 * PARALLELOGRAM_WEDGE_WIDTH,
            height: box_size.height,
        },
        d2_rs_core::ShapeKind::Step => Size {
            width: box_size.width - 2.0 * STEP_WEDGE_WIDTH,
            height: box_size.height,
        },
        d2_rs_core::ShapeKind::StoredData => Size {
            width: box_size.width - 2.0 * STORED_DATA_WEDGE_WIDTH,
            height: box_size.height,
        },
        d2_rs_core::ShapeKind::Callout => Size {
            width: box_size.width,
            height: box_size.height - callout_tip_height(box_size.height),
        },
        d2_rs_core::ShapeKind::Person => Size {
            width: box_size.width * (1.0 - 2.0 * (20.2 / 68.3)),
            height: box_size.height,
        },
        d2_rs_core::ShapeKind::C4Person => {
            let head_radius = box_size.width * C4_HEAD_RADIUS_FACTOR;
            let body_top = head_radius + head_radius * C4_BODY_TOP_FACTOR;
            Size {
                width: box_size.width * 0.9,
                height: box_size.height - body_top - box_size.height * 0.06,
            }
        }
        d2_rs_core::ShapeKind::Package => Size {
            width: box_size.width,
            height: box_size.height - package_top_height(box_size),
        },
        _ => box_size,
    }
}

fn cloud_inner_factors(content: Size) -> (f64, f64) {
    let aspect = content.width / content.height.max(f64::MIN_POSITIVE);
    if aspect > CLOUD_WIDE_ASPECT_BOUNDARY {
        (CLOUD_WIDE_INNER_X, CLOUD_WIDE_INNER_Y)
    } else if aspect < CLOUD_TALL_ASPECT_BOUNDARY {
        (CLOUD_TALL_INNER_X, CLOUD_TALL_INNER_Y)
    } else {
        (CLOUD_SQUARE_INNER_X, CLOUD_SQUARE_INNER_Y)
    }
}

fn cloud_inner_factors_and_size(content: Size) -> (f64, f64, f64, f64) {
    let aspect = content.width / content.height.max(f64::MIN_POSITIVE);
    if aspect > CLOUD_WIDE_ASPECT_BOUNDARY {
        (
            CLOUD_WIDE_INNER_X,
            CLOUD_WIDE_INNER_Y,
            CLOUD_WIDE_INNER_WIDTH,
            CLOUD_WIDE_INNER_HEIGHT,
        )
    } else if aspect < CLOUD_TALL_ASPECT_BOUNDARY {
        (
            CLOUD_TALL_INNER_X,
            CLOUD_TALL_INNER_Y,
            CLOUD_TALL_INNER_WIDTH,
            CLOUD_TALL_INNER_HEIGHT,
        )
    } else {
        (
            CLOUD_SQUARE_INNER_X,
            CLOUD_SQUARE_INNER_Y,
            CLOUD_SQUARE_INNER_WIDTH,
            CLOUD_SQUARE_INNER_HEIGHT,
        )
    }
}

fn cylinder_arc_height(height: f64) -> f64 {
    if height < 2.0 * DEFAULT_ARC_DEPTH {
        height / 2.0
    } else {
        DEFAULT_ARC_DEPTH
    }
}

fn queue_arc_width(width: f64) -> f64 {
    if width < 2.0 * DEFAULT_ARC_DEPTH {
        width / 2.0
    } else {
        DEFAULT_ARC_DEPTH
    }
}

fn package_top_height(size: Size) -> f64 {
    (size.height * PACKAGE_VERTICAL_SCALAR).min(PACKAGE_TOP_MAX_HEIGHT)
}

fn callout_tip_height(height: f64) -> f64 {
    if height < 2.0 * CALLOUT_TIP_HEIGHT {
        height / 2.0
    } else {
        CALLOUT_TIP_HEIGHT
    }
}

/// D2's grid layout adds temporary clearance for labels and icons before
/// choosing rows/columns. The layout then removes the margin and shifts the
/// object (and its descendants) by the remaining left/top clearance. Keeping
/// this as a pair of explicit phases is important: cell widths are computed
/// from the inflated boxes, while final target geometry is still the original
/// object size.
fn prepare_grid_objects(
    graph: &Graph,
    result: &mut LayoutResult,
    scope: Option<NodeId>,
    children: &[NodeId],
) -> BTreeMap<NodeId, Insets> {
    let mut margins = BTreeMap::new();
    for child in children.iter().copied() {
        let Some(node) = graph.node(child) else {
            continue;
        };
        let (label_position, icon_position) =
            effective_grid_positions(graph, Some(result), child, true);
        let Some(rect) = result.boxes.get_mut(&child) else {
            continue;
        };
        let margin = grid_object_margin(
            node,
            has_label(graph, child),
            label_position,
            icon_position,
            rect.size,
        );
        rect.size.width += margin.left + margin.right;
        rect.size.height += margin.top + margin.bottom;
        margins.insert(child, margin);
    }
    // A grid scope itself gets the same default label/icon positions as the
    // upstream root object. Its own label is handled by the container sizing
    // stage, but recording the position here keeps target serialization and
    // SVG placement consistent with D2.
    if let Some(scope) = scope {
        apply_grid_label_states(graph, result, Some(scope), &[]);
    }
    margins
}

fn finish_grid_objects(
    graph: &Graph,
    result: &mut LayoutResult,
    children: &[NodeId],
    original_margins: &BTreeMap<NodeId, Insets>,
    deltas: &mut BTreeMap<NodeId, Point>,
) {
    for child in children.iter().copied() {
        let Some(node) = graph.node(child) else {
            continue;
        };
        let (label_position, icon_position) =
            effective_grid_positions(graph, Some(result), child, true);
        let Some(rect) = result.boxes.get_mut(&child) else {
            continue;
        };
        // d2grid removes the exact margin added before layout, then measures
        // again. If the cell grew enough that the new margin is smaller, the
        // difference is restored; the final translation uses the new left/top
        // margin. This two-step cleanup is observable for nested grids whose
        // outside label is wider than the original object.
        let old_margin = original_margins
            .get(&child)
            .copied()
            .unwrap_or_else(|| Insets::uniform(0.0));
        let old_width = rect.size.width;
        let old_height = rect.size.height;
        rect.size.width = (old_width - old_margin.left - old_margin.right).max(0.0);
        rect.size.height = (old_height - old_margin.top - old_margin.bottom).max(0.0);
        let margin = grid_object_margin(
            node,
            has_label(graph, child),
            label_position,
            icon_position,
            rect.size,
        );
        let old_margin_x = old_margin.left + old_margin.right;
        let old_margin_y = old_margin.top + old_margin.bottom;
        let margin_x = margin.left + margin.right;
        let margin_y = margin.top + margin.bottom;
        if margin_x < old_margin_x {
            rect.size.width += old_margin_x - margin_x;
        }
        if margin_y < old_margin_y {
            rect.size.height += old_margin_y - margin_y;
        }
        if margin.left != 0.0 || margin.top != 0.0 {
            shift_subtree(
                graph,
                result,
                child,
                Point {
                    x: margin.left,
                    y: margin.top,
                },
                deltas,
            );
        }
    }
}

fn apply_grid_label_states(
    graph: &Graph,
    result: &mut LayoutResult,
    scope: Option<NodeId>,
    children: &[NodeId],
) {
    let mut ids = children.to_vec();
    if let Some(scope) = scope {
        ids.push(scope);
    }
    for id in ids {
        let Some(node) = graph.node(id) else {
            continue;
        };
        let (mut label_position, mut icon_position) =
            effective_grid_positions(graph, Some(result), id, Some(id) != scope);
        // The graph root receives a separate default in d2grid.Layout: its
        // label is inside-top-center and its icon is inside-top-left.
        if Some(id) == scope {
            if has_label(graph, id) && node.label_position == LabelPosition::Unset {
                label_position = LabelPosition::InsideTopCenter;
            }
            if node.has_icon && node.icon_position.is_none() {
                icon_position = Some(LabelPosition::InsideTopLeft);
            }
        }
        if !has_label(graph, id) && !node.has_icon {
            continue;
        }
        let state = result.node_labels.entry(id).or_insert(NodeLabelState {
            font_size_committed: false,
            size: node.label_size,
            font_size: node.font_size,
            position: node.label_position,
            icon_position: node.icon_position,
        });
        state.position = label_position;
        state.icon_position = icon_position;
    }
}

fn effective_grid_positions(
    graph: &Graph,
    result: Option<&LayoutResult>,
    id: NodeId,
    as_cell: bool,
) -> (LabelPosition, Option<LabelPosition>) {
    let Some(node) = graph.node(id) else {
        return (LabelPosition::Unset, None);
    };
    let is_grid_cell = node.parent.map_or_else(
        || graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some(),
        |parent| {
            graph
                .node(parent)
                .is_some_and(|parent| parent.grid_rows.is_some() || parent.grid_columns.is_some())
        },
    );
    let is_grid_scope = node.grid_rows.is_some() || node.grid_columns.is_some();
    let detached_cell = as_cell && (is_grid_scope || graph.is_sequence_diagram(id));
    let layout_label_position = result
        .filter(|_| !detached_cell)
        .and_then(|result| result.node_labels.get(&id))
        .map(|state| state.position)
        .filter(|position| *position != LabelPosition::Unset);
    let layout_icon_position = result
        .filter(|_| !detached_cell)
        .and_then(|result| result.node_labels.get(&id))
        .and_then(|state| state.icon_position);
    let has_children = !detached_cell && graph.nodes().any(|(_, child)| child.parent == Some(id));
    // Grid cells normally preserve a position chosen by nested layout. Its
    // generic OutsideBottomCenter default is temporary for ordinary leaf
    // cells, though: d2grid replaces that with its source-level fallback.
    // Images and person shapes intentionally keep their outside-bottom label.
    let temporary_leaf_label = is_grid_cell
        && !has_children
        && layout_label_position == Some(LabelPosition::OutsideBottomCenter)
        && !matches!(
            node.shape,
            d2_rs_core::ShapeKind::Image | d2_rs_core::ShapeKind::Person
        );
    let mut label_position = if node.label_position != LabelPosition::Unset {
        node.label_position
    } else if temporary_leaf_label {
        LabelPosition::Unset
    } else {
        layout_label_position.unwrap_or(node.label_position)
    };
    let mut icon_position = node.icon_position.or(layout_icon_position);
    if is_grid_scope && !as_cell {
        // d2grid.Layout is invoked with this object as the nested graph root;
        // its own label/icon defaults are therefore inside-top-center and
        // inside-top-left. The parent grid packs a detached leaf cell; root
        // positions are injected only after that packing stage.
        if node.label_position == LabelPosition::Unset {
            // A detached grid root has not visited the parent engine yet.
            // Discard its provisional container default even for an empty
            // label, for which d2grid.Layout leaves LabelPosition nil.
            label_position = if has_label(graph, id) {
                LabelPosition::InsideTopCenter
            } else {
                LabelPosition::Unset
            };
        }
        if node.has_icon && node.icon_position.is_none() {
            icon_position = Some(LabelPosition::InsideTopLeft);
        }
        return (label_position, icon_position);
    }
    let has_icon = node.has_icon && node.shape != d2_rs_core::ShapeKind::Image;
    let mut positioned_label = false;
    // d2grid.layoutGrid checks Icon != nil, including image shapes whose
    // HasIcon is false. Their position is still published in the target.
    if node.has_icon && icon_position.is_none() {
        if has_children {
            icon_position = Some(LabelPosition::OutsideTopLeft);
            if has_label(graph, id) && label_position == LabelPosition::Unset {
                label_position = LabelPosition::OutsideTopRight;
                positioned_label = true;
            }
        } else {
            icon_position = Some(LabelPosition::InsideMiddleCenter);
        }
    }
    if !positioned_label && has_label(graph, id) && label_position == LabelPosition::Unset {
        label_position = if has_children {
            LabelPosition::OutsideTopCenter
        } else if matches!(
            node.shape,
            d2_rs_core::ShapeKind::Image | d2_rs_core::ShapeKind::Person
        ) {
            LabelPosition::OutsideBottomCenter
        } else if has_icon {
            LabelPosition::InsideTopCenter
        } else {
            LabelPosition::InsideMiddleCenter
        };
    }
    (label_position, icon_position)
}

fn has_label(graph: &Graph, id: NodeId) -> bool {
    let Some(node) = graph.node(id) else {
        return false;
    };
    if matches!(
        node.shape,
        d2_rs_core::ShapeKind::Text
            | d2_rs_core::ShapeKind::Class
            | d2_rs_core::ShapeKind::SqlTable
            | d2_rs_core::ShapeKind::Code
    ) {
        return false;
    }
    graph
        .node_labels
        .get(&id)
        .is_some_and(|label| !label.is_empty())
        || node
            .label_size
            .is_some_and(|size| size.width > 0.0 || size.height > 0.0)
}

/// d2graph `Object.GetMargin`. The label branch is gated on `HasLabel` (a
/// non-empty label value), not on measured dimensions: a whitespace label such
/// as `" "` measures 0x0 yet still reserves `label.PADDING` of clearance.
fn grid_object_margin(
    node: &d2_rs_core::Node,
    has_label: bool,
    label_position: LabelPosition,
    icon_position: Option<LabelPosition>,
    object_size: Size,
) -> Insets {
    let mut margin = Insets::uniform(0.0);
    if has_label {
        let label = node.label_size.unwrap_or_default();
        let label_width = label.width + LABEL_PADDING;
        let label_height = label.height + LABEL_PADDING;
        match label_position {
            LabelPosition::OutsideTopLeft
            | LabelPosition::OutsideTopCenter
            | LabelPosition::OutsideTopRight => margin.top = label_height,
            LabelPosition::OutsideBottomLeft
            | LabelPosition::OutsideBottomCenter
            | LabelPosition::OutsideBottomRight => margin.bottom = label_height,
            LabelPosition::OutsideLeftTop
            | LabelPosition::OutsideLeftMiddle
            | LabelPosition::OutsideLeftBottom => margin.left = label_width,
            LabelPosition::OutsideRightTop
            | LabelPosition::OutsideRightMiddle
            | LabelPosition::OutsideRightBottom => margin.right = label_width,
            _ => {}
        }
        if label_width > object_size.width {
            let extra = label_width - object_size.width;
            match label_position {
                LabelPosition::OutsideTopLeft | LabelPosition::OutsideBottomLeft => {
                    margin.right = extra
                }
                LabelPosition::OutsideTopCenter | LabelPosition::OutsideBottomCenter => {
                    let half = (extra / 2.0).ceil();
                    margin.left = margin.left.max(half);
                    margin.right = margin.right.max(half);
                }
                LabelPosition::OutsideTopRight | LabelPosition::OutsideBottomRight => {
                    margin.left = extra
                }
                _ => {}
            }
        }
        if label_height > object_size.height {
            let extra = label_height - object_size.height;
            match label_position {
                LabelPosition::OutsideLeftTop | LabelPosition::OutsideRightTop => {
                    margin.bottom = extra
                }
                LabelPosition::OutsideLeftMiddle | LabelPosition::OutsideRightMiddle => {
                    let half = (extra / 2.0).ceil();
                    margin.top = margin.top.max(half);
                    margin.bottom = margin.bottom.max(half);
                }
                LabelPosition::OutsideLeftBottom | LabelPosition::OutsideRightBottom => {
                    margin.top = extra
                }
                _ => {}
            }
        }
    }
    if node.has_icon
        && node.shape != d2_rs_core::ShapeKind::Image
        && let Some(icon_position) = icon_position
    {
        let icon_size = MAX_ICON_SIZE + LABEL_PADDING;
        match icon_position {
            LabelPosition::OutsideTopLeft
            | LabelPosition::OutsideTopCenter
            | LabelPosition::OutsideTopRight => margin.top = margin.top.max(icon_size),
            LabelPosition::OutsideBottomLeft
            | LabelPosition::OutsideBottomCenter
            | LabelPosition::OutsideBottomRight => margin.bottom = margin.bottom.max(icon_size),
            LabelPosition::OutsideLeftTop
            | LabelPosition::OutsideLeftMiddle
            | LabelPosition::OutsideLeftBottom => margin.left = margin.left.max(icon_size),
            LabelPosition::OutsideRightTop
            | LabelPosition::OutsideRightMiddle
            | LabelPosition::OutsideRightBottom => margin.right = margin.right.max(icon_size),
            _ => {}
        }
    }
    // d2graph `GetModifierElementAdjustments`: a 3D hexagon's dy is the Go
    // integer constant expression THREE_DEE_OFFSET / 2, which is 7.
    if node.is_3d {
        margin.right += 15.0;
        margin.top += if node.shape == d2_rs_core::ShapeKind::Hexagon {
            7.0
        } else {
            15.0
        };
    } else if node.is_multiple {
        margin.right += 10.0;
        margin.top += 10.0;
    }
    margin
}

/// Returns the complete D2 `Object.Spacing` padding for a grid root. Grid
/// gaps provide the base padding, then inside labels/icons reserve their own
/// top, bottom, or side band.
fn grid_scope_padding(
    graph: &Graph,
    result: &LayoutResult,
    scope: NodeId,
    horizontal: f64,
    vertical: f64,
    content: Size,
) -> Insets {
    // Start with the shape/label spacing.  d2grid applies the configured
    // horizontal/vertical gap as a minimum only after label overflow has
    // been computed; adding overflow on top of the gap double-counts it for
    // wide top labels.
    let mut padding = Insets::uniform(0.0);
    let Some(node) = graph.node(scope) else {
        return padding;
    };
    let (label_position, icon_position) =
        effective_grid_positions(graph, Some(result), scope, false);
    let label_width = node
        .label_size
        .map(|size| size.width + 2.0 * LABEL_PADDING)
        .unwrap_or(0.0);
    let label_height = node
        .label_size
        .map(|size| size.height + 2.0 * LABEL_PADDING)
        .unwrap_or(0.0);
    // `Object.Spacing` reserves the label's own band first.  d2grid then
    // adds symmetric overflow when a top/bottom label is wider than the
    // rectangular cell area (or when a side label is taller).  Omitting this
    // step makes shaped grids such as a labelled circle far too small.
    if label_width > content.width
        && matches!(
            label_position,
            LabelPosition::OutsideTopLeft
                | LabelPosition::OutsideTopCenter
                | LabelPosition::OutsideTopRight
                | LabelPosition::InsideTopLeft
                | LabelPosition::InsideTopCenter
                | LabelPosition::InsideTopRight
                | LabelPosition::InsideBottomLeft
                | LabelPosition::InsideBottomCenter
                | LabelPosition::InsideBottomRight
                | LabelPosition::OutsideBottomLeft
                | LabelPosition::OutsideBottomCenter
                | LabelPosition::OutsideBottomRight
        )
    {
        let overflow = (label_width - content.width) / 2.0;
        padding.left += overflow;
        padding.right += overflow;
    }
    if label_height > content.height
        && matches!(
            label_position,
            LabelPosition::OutsideLeftTop
                | LabelPosition::OutsideLeftMiddle
                | LabelPosition::OutsideLeftBottom
                | LabelPosition::InsideMiddleLeft
                | LabelPosition::InsideMiddleCenter
                | LabelPosition::InsideMiddleRight
                | LabelPosition::OutsideRightTop
                | LabelPosition::OutsideRightMiddle
                | LabelPosition::OutsideRightBottom
        )
    {
        let overflow = (label_height - content.height) / 2.0;
        padding.top += overflow;
        padding.bottom += overflow;
    }
    match label_position {
        LabelPosition::InsideTopLeft
        | LabelPosition::InsideTopCenter
        | LabelPosition::InsideTopRight => padding.top = padding.top.max(label_height),
        LabelPosition::InsideBottomLeft
        | LabelPosition::InsideBottomCenter
        | LabelPosition::InsideBottomRight => padding.bottom = padding.bottom.max(label_height),
        LabelPosition::InsideMiddleLeft => padding.left = padding.left.max(label_width),
        LabelPosition::InsideMiddleRight => padding.right = padding.right.max(label_width),
        _ => {}
    }
    if node.has_icon && node.shape != d2_rs_core::ShapeKind::Image {
        let icon_size = MAX_ICON_SIZE + 2.0 * LABEL_PADDING;
        match icon_position {
            Some(
                LabelPosition::InsideTopLeft
                | LabelPosition::InsideTopCenter
                | LabelPosition::InsideTopRight,
            ) => padding.top = padding.top.max(icon_size),
            Some(
                LabelPosition::InsideBottomLeft
                | LabelPosition::InsideBottomCenter
                | LabelPosition::InsideBottomRight,
            ) => padding.bottom = padding.bottom.max(icon_size),
            Some(LabelPosition::InsideMiddleLeft) => padding.left = padding.left.max(icon_size),
            Some(LabelPosition::InsideMiddleRight) => padding.right = padding.right.max(icon_size),
            _ => {}
        }
    }
    // d2grid.Layout's symmetric top-icon/center-label band depends on the
    // committed positions, independently of Object.HasIcon (which excludes
    // images). Grid roots still receive IconPosition for an image's Icon.
    if icon_position == Some(LabelPosition::InsideTopLeft)
        && label_position == LabelPosition::InsideTopCenter
    {
        let icon_size = MAX_ICON_SIZE + 2.0 * LABEL_PADDING;
        padding.left = padding.left.max(icon_size);
        padding.right = padding.right.max(icon_size);
        let minimum_width = 2.0 * icon_size + label_width;
        if minimum_width > content.width {
            let overflow = (minimum_width - content.width) / 2.0;
            padding.left = padding.left.max(overflow);
            padding.right = padding.right.max(overflow);
        }
    }
    padding.top = padding.top.max(vertical);
    padding.bottom = padding.bottom.max(vertical);
    padding.left = padding.left.max(horizontal);
    padding.right = padding.right.max(horizontal);
    padding
}

fn move_grid_child(
    graph: &Graph,
    result: &mut LayoutResult,
    child: NodeId,
    proposed: Point,
    size: Size,
    deltas: &mut BTreeMap<NodeId, Point>,
) {
    let Some(previous) = result.boxes.get(&child).copied() else {
        return;
    };
    let Some(node) = graph.node(child) else {
        return;
    };
    let mut target = proposed;
    if let Some(locked) = node.locked_position {
        target = locked;
    }
    if let Some(x) = node.constrained_x {
        target.x = x;
    }
    if let Some(y) = node.constrained_y {
        target.y = y;
    }
    if let Some(rect) = result.boxes.get_mut(&child) {
        rect.size = size;
    }
    let delta = Point {
        x: target.x - previous.origin.x,
        y: target.y - previous.origin.y,
    };
    if delta.x != 0.0 || delta.y != 0.0 {
        shift_subtree(graph, result, child, delta, deltas);
    }
}

pub(crate) fn set_grid_edge_label(
    graph: &Graph,
    result: &mut LayoutResult,
    id: d2_rs_core::EdgeId,
) {
    if let Some(mut label) = result
        .edge_labels
        .get(&id)
        .or_else(|| graph.edge_label(id))
        .cloned()
    {
        label.position = LabelPosition::InsideMiddleCenter;
        result.edge_labels.insert(id, label);
    }
}

fn set_direct_grid_edge_labels(graph: &Graph, result: &mut LayoutResult, scope: Option<NodeId>) {
    for (id, edge) in graph.edges() {
        if graph
            .node(edge.source)
            .is_some_and(|node| node.parent == scope)
            && graph
                .node(edge.target)
                .is_some_and(|node| node.parent == scope)
        {
            set_grid_edge_label(graph, result, id);
        }
    }
}

fn update_grid_routes(
    graph: &Graph,
    result: &mut LayoutResult,
    scope: Option<NodeId>,
    _deltas: &BTreeMap<NodeId, Point>,
) {
    for (edge_id, edge) in graph.edges() {
        let direct_child_edge = scope.is_some_and(|scope| {
            graph.node(edge.source).and_then(|node| node.parent) == Some(scope)
                && graph.node(edge.target).and_then(|node| node.parent) == Some(scope)
        });
        let root_child_edge = scope.is_none()
            && graph
                .node(edge.source)
                .is_some_and(|node| node.parent.is_none())
            && graph
                .node(edge.target)
                .is_some_and(|node| node.parent.is_none());
        if (root_child_edge || direct_child_edge)
            && let (Some(source), Some(target)) = (
                result.boxes.get(&edge.source),
                result.boxes.get(&edge.target),
            )
        {
            result.routes.insert(
                edge_id,
                grid_edge_route(graph, edge_id, *source, *target, &result.node_labels),
            );
            set_grid_edge_label(graph, result, edge_id);
            continue;
        }

        // One-axis grids take the same boundary as rectangular grids: once a
        // cell has been placed, an edge crossing two distinct cells is owned
        // by the grid and must not retain Dagre's old rank corridor. This is
        // especially visible when the grid has an external source or a
        // descendant endpoint.
        let Some(scope) = scope else {
            continue;
        };
        let source_cell = direct_grid_child(graph, edge.source, scope);
        let target_cell = direct_grid_child(graph, edge.target, scope);
        if source_cell.is_none() || source_cell == target_cell {
            continue;
        }
        if let (Some(source), Some(target)) = (
            result.boxes.get(&edge.source),
            result.boxes.get(&edge.target),
        ) {
            result.routes.insert(
                edge_id,
                grid_edge_route(graph, edge_id, *source, *target, &result.node_labels),
            );
        }
    }
}

fn expand_group_widths(
    result: &mut LayoutResult,
    group: &[NodeId],
    width: &mut f64,
    target: f64,
    gap: f64,
) {
    let delta = target - *width;
    if delta <= 0.0 || group.is_empty() {
        return;
    }
    let widest = group
        .iter()
        .map(|id| result.boxes[id].size.width)
        .fold(0.0, f64::max);
    let diffs = group
        .iter()
        .map(|id| widest - result.boxes[id].size.width)
        .collect::<Vec<_>>();
    let total_diff = diffs.iter().sum::<f64>();
    let growth = delta.min(total_diff);
    for (id, diff) in group.iter().zip(diffs.iter().copied()) {
        if total_diff > 0.0 {
            let size = &mut result.boxes.get_mut(id).unwrap().size.width;
            *size = d2_rs_core::go_fmadd(diff / total_diff, growth, *size);
        }
    }
    if delta > total_diff {
        let extra = (delta - total_diff) / group.len() as f64;
        for id in group {
            result.boxes.get_mut(id).unwrap().size.width += extra;
        }
    }
    *width = group
        .iter()
        .map(|id| result.boxes[id].size.width)
        .sum::<f64>()
        + gap * group.len().saturating_sub(1) as f64;
}

fn expand_group_heights(
    result: &mut LayoutResult,
    group: &[NodeId],
    height: &mut f64,
    target: f64,
    gap: f64,
) {
    let delta = target - *height;
    if delta <= 0.0 || group.is_empty() {
        return;
    }
    let tallest = group
        .iter()
        .map(|id| result.boxes[id].size.height)
        .fold(0.0, f64::max);
    let diffs = group
        .iter()
        .map(|id| tallest - result.boxes[id].size.height)
        .collect::<Vec<_>>();
    let total_diff = diffs.iter().sum::<f64>();
    let growth = delta.min(total_diff);
    for (id, diff) in group.iter().zip(diffs.iter().copied()) {
        if total_diff > 0.0 {
            let size = &mut result.boxes.get_mut(id).unwrap().size.height;
            *size = d2_rs_core::go_fmadd(diff / total_diff, growth, *size);
        }
    }
    if delta > total_diff {
        let extra = (delta - total_diff) / group.len() as f64;
        for id in group {
            result.boxes.get_mut(id).unwrap().size.height += extra;
        }
    }
    *height = group
        .iter()
        .map(|id| result.boxes[id].size.height)
        .sum::<f64>()
        + gap * group.len().saturating_sub(1) as f64;
}

fn partition_children(
    children: &[NodeId],
    sizes: &[Size],
    groups: usize,
    gap: f64,
    by_height: bool,
) -> Vec<Vec<NodeId>> {
    let count = children.len();
    if count == 0 {
        return Vec::new();
    }
    let groups = groups.max(1).min(count);
    let values = sizes
        .iter()
        .map(|size| if by_height { size.height } else { size.width })
        .collect::<Vec<_>>();
    let total = values.iter().sum::<f64>() + gap * count.saturating_sub(groups) as f64;
    let target = total / groups as f64;
    let cuts = best_grid_cuts(&values, groups, gap, target);
    let mut output = Vec::with_capacity(groups);
    let mut start = 0;
    for end in cuts.into_iter().chain(std::iter::once(count - 1)) {
        let stop = end + 1;
        output.push(children[start..stop].to_vec());
        start = stop;
    }
    output
}

fn best_grid_cuts(values: &[f64], groups: usize, gap: f64, target: f64) -> Vec<usize> {
    if groups <= 1 {
        return Vec::new();
    }
    let cuts_needed = groups - 1;
    let mut best = None;
    let mut best_dist = f64::MAX;
    let mut fast_is_best = false;

    // d2grid first tries its debt-carrying greedy layout.  This is not merely
    // an optimization: an exact zero-distance result returns immediately.
    let mut debt = 0.0;
    let mut row_size = 0.0;
    let mut fast = Vec::with_capacity(cuts_needed);
    for (index, size) in values.iter().copied().enumerate() {
        if row_size == 0.0 {
            if size > target - debt {
                fast.push(index);
                debt += size - target;
            } else {
                row_size = size;
            }
            continue;
        }
        if row_size + gap + size / 2.0 > target - debt {
            fast.push(index - 1);
            debt += row_size - target;
            row_size = size;
        } else {
            row_size += gap + size;
        }
    }
    if fast.len() == cuts_needed {
        let distance = grid_cuts_distance(values, &fast, gap, target);
        if distance == 0.0 {
            return fast;
        }
        best_dist = distance;
        best = Some(fast);
        fast_is_best = true;
    }

    let standard_deviation = grid_stddev(values);
    let mut threshold_attempts = standard_deviation.ceil() as usize;
    threshold_attempts =
        threshold_attempts.clamp(GRID_MIN_THRESHOLD_ATTEMPTS, GRID_MAX_THRESHOLD_ATTEMPTS);

    let mut threshold = GRID_STARTING_THRESHOLD;
    let mut starting_cache = BTreeMap::<usize, bool>::new();
    let mut measurements = GridRowMeasurements::new(values, gap);

    // This mirrors d2grid.iterDivisions.  The final group is visited first,
    // so later cuts win ties exactly as in the Go implementation.
    // Keep the argument shape explicit: these are the mutable search state
    // carried through d2grid.iterDivisions and changing their order changes
    // the first accepted cut.
    #[allow(clippy::too_many_arguments)]
    fn visit_divisions(
        measurements: &mut GridRowMeasurements<'_>,
        cuts: &mut [usize],
        end: usize,
        remaining: usize,
        gap: f64,
        target: f64,
        threshold: f64,
        skip_count: &mut usize,
        starting_cache: &mut BTreeMap<usize, bool>,
        attempt_count: &mut usize,
        best: &mut Option<Vec<usize>>,
        best_dist: &mut f64,
        fast_is_best: &mut bool,
    ) -> bool {
        for index in (remaining..end).rev() {
            if !grid_row_ok(
                measurements,
                gap,
                target,
                threshold,
                index,
                end,
                false,
                skip_count,
                starting_cache,
            ) {
                continue;
            }
            cuts[remaining - 1] = index - 1;
            if remaining > 1 {
                if visit_divisions(
                    measurements,
                    cuts,
                    index,
                    remaining - 1,
                    gap,
                    target,
                    threshold,
                    skip_count,
                    starting_cache,
                    attempt_count,
                    best,
                    best_dist,
                    fast_is_best,
                ) {
                    return true;
                }
            } else if grid_row_ok(
                measurements,
                gap,
                target,
                threshold,
                0,
                index,
                true,
                skip_count,
                starting_cache,
            ) {
                let distance = measurements.cuts_distance(cuts, target);
                if distance < *best_dist || (*fast_is_best && distance == *best_dist) {
                    *best_dist = distance;
                    *best = Some(cuts.to_vec());
                    *fast_is_best = false;
                }
                *attempt_count += 1;
                if *attempt_count >= GRID_ATTEMPT_LIMIT || *skip_count >= GRID_SKIP_LIMIT {
                    return true;
                }
            }
        }
        false
    }

    let mut threshold_attempt = 0;
    // getBestLayout keeps relaxing the threshold until there is a layout.
    // Replacing this with an arbitrary partition changes sparse/uneven grids.
    while threshold_attempt < threshold_attempts || best.is_none() {
        threshold_attempt += 1;
        let mut attempt_count = 0usize;
        let mut skip_count = 0usize;
        let mut cuts = vec![0; cuts_needed];
        visit_divisions(
            &mut measurements,
            &mut cuts,
            values.len(),
            cuts_needed,
            gap,
            target,
            threshold,
            &mut skip_count,
            &mut starting_cache,
            &mut attempt_count,
            &mut best,
            &mut best_dist,
            &mut fast_is_best,
        );
        threshold += GRID_THRESHOLD_STEP;
        starting_cache.clear();
        if skip_count == 0 {
            break;
        }
        if attempt_count == 0 && threshold_attempts < GRID_MAX_THRESHOLD_ATTEMPTS {
            threshold_attempts += 1;
        }
    }

    best.expect("grid division search finds a layout before terminating")
}

#[cfg(test)]
fn grid_row_size(values: &[f64], start: usize, end: usize, gap: f64) -> f64 {
    values[start..end].iter().sum::<f64>() + gap * (end - start).saturating_sub(1) as f64
}

// Pinned d2grid/grid_search.go caches measurements without changing the
// left-to-right sums or the bounded division search order.
struct GridRowMeasurements<'a> {
    values: &'a [f64],
    gap: f64,
    cache: Vec<Option<(usize, usize, f64, f64)>>,
    prefix_checked: bool,
    integral_prefix: Option<Vec<f64>>,
}

impl<'a> GridRowMeasurements<'a> {
    fn new(values: &'a [f64], gap: f64) -> Self {
        Self {
            values,
            gap,
            cache: Vec::new(),
            prefix_checked: false,
            integral_prefix: None,
        }
    }

    fn get(&mut self, start: usize, end: usize) -> (f64, f64) {
        if end - start <= 1 {
            return (
                self.values[start..end].iter().sum(),
                grid_row_with_gap(self.values, start, end, self.gap),
            );
        }
        if !self.prefix_checked {
            self.init_integral_prefix();
        }
        if let Some(prefix) = &self.integral_prefix {
            let size = prefix[end] - prefix[start];
            return (size, size + self.gap * (end - start - 1) as f64);
        }
        if self.cache.is_empty() {
            let mut entries = 1usize;
            while entries < 32 * 1024 && entries / self.values.len() < self.values.len() {
                entries *= 2;
            }
            self.cache.resize(entries, None);
        }
        let slot = start.wrapping_mul(self.values.len()).wrapping_add(end) & (self.cache.len() - 1);
        if let Some((cached_start, cached_end, size, with_gap)) = self.cache[slot] {
            if cached_start == start && cached_end == end {
                return (size, with_gap);
            }
        }
        let size = self.values[start..end].iter().sum();
        let with_gap = grid_row_with_gap(self.values, start, end, self.gap);
        self.cache[slot] = Some((start, end, size, with_gap));
        (size, with_gap)
    }

    // d2grid/grid_search.go:initIntegralPrefix: integer arithmetic remains
    // exact through 2^53, including the original trailing-gap intermediate.
    fn init_integral_prefix(&mut self) {
        self.prefix_checked = true;
        const MAX_EXACT: u64 = 1 << 53;
        let exact =
            |value: f64| value >= 0.0 && value <= MAX_EXACT as f64 && value.trunc() == value;
        if !exact(self.gap) {
            return;
        }
        let gap = self.gap as u64;
        let mut total = 0u64;
        for &value in self.values {
            if !exact(value) {
                return;
            }
            let size = value as u64;
            if size > MAX_EXACT - total || gap > MAX_EXACT - total - size {
                return;
            }
            total += size + gap;
        }
        let mut prefix = Vec::with_capacity(self.values.len() + 1);
        prefix.push(0.0);
        for &value in self.values {
            prefix.push(prefix.last().copied().unwrap() + value);
        }
        self.integral_prefix = Some(prefix);
    }

    fn cuts_distance(&mut self, cuts: &[usize], target: f64) -> f64 {
        let mut start = 0;
        let mut distance = 0.0;
        for cut in cuts
            .iter()
            .copied()
            .chain(std::iter::once(self.values.len() - 1))
        {
            let end = cut + 1;
            distance += (self.get(start, end).1 - target).abs();
            start = end;
        }
        distance
    }
}

// This mirrors the source helper's explicit search-state parameters.
#[allow(clippy::too_many_arguments)]
fn grid_row_ok(
    measurements: &mut GridRowMeasurements<'_>,
    gap: f64,
    target: f64,
    threshold: f64,
    start: usize,
    end: usize,
    starting: bool,
    skip_count: &mut usize,
    starting_cache: &mut BTreeMap<usize, bool>,
) -> bool {
    let length = end - start;
    if starting && let Some(ok) = starting_cache.get(&length) {
        return *ok;
    }
    let row_size = measurements.get(start, end).0 + gap * length.saturating_sub(1) as f64;
    let mut ok = true;
    if length > 1 && row_size > threshold * target {
        *skip_count += 1;
        ok = *skip_count >= GRID_SKIP_LIMIT;
        // In Go, a failed check with skip_count below the limit skips this
        // candidate; reaching the limit stops the search.  The caller cannot
        // distinguish those booleans, so encode the stop as true only at the
        // limit and let the caller's budget check terminate the traversal.
        if !ok {
            if starting {
                starting_cache.insert(length, false);
            }
            return false;
        }
    }
    if row_size < target / threshold {
        *skip_count += 1;
        ok = *skip_count >= GRID_SKIP_LIMIT;
        if !ok {
            if starting {
                starting_cache.insert(length, false);
            }
            return false;
        }
    }
    if starting {
        starting_cache.insert(length, ok);
    }
    ok
}

fn grid_cuts_distance(values: &[f64], cuts: &[usize], gap: f64, target: f64) -> f64 {
    let mut start = 0;
    let mut distance = 0.0;
    for cut in cuts
        .iter()
        .copied()
        .chain(std::iter::once(values.len() - 1))
    {
        let end = cut + 1;
        // getDistToTarget/gridRowMeasurements.withGap add each trailing gap
        // with its object, then subtract the last gap. Reassociation changes
        // fractional-size ties and can select a different ordered partition.
        let with_gap = grid_row_with_gap(values, start, end, gap);
        distance += (with_gap - target).abs();
        start = end;
    }
    distance
}

// d2grid/grid_search.go:88–99, shared by the search and its original-test replay.
// Keep each value's gap in the left-to-right sum before removing the trailing one.
fn grid_row_with_gap(values: &[f64], start: usize, end: usize, gap: f64) -> f64 {
    let mut with_gap = 0.0;
    for value in &values[start..end] {
        with_gap += value + gap;
    }
    if start < end {
        with_gap -= gap;
    }
    with_gap
}

fn grid_stddev(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let deviation = mean - value;
            deviation * deviation
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

fn grid_cell(index: usize, rows: usize, columns: usize, row_directed: bool) -> (usize, usize) {
    if row_directed {
        (index / columns, index % columns)
    } else {
        (index % rows, index / rows)
    }
}

fn grid_index(row: usize, column: usize, rows: usize, columns: usize, row_directed: bool) -> usize {
    if row_directed {
        row * columns + column
    } else {
        column * rows + row
    }
}

fn shift_subtree(
    graph: &Graph,
    result: &mut LayoutResult,
    root: NodeId,
    delta: Point,
    deltas: &mut BTreeMap<NodeId, Point>,
) {
    // Move the whole subtree in one transaction.  Translating routes here,
    // instead of rebuilding them from centers, preserves Dagre's cubic route
    // controls and also prevents recursive child visits from shifting a route
    // more than once.
    let mut ids = vec![root];
    let mut index = 0;
    while index < ids.len() {
        let parent = ids[index];
        ids.extend(
            graph
                .nodes()
                .filter_map(|(id, node)| (node.parent == Some(parent)).then_some(id)),
        );
        index += 1;
    }
    for id in ids.iter().copied() {
        if let Some(rect) = result.boxes.get_mut(&id) {
            rect.origin.x += delta.x;
            rect.origin.y += delta.y;
            deltas.insert(id, delta);
        }
    }
    for line in &mut result.sequence_lifelines {
        if ids.contains(&line.actor) {
            for point in &mut line.route {
                point.x += delta.x;
                point.y += delta.y;
            }
        }
    }
    for (edge_id, edge) in graph.edges() {
        if !ids.contains(&edge.source) || !ids.contains(&edge.target) {
            continue;
        }
        if let Some(route) = result.routes.get_mut(&edge_id) {
            for point in route {
                point.x += delta.x;
                point.y += delta.y;
            }
        }
    }
}

/// Lays out the direct children of the first root container in a grid.
///
/// Rows and columns follow the same capacity rules as D2: an explicit axis is
/// honored, the other axis grows to fit all children, and an undersized
/// explicit pair grows along the row axis. The root box is resized around the
/// cells while preserving its existing dimensions when they are large enough.
pub fn layout(graph: &Graph, options: &LayoutOptions) -> Result<LayoutResult, LayoutError> {
    validate(graph)?;
    // Grid layout does not run a seed race. This is report metadata only;
    // the selected core engine validates seeds if a nested scope invokes it.
    let seed = graph
        .tala_seed_data
        .as_ref()
        .and_then(|data| data.as_ref().ok())
        .and_then(|seeds| seeds.first())
        .or_else(|| options.seeds.first())
        .copied()
        .unwrap_or(1);
    let object_root = graph
        .nodes()
        .find_map(|(id, node)| node.parent.is_none().then_some(id));
    let synthetic_root = graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some();
    let root = if synthetic_root {
        None
    } else {
        Some(object_root.ok_or(LayoutError::IdOverflow)?)
    };
    let children = match root {
        Some(root) => graph
            .nodes()
            .filter_map(|(id, node)| (node.parent == Some(root)).then_some(id))
            .collect::<Vec<_>>(),
        None => graph
            .nodes()
            .filter_map(|(id, node)| node.parent.is_none().then_some(id))
            .collect::<Vec<_>>(),
    };
    let root_node = root.and_then(|id| graph.node(id));
    let configured_rows = root_node
        .and_then(|node| node.grid_rows)
        .or(graph.root_grid_rows);
    let configured_columns = root_node
        .and_then(|node| node.grid_columns)
        .or(graph.root_grid_columns);
    let row_directed = root
        .and_then(|id| {
            graph
                .grid_row_directed
                .get(id.0 as usize)
                .and_then(|value| *value)
        })
        .or(graph.root_grid_row_directed)
        .unwrap_or(true);
    let (rows, columns) = grid_dimensions(
        configured_rows,
        configured_columns,
        children.len(),
        row_directed,
    )?;

    let (grid_gap, root_vertical_gap, root_horizontal_gap) = root
        .map(|id| {
            let spacing = graph.grid_spacing(id);
            (
                spacing.grid_gap,
                spacing.vertical_gap,
                spacing.horizontal_gap,
            )
        })
        .unwrap_or((
            graph.root_grid_gap,
            graph.root_vertical_gap,
            graph.root_horizontal_gap,
        ));
    let root_origin = root_node
        .and_then(|node| node.locked_position)
        .unwrap_or_default();

    let dynamic_axis = match (configured_rows, configured_columns) {
        (Some(_), None) => Some(DynamicAxis::Rows),
        (None, Some(_)) => Some(DynamicAxis::Columns),
        _ => None,
    };
    if let Some(dynamic_axis) = dynamic_axis {
        let mut boxes = BTreeMap::new();
        if let Some(root) = root {
            boxes.insert(
                root,
                Rect {
                    origin: root_origin,
                    size: root_node.map_or(Size::default(), |node| node.size),
                },
            );
        }
        for child in &children {
            let node = graph.node(*child).expect("child exists");
            boxes.insert(
                *child,
                Rect {
                    origin: Point::default(),
                    size: node.size,
                },
            );
        }
        let mut result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes,
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: graph
                .nodes()
                .map(|(id, node)| {
                    (
                        id,
                        NodeLabelState {
                            font_size_committed: false,
                            size: node.label_size,
                            font_size: node.font_size,
                            position: node.label_position,
                            icon_position: node.icon_position,
                        },
                    )
                })
                .collect(),
            report: LayoutReport {
                selected_seed: seed,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 1,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let margins = prepare_grid_objects(graph, &mut result, root, &children);
        apply_dynamic_scope(
            graph,
            &mut result,
            root,
            children.clone(),
            GridScopeConfig {
                rows,
                columns,
                row_directed,
                dynamic_axis: Some(dynamic_axis),
                grid_gap,
                vertical_gap: root_vertical_gap,
                horizontal_gap: root_horizontal_gap,
            },
            dynamic_axis,
            margins,
        );
        apply_grid_label_states(graph, &mut result, root, &children);
        #[cfg(feature = "diagnostic-traces")]
        trace_grid_snapshot(graph, &result, root, "grid");
        result.edge_labels = graph
            .edges()
            .filter_map(|(id, _)| graph.edge_label(id).cloned().map(|label| (id, label)))
            .collect();
        set_direct_grid_edge_labels(graph, &mut result, root);
        result.report.candidates.push(CandidateReport {
            seed,
            pass: 0,
            score: Score::default(),
            race_score: None,
            race_score_components: None,
            race_seed_edge_order: None,
            accepted: true,
        });
        return Ok(result);
    }

    // Use the same Source-shaped sizing/spacing/inner-box placement as
    // nested grids. The old flat path used an unconditional 60px inset and
    // skipped label overflow and centering inside authored fixed dimensions.
    let mut boxes = BTreeMap::new();
    if let Some(root) = root {
        boxes.insert(
            root,
            Rect {
                origin: root_origin,
                size: root_node.unwrap().size,
            },
        );
    }
    for child in &children {
        boxes.insert(
            *child,
            Rect {
                origin: Point::default(),
                size: graph.node(*child).unwrap().size,
            },
        );
    }
    let mut result = LayoutResult {
        removed_edges: Default::default(),
        source_edge_storage: Default::default(),
        synthetic_root: None,
        route_curves: Default::default(),
        sequence_lifelines: Vec::new(),
        edge_label_percentage_commits: Default::default(),
        boxes,
        routes: BTreeMap::new(),
        edge_labels: graph
            .edges()
            .filter_map(|(id, _)| graph.edge_label(id).cloned().map(|label| (id, label)))
            .collect(),
        node_labels: graph
            .nodes()
            .map(|(id, node)| {
                (
                    id,
                    NodeLabelState {
                        font_size_committed: false,
                        size: node.label_size,
                        font_size: node.font_size,
                        position: node.label_position,
                        icon_position: node.icon_position,
                    },
                )
            })
            .collect(),
        report: LayoutReport {
            selected_seed: seed,
            selected_pass: 0,
            score: Score::default(),
            improvement_passes: 1,
            candidates: vec![CandidateReport {
                seed,
                pass: 0,
                score: Score::default(),
                race_score: None,
                race_score_components: None,
                race_seed_edge_order: None,
                accepted: true,
            }],
            warnings: Vec::new(),
        },
    };
    apply_scope(
        graph,
        &mut result,
        root,
        children,
        GridScopeConfig {
            rows,
            columns,
            row_directed,
            dynamic_axis: None,
            grid_gap,
            vertical_gap: root_vertical_gap,
            horizontal_gap: root_horizontal_gap,
        },
    );
    // The synthetic document-root scope has no container parent identity;
    // route its direct cell edges just as the object-root scope does.
    result.routes = graph
        .edges()
        .filter_map(|(id, edge)| {
            Some((
                id,
                grid_edge_route(
                    graph,
                    id,
                    *result.boxes.get(&edge.source)?,
                    *result.boxes.get(&edge.target)?,
                    &result.node_labels,
                ),
            ))
        })
        .collect();
    set_direct_grid_edge_labels(graph, &mut result, root);
    #[cfg(feature = "diagnostic-traces")]
    trace_grid_snapshot(graph, &result, root, "grid");
    Ok(result)
}

fn grid_edge_route(
    graph: &Graph,
    edge_id: d2_rs_core::EdgeId,
    source: Rect,
    target: Rect,
    labels: &BTreeMap<NodeId, NodeLabelState>,
) -> Vec<Point> {
    d2_rs_dagre::route_straight_edge_with_labels(graph, edge_id, source, target, Some(labels))
        .unwrap_or_else(|| vec![source.center(), target.center()])
}

fn validate(graph: &Graph) -> Result<(), LayoutError> {
    for (id, node) in graph.nodes() {
        if !node.size.width.is_finite()
            || !node.size.height.is_finite()
            || node.size.width < 0.0
            || node.size.height < 0.0
        {
            return Err(LayoutError::InvalidSize {
                node: id,
                width: node.size.width,
                height: node.size.height,
            });
        }
    }
    for (id, edge) in graph.edges() {
        if graph.node(edge.source).is_none() || graph.node(edge.target).is_none() {
            return Err(LayoutError::MissingEndpoint { edge: id });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn integral_grid_prefix_preserves_row_bits_and_rejects_inexact_inputs() {
        for (values, gap, uses_prefix) in [
            (vec![1.0, 2.0, 9.0, 0.0], 3.0, true),
            (vec![1.0, 2.5, 9.0], 3.0, false),
            (vec![1.0, 2.0, 9.0], 0.5, false),
            (vec![(1u64 << 53) as f64, 1.0], 0.0, false),
        ] {
            let mut measurements = super::GridRowMeasurements::new(&values, gap);
            for start in 0..values.len() {
                for end in start..=values.len() {
                    let (size, with_gap) = measurements.get(start, end);
                    assert_eq!(
                        size.to_bits(),
                        values[start..end].iter().sum::<f64>().to_bits()
                    );
                    assert_eq!(
                        with_gap.to_bits(),
                        super::grid_row_with_gap(&values, start, end, gap).to_bits()
                    );
                }
            }
            assert_eq!(measurements.integral_prefix.is_some(), uses_prefix);
        }
    }
    #[test]
    fn cached_grid_rows_preserve_sum_bits_with_collisions() {
        let values: Vec<f64> = (0..200).map(|i| (i as f64 + 0.3) / 7.0).collect();
        let gap = 0.17;
        let mut measurements = super::GridRowMeasurements::new(&values, gap);
        for _ in 0..2 {
            for start in 0..values.len() {
                for end in start..=values.len() {
                    let (size, with_gap) = measurements.get(start, end);
                    let expected: f64 = values[start..end].iter().sum();
                    assert_eq!(size.to_bits(), expected.to_bits());
                    assert_eq!(
                        with_gap.to_bits(),
                        super::grid_row_with_gap(&values, start, end, gap).to_bits()
                    );
                }
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn fixed_flat_grid_placement_matches_pristine_graph_float_bits() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/grid-fixed-placement-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 18);
        for row in rows.as_array().unwrap() {
            assert_eq!(row["error"], "");
            let parts = row["name"].as_str().unwrap().split('-').collect::<Vec<_>>();
            let width = parts[0].parse::<f64>().unwrap();
            let height = parts[1].parse::<f64>().unwrap();
            let gap = parts[2].parse::<f64>().unwrap();
            let mut graph = Graph::default();
            let mut ids = Vec::new();
            for (index, object) in row["objects"].as_array().unwrap().iter().enumerate() {
                let size = Size {
                    width: object["width"].as_f64().unwrap(),
                    height: object["height"].as_f64().unwrap(),
                };
                let mut node = Node::new(object["id"].as_str().unwrap(), size);
                node.label_size = Some(Size {
                    width: object["label_width"].as_f64().unwrap(),
                    height: object["label_height"].as_f64().unwrap(),
                });
                if index == 0 {
                    node.grid_rows = Some(2);
                    node.grid_columns = Some(2);
                    node.fixed_width = true;
                    node.fixed_height = true;
                    node.declared_size = Some(Size { width, height });
                    node.font_size = Some(28);
                } else {
                    node.parent = Some(ids[0]);
                    node.font_size = Some(16);
                }
                ids.push(graph.add_node(node));
            }
            graph.set_grid_spacing(
                ids[0],
                d2_rs_core::GridSpacing {
                    grid_gap: Some(gap),
                    ..Default::default()
                },
            );
            graph.grid_row_directed[ids[0].0 as usize] = Some(parts[3] == "rows");
            let result = layout(&graph, &LayoutOptions::default()).unwrap();
            for (id, object) in ids.iter().zip(row["objects"].as_array().unwrap()) {
                let rect = result.boxes[id];
                let bits = [
                    rect.origin.x.to_bits(),
                    rect.origin.y.to_bits(),
                    rect.size.width.to_bits(),
                    rect.size.height.to_bits(),
                ];
                let expected = object["bits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_u64().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(
                    bits.as_slice(),
                    expected.as_slice(),
                    "{} {}",
                    row["name"],
                    object["id"]
                );
            }
        }
    }

    use super::apply_scopes;
    use super::layout;
    use super::shift_subtree;
    use super::{best_grid_cuts, grid_shape_shift, shape_dimensions_to_fit};
    use d2_rs_core::{
        Edge, Graph, LabelPosition, LayoutOptions, LayoutReport, LayoutResult, Node,
        NodeLabelState, Point, Rect, Score, ShapeKind, Size,
    };
    use std::collections::BTreeMap;

    #[test]
    fn fixed_grid_edge_labels_are_centered_in_both_axis_modes() {
        for columns in [None, Some(2)] {
            let mut graph = Graph::default();
            let mut root = Node::new(
                "grid",
                Size {
                    width: 1.0,
                    height: 1.0,
                },
            );
            root.grid_rows = Some(1);
            root.grid_columns = columns;
            let root = graph.add_node(root);
            let mut a = Node::new(
                "grid.a",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            );
            a.parent = Some(root);
            let a = graph.add_node(a);
            let mut b = Node::new(
                "grid.b",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            );
            b.parent = Some(root);
            let b = graph.add_node(b);
            let edge = graph.add_edge(Edge {
                source: a,
                target: b,
            });
            graph.set_edge_label(
                edge,
                Some(d2_rs_core::EdgeLabel {
                    text: "fixed".into(),
                    size: Size {
                        width: 30.0,
                        height: 12.0,
                    },
                    position: LabelPosition::OutsideTopCenter,
                    percentage: 0.0,
                }),
            );
            let result = layout(&graph, &LayoutOptions::default()).unwrap();
            assert_eq!(
                result.edge_labels[&edge].position,
                LabelPosition::InsideMiddleCenter
            );
            assert_eq!(result.edge_labels[&edge].percentage, 0.0);
        }
    }

    #[test]
    fn nested_sequence_grid_scopes_do_not_relayout_sequence_actors() {
        let mut graph = Graph::default();
        let sequence = graph.add_node(Node::new(
            "sequence",
            Size {
                width: 200.0,
                height: 120.0,
            },
        ));
        graph.set_sequence_diagram(sequence, true);
        let actor = graph.add_node(Node {
            grid_columns: Some(3),
            parent: Some(sequence),
            ..Node::new(
                "sequence.actor",
                Size {
                    width: 80.0,
                    height: 24.0,
                },
            )
        });
        for name in ["x", "y", "z"] {
            graph.add_node(Node {
                parent: Some(actor),
                ..Node::new(
                    format!("sequence.actor.{name}"),
                    Size {
                        width: 20.0,
                        height: 16.0,
                    },
                )
            });
        }

        // Sequence layout owns actor internals. The enclosing layout result
        // deliberately has no boxes for those internals, as in D2's
        // extracted SequenceDiagram graph.
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::new(),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let output = apply_scopes(&graph, result, &LayoutOptions::default()).unwrap();
        assert!(output.boxes.is_empty());
    }

    #[test]
    fn nested_grid_root_and_detached_parent_cell_have_distinct_label_defaults() {
        let mut graph = Graph {
            root_grid_columns: Some(1),
            ..Graph::default()
        };
        let grid = graph.add_node(Node {
            grid_columns: Some(1),
            label_size: Some(Size {
                width: 20.0,
                height: 20.0,
            }),
            ..Node::new(
                "grid",
                Size {
                    width: 100.0,
                    height: 100.0,
                },
            )
        });
        graph.add_node(Node {
            parent: Some(grid),
            ..Node::new(
                "grid.child",
                Size {
                    width: 40.0,
                    height: 40.0,
                },
            )
        });
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: graph
                .nodes()
                .map(|(id, node)| {
                    (
                        id,
                        Rect {
                            origin: Point::default(),
                            size: node.size,
                        },
                    )
                })
                .collect(),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        assert_eq!(
            super::effective_grid_positions(&graph, Some(&result), grid, false).0,
            LabelPosition::InsideTopCenter
        );
        assert_eq!(
            super::effective_grid_positions(&graph, Some(&result), grid, true).0,
            LabelPosition::InsideMiddleCenter
        );
        let output = apply_scopes(&graph, result, &LayoutOptions::default()).unwrap();
        assert_eq!(
            output.node_labels[&grid].position,
            LabelPosition::InsideTopCenter
        );
        assert_eq!(
            super::effective_grid_positions(&graph, Some(&output), grid, true).0,
            LabelPosition::InsideMiddleCenter
        );
    }

    #[test]
    fn sequence_cell_injects_root_labels_after_grid_placeholder_layout() {
        // Actual Go grid/InjectNested observations: an automatic sequence
        // placeholder uses middle-center, an authored one keeps bottom-center,
        // then the completed sequence root overwrites both with top-center.
        for authored in [LabelPosition::Unset, LabelPosition::InsideBottomCenter] {
            let mut graph = Graph {
                root_grid_columns: Some(1),
                ..Graph::default()
            };
            let sequence = graph.add_node(Node {
                label_position: authored,
                label_size: Some(Size {
                    width: 72.0,
                    height: 21.0,
                }),
                ..Node::new(
                    "timeline",
                    Size {
                        width: 224.0,
                        height: 236.0,
                    },
                )
            });
            graph.set_sequence_diagram(sequence, true);
            graph.add_node(Node {
                parent: Some(sequence),
                ..Node::new(
                    "timeline.actor",
                    Size {
                        width: 100.0,
                        height: 30.0,
                    },
                )
            });
            let mut result = super::super::empty_layout_result(&LayoutOptions::default());
            for (id, node) in graph.nodes() {
                result.boxes.insert(
                    id,
                    Rect {
                        origin: Point::default(),
                        size: node.size,
                    },
                );
            }
            result.node_labels.insert(
                sequence,
                NodeLabelState {
                    font_size_committed: false,
                    size: graph.node(sequence).unwrap().label_size,
                    font_size: None,
                    position: LabelPosition::InsideTopCenter,
                    icon_position: None,
                },
            );
            let placeholder = if authored == LabelPosition::Unset {
                LabelPosition::InsideMiddleCenter
            } else {
                authored
            };
            let mut packing = result.clone();
            super::apply_scope(
                &graph,
                &mut packing,
                None,
                vec![sequence],
                super::GridScopeConfig {
                    rows: 1,
                    columns: 1,
                    row_directed: true,
                    dynamic_axis: Some(super::DynamicAxis::Columns),
                    grid_gap: None,
                    vertical_gap: None,
                    horizontal_gap: None,
                },
            );
            assert_eq!(packing.node_labels[&sequence].position, placeholder);
            assert_eq!(packing.boxes[&sequence], result.boxes[&sequence]);
            let output = apply_scopes(&graph, result, &LayoutOptions::default()).unwrap();
            assert_eq!(
                output.node_labels[&sequence].position,
                LabelPosition::InsideTopCenter
            );
        }
    }

    #[test]
    fn root_sequence_diagrams_do_not_run_the_root_grid_pass() {
        let mut graph = Graph {
            root_grid_columns: Some(2),
            root_sequence_diagram: true,
            ..Graph::default()
        };
        for name in ["a", "b"] {
            graph.add_node(Node::new(
                name,
                Size {
                    width: 30.0,
                    height: 20.0,
                },
            ));
        }
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::new(),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let output = apply_scopes(&graph, result, &LayoutOptions::default()).unwrap();
        assert!(output.boxes.is_empty());
    }

    #[test]
    fn grid_defaults_ignore_temporary_outside_label_positions_from_layout() {
        let mut graph = Graph::default();
        let outer = graph.add_node(Node::new(
            "outer",
            Size {
                width: 400.0,
                height: 300.0,
            },
        ));
        let grid = graph.add_node(Node {
            parent: Some(outer),
            grid_columns: Some(2),
            ..Node::new(
                "outer.grid",
                Size {
                    width: 200.0,
                    height: 150.0,
                },
            )
        });
        let a = graph.add_node(Node {
            parent: Some(grid),
            label_size: Some(Size {
                width: 8.0,
                height: 21.0,
            }),
            ..Node::new(
                "outer.grid.a",
                Size {
                    width: 50.0,
                    height: 72.0,
                },
            )
        });
        let b = graph.add_node(Node {
            parent: Some(grid),
            label_size: Some(Size {
                width: 8.0,
                height: 21.0,
            }),
            ..Node::new(
                "outer.grid.b",
                Size {
                    width: 50.0,
                    height: 30.0,
                },
            )
        });
        let loose = graph.add_node(Node {
            parent: Some(outer),
            label_size: Some(Size {
                width: 8.0,
                height: 21.0,
            }),
            ..Node::new(
                "outer.loose",
                Size {
                    width: 50.0,
                    height: 30.0,
                },
            )
        });
        let mut boxes = BTreeMap::new();
        boxes.insert(
            outer,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 400.0,
                    height: 300.0,
                },
            },
        );
        boxes.insert(
            grid,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 200.0,
                    height: 150.0,
                },
            },
        );
        boxes.insert(
            a,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 50.0,
                    height: 72.0,
                },
            },
        );
        boxes.insert(
            b,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 50.0,
                    height: 30.0,
                },
            },
        );
        boxes.insert(
            loose,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 50.0,
                    height: 30.0,
                },
            },
        );
        let output = apply_scopes(
            &graph,
            LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                edge_label_percentage_commits: Default::default(),
                boxes,
                routes: BTreeMap::new(),
                edge_labels: BTreeMap::new(),
                node_labels: BTreeMap::from([
                    (
                        a,
                        NodeLabelState {
                            font_size_committed: false,
                            size: Some(Size {
                                width: 8.0,
                                height: 21.0,
                            }),
                            font_size: Some(16),
                            position: LabelPosition::InsideMiddleCenter,
                            icon_position: None,
                        },
                    ),
                    (
                        b,
                        NodeLabelState {
                            font_size_committed: false,
                            size: Some(Size {
                                width: 8.0,
                                height: 21.0,
                            }),
                            font_size: Some(16),
                            position: LabelPosition::OutsideBottomCenter,
                            icon_position: None,
                        },
                    ),
                    (
                        loose,
                        NodeLabelState {
                            font_size_committed: false,
                            size: Some(Size {
                                width: 8.0,
                                height: 21.0,
                            }),
                            font_size: Some(16),
                            position: LabelPosition::OutsideBottomCenter,
                            icon_position: None,
                        },
                    ),
                ]),
                report: LayoutReport {
                    selected_seed: 0,
                    selected_pass: 0,
                    score: Score::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            },
            &LayoutOptions::default(),
        )
        .unwrap();

        assert_eq!(output.boxes[&a].size.height, 72.0);
        assert_eq!(output.boxes[&b].size.height, 72.0);
        assert_eq!(
            output.node_labels[&b].position,
            LabelPosition::InsideMiddleCenter
        );
        assert_eq!(
            output.node_labels[&loose].position,
            LabelPosition::OutsideBottomCenter
        );
    }

    #[test]
    fn grid_owned_routes_clip_centers_to_shape_borders() {
        let mut graph = Graph::default();
        let scope = graph.add_node(Node {
            grid_columns: Some(2),
            ..Node::new(
                "grid",
                Size {
                    width: 260.0,
                    height: 150.0,
                },
            )
        });
        let source = graph.add_node(Node {
            parent: Some(scope),
            ..Node::new(
                "grid.source",
                Size {
                    width: 52.0,
                    height: 66.0,
                },
            )
        });
        let target = graph.add_node(Node {
            parent: Some(scope),
            ..Node::new(
                "grid.target",
                Size {
                    width: 53.0,
                    height: 66.0,
                },
            )
        });
        let edge = graph.add_edge(Edge { source, target });
        let mut result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (scope, Rect::default()),
                (source, Rect::default()),
                (target, Rect::default()),
            ]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };

        super::apply_scope(
            &graph,
            &mut result,
            Some(scope),
            vec![source, target],
            super::GridScopeConfig {
                rows: 1,
                columns: 2,
                row_directed: true,
                dynamic_axis: None,
                grid_gap: None,
                vertical_gap: None,
                horizontal_gap: None,
            },
        );

        let source_box = result.boxes[&source];
        let target_box = result.boxes[&target];
        assert_eq!(
            result.routes[&edge],
            vec![
                Point {
                    x: source_box.right(),
                    y: source_box.center().y,
                },
                Point {
                    x: target_box.origin.x,
                    y: target_box.center().y,
                },
            ]
        );
    }

    #[test]
    fn dynamic_grid_search_matches_d2_reverse_order_and_thresholds() {
        let cases = [
            (vec![100.0, 10.0, 20.0, 30.0, 40.0], 2, vec![1]),
            (vec![10.0, 100.0, 20.0, 90.0, 30.0, 80.0], 3, vec![1, 3]),
            (
                vec![20.0, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0],
                3,
                vec![2, 5],
            ),
            (vec![20.0, 20.0, 20.0, 20.0, 20.0], 3, vec![0, 2]),
            (
                vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
                4,
                vec![2, 4, 6],
            ),
        ];
        for (values, groups, expected) in cases {
            let target = (values.iter().sum::<f64>()
                + 40.0 * values.len().saturating_sub(groups) as f64)
                / groups as f64;
            assert_eq!(best_grid_cuts(&values, groups, 40.0, target), expected);
        }
    }

    #[test]
    fn shaped_grid_uses_d2_size_to_content_and_inner_box_geometry() {
        let circle = shape_dimensions_to_fit(
            ShapeKind::Circle,
            Size {
                width: 437.0,
                height: 292.0,
            },
        );
        assert_eq!(
            circle,
            Size {
                width: 619.0,
                height: 619.0
            }
        );

        let oval = shape_dimensions_to_fit(
            ShapeKind::Oval,
            Size {
                width: 437.0,
                height: 292.0,
            },
        );
        assert_eq!(
            oval,
            Size {
                width: 619.0,
                height: 413.0
            }
        );

        let shift = grid_shape_shift(
            ShapeKind::Circle,
            circle,
            Size {
                width: 437.0,
                height: 292.0,
            },
        );
        assert_eq!(shift.x, 91.0);
        assert_eq!(shift.y, 163.5);
    }

    #[test]
    fn explicit_grid_dimensions_place_children_and_resize_root() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node {
            grid_rows: Some(2),
            grid_columns: Some(2),
            size: Size {
                width: 1.0,
                height: 1.0,
            },
            ..Node::new(
                "root",
                Size {
                    width: 1.0,
                    height: 1.0,
                },
            )
        });
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 20.0,
                height: 10.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 30.0,
                height: 10.0,
            },
        ));
        graph.node_mut(a).unwrap().parent = Some(root);
        graph.node_mut(b).unwrap().parent = Some(root);
        graph.add_edge(Edge {
            source: a,
            target: b,
        });
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(result.boxes.len(), 3);
        assert!(result.boxes[&root].size.width >= 150.0);
        assert_eq!(result.routes.len(), 1);
    }

    #[test]
    fn column_directed_grid_fills_down_columns() {
        let mut graph = Graph {
            root_grid_rows: Some(2),
            root_grid_columns: Some(2),
            root_grid_row_directed: Some(false),
            ..Graph::default()
        };
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 20.0,
                height: 10.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let c = graph.add_node(Node::new(
            "c",
            Size {
                width: 60.0,
                height: 50.0,
            },
        ));
        let d = graph.add_node(Node::new(
            "d",
            Size {
                width: 80.0,
                height: 70.0,
            },
        ));
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(result.boxes[&a].origin.x, result.boxes[&b].origin.x);
        assert!(result.boxes[&b].origin.y > result.boxes[&a].origin.y);
        assert!(result.boxes[&c].origin.x > result.boxes[&a].origin.x);
        assert_eq!(result.boxes[&c].origin.y, result.boxes[&a].origin.y);
        assert!(result.boxes[&d].origin.x > result.boxes[&b].origin.x);
    }

    #[test]
    fn rows_only_grid_partitions_children_by_measured_width() {
        let mut graph = Graph {
            root_grid_rows: Some(2),
            root_grid_gap: Some(0.0),
            ..Graph::default()
        };
        let ids = [
            ("a", 80.0),
            ("b", 80.0),
            ("c", 54.0),
            ("d", 54.0),
            ("e", 52.0),
        ]
        .into_iter()
        .map(|(name, width)| {
            graph.add_node(Node::new(
                name,
                Size {
                    width,
                    height: 66.0,
                },
            ))
        })
        .collect::<Vec<_>>();
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(
            result.boxes[&ids[0]].origin.y,
            result.boxes[&ids[1]].origin.y
        );
        assert!(result.boxes[&ids[2]].origin.y > result.boxes[&ids[1]].origin.y);
        assert_eq!(
            result.boxes[&ids[2]].origin.y,
            result.boxes[&ids[4]].origin.y
        );
        assert_eq!(result.boxes[&ids[2]].origin.x, 0.0);
    }

    #[test]
    fn columns_only_grid_partitions_children_by_measured_height() {
        let mut graph = Graph {
            root_grid_columns: Some(2),
            root_grid_gap: Some(0.0),
            root_grid_row_directed: Some(false),
            ..Graph::default()
        };
        let ids = [
            ("a", 66.0),
            ("b", 66.0),
            ("c", 66.0),
            ("d", 99.0),
            ("e", 99.0),
        ]
        .into_iter()
        .map(|(name, height)| {
            graph.add_node(Node::new(
                name,
                Size {
                    width: 55.0,
                    height,
                },
            ))
        })
        .collect::<Vec<_>>();
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(
            result.boxes[&ids[0]].origin.x,
            result.boxes[&ids[1]].origin.x
        );
        assert_eq!(
            result.boxes[&ids[2]].origin.x,
            result.boxes[&ids[0]].origin.x
        );
        assert!(result.boxes[&ids[3]].origin.x > result.boxes[&ids[2]].origin.x);
        assert_eq!(
            result.boxes[&ids[4]].origin.x,
            result.boxes[&ids[3]].origin.x
        );
    }

    #[test]
    fn moving_a_grid_subtree_translates_owned_cubic_routes_once() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node::new(
            "root",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let source = graph.add_node(Node::new(
            "source",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let target = graph.add_node(Node::new(
            "target",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        graph.node_mut(source).unwrap().parent = Some(root);
        graph.node_mut(target).unwrap().parent = Some(root);
        let edge = graph.add_edge(Edge { source, target });
        let original = vec![
            Point { x: 10.0, y: 20.0 },
            Point { x: 30.0, y: 20.0 },
            Point { x: 40.0, y: 30.0 },
            Point { x: 60.0, y: 40.0 },
        ];
        let mut result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: [
                (root, Rect::default()),
                (source, Rect::default()),
                (target, Rect::default()),
            ]
            .into_iter()
            .collect(),
            routes: [(edge, original.clone())].into_iter().collect(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: d2_rs_core::LayoutReport {
                selected_seed: 0,
                selected_pass: 0,
                score: Default::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let mut deltas = BTreeMap::new();
        shift_subtree(
            &graph,
            &mut result,
            root,
            Point { x: 7.0, y: 11.0 },
            &mut deltas,
        );
        let shifted = result.routes.get(&edge).expect("route remains present");
        assert_eq!(shifted.len(), original.len());
        for (actual, expected) in shifted.iter().zip(original) {
            assert_eq!(actual.x, expected.x + 7.0);
            assert_eq!(actual.y, expected.y + 11.0);
        }
    }
}
