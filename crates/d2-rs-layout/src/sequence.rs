// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Measured-graph sequence layout corresponding to D2's `d2sequence` package.

use super::{EngineKind, LayoutDispatcher};
use d2_rs_core::{
    CancellationToken, CandidateReport, Edge, EdgeId, Graph, LabelPosition, LayoutError,
    LayoutOptions, LayoutReport, LayoutResult, NodeId, NodeLabelState, Point, Rect, Score, Size,
};
use std::collections::{BTreeMap, BTreeSet};

/// Minimum distance between actor centers.
pub const MIN_ACTOR_DISTANCE: f64 = 150.0;
/// Minimum distance between message rows.
pub const MIN_MESSAGE_DISTANCE: f64 = 30.0;
/// Horizontal padding around the sequence diagram.
pub const GROUP_CONTAINER_PADDING: f64 = 12.0;
/// Minimum width assigned to an actor.
pub const MIN_ACTOR_WIDTH: f64 = 100.0;
/// Vertical padding below actor boxes and above the first message.
pub const VERTICAL_PADDING: f64 = 40.0;
/// Horizontal travel used for self messages and messages to descendants.
pub const SELF_MESSAGE_HORIZONTAL_TRAVEL: f64 = 80.0;
/// Horizontal clearance used when sizing sequence groups and actor spacing.
pub const HORIZONTAL_PAD: f64 = 40.0;
/// Horizontal clearance around message labels.
pub const LABEL_HORIZONTAL_PAD: f64 = 60.0;
/// Minimum height of a nested sequence span.
pub const MIN_SPAN_HEIGHT: f64 = 30.0;
/// Width of a nested sequence span.
pub const SPAN_BASE_WIDTH: f64 = 12.0;
/// Additional span width for each nesting level.
pub const SPAN_DEPTH_GROWTH_FACTOR: f64 = 8.0;
/// Clearance around a span's first and last message.
pub const SPAN_MESSAGE_PAD: f64 = 10.0;
/// Extra vertical room reserved for a labeled edge group.
pub const EDGE_GROUP_LABEL_PADDING: f64 = 20.0;

/// Lays out a marked sequence-diagram scope with stable actor and message
/// ordering.
///
/// Actors are direct children of the marked root (or synthetic document root).
/// Their source order is preserved, nested spans are sized around their
/// messages, notes are stacked by source order, and self/descendant messages
/// use D2's elbow route. Synthetic lifeline edges remain outside the shared
/// edge arena and are reported as a follow-up boundary.
pub fn layout(graph: &Graph, options: &LayoutOptions) -> Result<LayoutResult, LayoutError> {
    let root = if graph.root_sequence_diagram {
        None
    } else {
        graph
            .nodes()
            .find_map(|(id, _)| graph.is_sequence_diagram(id).then_some(id))
    };
    layout_inner(graph, options, root, EngineKind::Tala, false, None)
}

/// Lays out a root sequence diagram after applying LayoutNested's actor
/// subtree passes with the selected core engine.
pub fn layout_with_nested(
    graph: &Graph,
    options: &LayoutOptions,
    engine: EngineKind,
) -> Result<LayoutResult, LayoutError> {
    layout_with_nested_and_cancellation(graph, options, engine, None)
}

/// Carries context to extracted core layouts and crossing-edge routing. The
/// sequence algorithm itself does not observe cancellation in D2 v0.9.0.
pub fn layout_with_nested_and_cancellation(
    graph: &Graph,
    options: &LayoutOptions,
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
) -> Result<LayoutResult, LayoutError> {
    let root = if graph.root_sequence_diagram {
        None
    } else {
        graph
            .nodes()
            .find_map(|(id, _)| graph.is_sequence_diagram(id).then_some(id))
    };
    let result = layout_inner(graph, options, root, engine, true, cancellation)?;
    validate_scope_positions(graph, &result, root)?;
    Ok(result)
}

/// Lays out one explicit sequence-diagram scope inside a larger graph.
///
/// The general TALA pass owns the surrounding document. This entry point is
/// used by the dispatcher to replace the geometry for one nested sequence
/// scope after that pass has established its containing rectangle.
pub fn layout_scope(
    graph: &Graph,
    options: &LayoutOptions,
    root: NodeId,
    engine: EngineKind,
) -> Result<LayoutResult, LayoutError> {
    layout_scope_with_cancellation(graph, options, root, engine, None)
}

/// Lays out an extracted sequence scope with the parent context.
pub fn layout_scope_with_cancellation(
    graph: &Graph,
    options: &LayoutOptions,
    root: NodeId,
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
) -> Result<LayoutResult, LayoutError> {
    let result = layout_inner(graph, options, Some(root), engine, true, cancellation)?;
    validate_scope_positions(graph, &result, Some(root))?;
    Ok(result)
}

/// The parent-graph view used when a Dagre document contains nested sequence
/// scopes. D2 extracts each sequence graph before running its enclosing core
/// layout; this projection keeps the same boundary while retaining the
/// original node and edge IDs for the later geometry replacement.
pub struct DagreSequenceProjection {
    /// Graph with sequence descendants removed from parent ranking.
    pub graph: Graph,
    /// Geometry produced for each extracted sequence scope.
    pub locals: BTreeMap<NodeId, LayoutResult>,
    /// Original node ID for each projected node ID, in projected arena order.
    pub node_map: Vec<NodeId>,
    /// Original edge ID for each projected edge ID. Edges touching extracted
    /// sequence descendants are omitted until their actual endpoints return.
    pub edge_map: Vec<EdgeId>,
}

/// Restore the sequence-local geometry removed from a Dagre parent graph.
///
/// `project_for_dagre` lays out each sequence scope before the enclosing
/// graph so Dagre can size its placeholder. Those local results must also be
/// committed after Dagre places the placeholders; otherwise the projection
/// drops every internal sequence route and descendant box from the final
/// result.
pub fn apply_projection_locals(
    graph: &Graph,
    mut result: LayoutResult,
    locals: BTreeMap<NodeId, LayoutResult>,
) -> LayoutResult {
    for (root, local) in locals {
        result.removed_edges.extend(&local.removed_edges);
        let Some(local_root) = local.boxes.get(&root).copied() else {
            continue;
        };
        let anchor = result.boxes.get(&root).copied().unwrap_or(local_root);
        let delta = Point {
            x: anchor.origin.x - local_root.origin.x,
            y: anchor.origin.y - local_root.origin.y,
        };
        // LayoutNested fits the placeholder before the parent engine runs.
        // InjectNested restores contents and attributes afterward, but never
        // replaces the parent's resulting container dimensions.
        result.boxes.insert(root, anchor);
        for (id, mut rect) in local.boxes {
            if id != root && is_descendant_or_self(graph, id, Some(root)) {
                rect.origin.x += delta.x;
                rect.origin.y += delta.y;
                result.boxes.insert(id, rect);
            }
        }
        result
            .sequence_lifelines
            .retain(|line| !is_descendant_or_self(graph, line.actor, Some(root)));
        for mut line in local.sequence_lifelines {
            for point in &mut line.route {
                point.x += delta.x;
                point.y += delta.y;
            }
            result.sequence_lifelines.push(line);
        }
        for (edge_id, mut route) in local.routes {
            let Some(edge) = graph.edges.get(edge_id.0 as usize) else {
                continue;
            };
            if !is_descendant_or_self(graph, edge.source, Some(root))
                || !is_descendant_or_self(graph, edge.target, Some(root))
            {
                continue;
            }
            for point in &mut route {
                point.x += delta.x;
                point.y += delta.y;
            }
            result.routes.insert(edge_id, route);
        }
        for (edge_id, label) in local.edge_labels {
            let Some(edge) = graph.edges.get(edge_id.0 as usize) else {
                continue;
            };
            if is_descendant_or_self(graph, edge.source, Some(root))
                && is_descendant_or_self(graph, edge.target, Some(root))
            {
                result.edge_labels.insert(edge_id, label);
                if local.edge_label_percentage_commits.contains(&edge_id) {
                    result.edge_label_percentage_commits.insert(edge_id);
                }
            }
        }
        for (id, label) in local.node_labels {
            if id == root || is_descendant_or_self(graph, id, Some(root)) {
                result.node_labels.insert(id, label);
            }
        }
    }
    result
}

/// Backwards-compatible Dagre-named entry point for restoring sequence-local
/// geometry. The same projection is now used by TALA because D2 extracts
/// nested sequence diagrams before invoking either parent engine.
pub fn apply_dagre_projection_locals(
    graph: &Graph,
    result: LayoutResult,
    locals: BTreeMap<NodeId, LayoutResult>,
) -> LayoutResult {
    apply_projection_locals(graph, result, locals)
}

/// Build the nested-sequence projection consumed by the Dagre dispatcher.
pub fn project_for_dagre(
    graph: &Graph,
    options: &LayoutOptions,
) -> Result<Option<DagreSequenceProjection>, LayoutError> {
    project_for_dagre_with_cache(graph, options, None, None)
}

pub(super) fn project_for_tala_with_cache(
    graph: &Graph,
    options: &LayoutOptions,
    scope_cache: Option<&mut super::DagreScopeCache>,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<DagreSequenceProjection>, LayoutError> {
    project_for_engine_with_cache(graph, options, scope_cache, EngineKind::Tala, cancellation)
}

pub(super) fn project_for_dagre_with_cache(
    graph: &Graph,
    options: &LayoutOptions,
    scope_cache: Option<&mut super::DagreScopeCache>,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<DagreSequenceProjection>, LayoutError> {
    project_for_engine_with_cache(graph, options, scope_cache, EngineKind::Dagre, cancellation)
}

pub(super) fn project_for_elk_with_cache(
    graph: &Graph,
    options: &LayoutOptions,
    scope_cache: Option<&mut super::DagreScopeCache>,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<DagreSequenceProjection>, LayoutError> {
    project_for_engine_with_cache(graph, options, scope_cache, EngineKind::Elk, cancellation)
}

fn project_for_engine_with_cache(
    graph: &Graph,
    options: &LayoutOptions,
    mut scope_cache: Option<&mut super::DagreScopeCache>,
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<DagreSequenceProjection>, LayoutError> {
    let roots = graph
        .nodes()
        .filter_map(|(id, _)| {
            (graph.is_sequence_diagram(id)
                && sequence_owner(graph, id).is_none()
                && graph.nodes().any(|(_, child)| child.parent == Some(id))
                && !sequence_belongs_to_grid_extraction(graph, id))
            .then_some(id)
        })
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Ok(None);
    }

    let mut locals = BTreeMap::new();
    for root in roots.iter().copied() {
        let cache_key = graph
            .node(root)
            .map(|node| node.external_id.as_str())
            .unwrap_or_default();
        let cached = scope_cache
            .as_deref()
            .and_then(|cache| cache.sequences.get(cache_key))
            .cloned();
        let local = match cached.and_then(|cached| super::restore_layout_result(graph, &cached)) {
            Some(local) => local,
            None => {
                let local = layout_scope_with_cancellation(
                    graph,
                    options,
                    root,
                    engine.clone(),
                    cancellation,
                )?;
                if let Some(cache) = scope_cache.as_deref_mut()
                    && let Some(node) = graph.node(root)
                {
                    cache.sequences.insert(
                        node.external_id.clone(),
                        super::cache_layout_result(graph, &local),
                    );
                }
                local
            }
        };
        locals.insert(root, local);
    }

    let mut projected = graph.clone();
    for root in roots.iter().copied() {
        if let Some(local_root) = locals
            .get(&root)
            .and_then(|result| result.boxes.get(&root))
            .copied()
            && let Some(node) = projected.node_mut(root)
        {
            node.size = local_root.size;
            node.declared_size = Some(local_root.size);
        }
    }

    let selected_roots = roots.iter().copied().collect::<BTreeSet<_>>();
    let extracted_ancestor = |id| {
        let mut parent = graph.node(id).and_then(|node| node.parent);
        while let Some(candidate) = parent {
            if selected_roots.contains(&candidate) {
                return true;
            }
            parent = graph.node(candidate).and_then(|node| node.parent);
        }
        false
    };

    let mut edge_map = Vec::new();
    let mut keep_edges = Vec::new();
    for (edge_id, edge) in graph.edges.iter().enumerate() {
        // ExtractSubgraph removes both internal and crossing descendant
        // edges. A crossing edge is restored only after its real endpoints
        // are reinjected; substituting the root changes parent placement.
        if extracted_ancestor(edge.source) || extracted_ancestor(edge.target) {
            continue;
        }
        keep_edges.push(edge_id);
        edge_map.push(EdgeId(edge_id as u32));
    }
    projected.edges = keep_edges
        .iter()
        .filter_map(|index| graph.edges.get(*index).cloned())
        .collect();
    projected.sequence_edge_lines = keep_edges
        .iter()
        .enumerate()
        .filter_map(|(index, old)| {
            graph
                .sequence_edge_lines
                .get(&EdgeId(*old as u32))
                .map(|line| (EdgeId(index as u32), *line))
        })
        .collect();
    retain_edge_data(&mut projected.tala_edge_ids, &keep_edges);
    retain_edge_data(&mut projected.tala_edge_invisible, &keep_edges);
    retain_edge_data(&mut projected.tala_edge_label_fixed, &keep_edges);
    retain_edge_data(&mut projected.edge_indices, &keep_edges);
    retain_edge_data(&mut projected.edge_arrows, &keep_edges);
    retain_edge_data(&mut projected.edge_arrowheads, &keep_edges);
    retain_edge_data(&mut projected.edge_arrowhead_declarations, &keep_edges);
    retain_edge_data(&mut projected.edge_labels, &keep_edges);
    retain_edge_data(&mut projected.edge_label_percentage_presence, &keep_edges);
    retain_edge_data(&mut projected.edge_font_sizes, &keep_edges);
    retain_edge_data(&mut projected.edge_label_bold, &keep_edges);
    retain_edge_data(&mut projected.edge_arrowhead_labels, &keep_edges);
    retain_edge_data(&mut projected.edge_styles, &keep_edges);
    retain_edge_data(&mut projected.edge_links, &keep_edges);
    retain_edge_data(&mut projected.edge_tooltips, &keep_edges);
    retain_edge_data(&mut projected.edge_icons, &keep_edges);
    retain_edge_data(&mut projected.edge_icon_positions, &keep_edges);
    retain_edge_data(&mut projected.edge_icon_border_radii, &keep_edges);
    retain_edge_data(&mut projected.edge_is_curves, &keep_edges);
    retain_edge_data(&mut projected.edge_table_columns, &keep_edges);
    retain_edge_data(&mut projected.edge_routes, &keep_edges);
    retain_edge_data(&mut projected.d2_input_routes, &keep_edges);
    retain_edge_data(&mut projected.d2_edge_validation, &keep_edges);
    retain_edge_data(&mut projected.edge_min_widths, &keep_edges);
    retain_edge_data(&mut projected.edge_min_heights, &keep_edges);
    // LayoutNested removes extracted sequence descendants from the parent
    // graph entirely. Keeping sparse source node IDs in the Dagre projection
    // made trace aliases skip those extracted objects (for example
    // object:0, object:2) even though their ranks were correctly omitted.
    let visible_nodes = projected
        .nodes()
        .filter_map(|(id, _)| (!extracted_ancestor(id)).then_some(id))
        .collect::<Vec<_>>();
    let (projected, node_map, compacted_edges) =
        super::project_graph(&projected, &visible_nodes, false);
    let edge_map = compacted_edges
        .iter()
        .filter_map(|id| edge_map.get(id.0 as usize).copied())
        .collect();
    Ok(Some(DagreSequenceProjection {
        graph: projected,
        locals,
        node_map,
        edge_map,
    }))
}

/// d2layouts.collectNestedExtractions stops descending at a grid diagram.
/// Its recursive LayoutNested call owns sequence extraction and reinjection
/// below that grid before restoring the grid's crossing edges.
fn sequence_belongs_to_grid_extraction(graph: &Graph, id: NodeId) -> bool {
    if !graph.root_sequence_diagram
        && (graph.root_grid_rows.is_some() || graph.root_grid_columns.is_some())
    {
        return true;
    }
    let mut parent = graph.node(id).and_then(|node| node.parent);
    while let Some(candidate) = parent {
        let Some(node) = graph.node(candidate) else {
            break;
        };
        if !graph.is_sequence_diagram(candidate)
            && (node.grid_rows.is_some() || node.grid_columns.is_some())
        {
            return true;
        }
        parent = node.parent;
    }
    false
}

fn retain_edge_data<T: Clone>(values: &mut Vec<T>, keep: &[usize]) {
    let original = values.clone();
    *values = keep
        .iter()
        .filter_map(|index| original.get(*index).cloned())
        .collect();
}

fn adjust_group_label(
    graph: &Graph,
    group: NodeId,
    groups: &[NodeId],
    spans: &[NodeId],
    notes: &[NodeId],
    boxes: &mut BTreeMap<NodeId, Rect>,
    routes: &mut BTreeMap<EdgeId, Vec<Point>>,
) {
    // HasLabel checks content, not cached dimensions. Direct callers can
    // clear a label while retaining its old measured box.
    if graph.node_labels.get(&group).is_some_and(String::is_empty) {
        return;
    }
    let Some(label_height) = graph
        .node(group)
        .and_then(|node| node.label_size)
        .map(|size| size.height)
    else {
        return;
    };
    let height_add = label_height + EDGE_GROUP_LABEL_PADDING / 2.0;
    if height_add < GROUP_CONTAINER_PADDING {
        return;
    }
    let Some(group_top) = boxes.get(&group).map(|rect| rect.origin.y) else {
        return;
    };
    if let Some(group_box) = boxes.get_mut(&group) {
        group_box.size.height += height_add;
    }
    // Source extends every group/span crossing this title band before
    // moving later objects. Descendants alone are not the contract.
    for id in groups.iter().chain(spans) {
        if let Some(rect) = boxes.get_mut(id)
            && rect.origin.y < group_top
            && rect.bottom() > group_top
        {
            rect.size.height += height_add;
        }
    }
    for route in routes.values_mut() {
        if route
            .first()
            .zip(route.last())
            .is_some_and(|(first, last)| first.y.min(last.y) > group_top)
        {
            for point in route {
                point.y += height_add;
            }
        }
    }
    for id in spans.iter().chain(groups).chain(notes) {
        if let Some(rect) = boxes.get_mut(id)
            && rect.origin.y > group_top
        {
            rect.origin.y += height_add;
        }
    }
}

fn layout_inner(
    graph: &Graph,
    options: &LayoutOptions,
    root_node: Option<NodeId>,
    engine: EngineKind,
    layout_nested_actors: bool,
    cancellation: Option<&CancellationToken>,
) -> Result<LayoutResult, LayoutError> {
    validate(graph)?;
    if layout_nested_actors {
        // LayoutNested walks through ordinary actors to extract special
        // descendants before the enclosing sequence partitions notes/spans.
        // Sequence roots remain as placeholders after their contents are
        // extracted, so the outer sequence still validates them as actors.
        // Direct grid actors already have their own extraction below.
        let mut queue = graph
            .nodes()
            .filter_map(|(id, node)| (node.parent == root_node).then_some(id))
            .collect::<Vec<_>>();
        let mut roots = Vec::new();
        let mut cursor = 0;
        while let Some(&id) = queue.get(cursor) {
            cursor += 1;
            let node = graph.node(id).expect("sequence descendant");
            if graph.is_sequence_diagram(id) {
                if has_direct_child(graph, id) {
                    roots.push(id);
                }
                continue;
            }
            if node.grid_rows.is_some() || node.grid_columns.is_some() {
                if node.parent != root_node && has_direct_child(graph, id) {
                    roots.push(id);
                }
                continue;
            }
            queue.extend(
                graph
                    .nodes()
                    .filter_map(|(child, node)| (node.parent == Some(id)).then_some(child)),
            );
        }
        if !roots.is_empty() {
            let locals =
                layout_actor_subgraphs(graph, options, &roots, engine.clone(), cancellation)?;
            let mut source = graph.clone();
            for (&id, local) in &locals {
                if let Some(rect) = local.boxes.get(&id) {
                    let node = source.node_mut(id).expect("extracted special root");
                    node.size = rect.size;
                    node.declared_size = Some(rect.size);
                }
            }
            let visible = graph
                .nodes()
                .filter_map(|(id, _)| {
                    (!roots
                        .iter()
                        .any(|root| id != *root && is_descendant_or_self(graph, id, Some(*root))))
                    .then_some(id)
                })
                .collect::<Vec<_>>();
            let (projected, nodes, edges) = super::project_graph(&source, &visible, false);
            let projected_root = root_node.map(|root| {
                NodeId(
                    nodes
                        .iter()
                        .position(|id| *id == root)
                        .expect("sequence root remains visible") as u32,
                )
            });
            let local = layout_inner(
                &projected,
                options,
                projected_root,
                engine.clone(),
                true,
                cancellation,
            )?;
            let mut result = super::remap_result(local, &nodes, &edges);
            result = apply_projection_locals(graph, result, locals);
            super::reroute_sequence_actor_crossings(
                graph,
                &mut result,
                root_node,
                &roots,
                options,
                engine,
                cancellation,
            )
            .map_err(super::LayoutDispatchError::into_core_error)?;
            return Ok(result);
        }
    }
    // Sequence layout does not run a seed race. Nested ordinary actor graphs
    // pass the original options and graph data to their actual core engine.
    let seed = graph
        .tala_seed_data
        .as_ref()
        .and_then(|data| data.as_ref().ok())
        .and_then(|seeds| seeds.first())
        .or_else(|| options.seeds.first())
        .copied()
        .unwrap_or(1);
    // A source diagram can mark the synthetic document root itself as a
    // sequence diagram. Keep `None` as that root so its actors are the graph's
    // root nodes and no artificial box is published.
    if root_node.is_none() && !graph.root_sequence_diagram {
        return Err(LayoutError::IdOverflow);
    }
    let all_scope_nodes = graph
        .nodes()
        .filter_map(|(id, _)| is_descendant_or_self(graph, id, root_node).then_some(id))
        .collect::<Vec<_>>();
    let mut groups = Vec::new();
    for (id, node) in graph.nodes() {
        if node.parent == root_node && is_sequence_group(graph, id, root_node) {
            groups.push(id);
            groups.extend(super::ordered_descendants(graph, id));
        }
    }
    groups.sort();
    groups.dedup();
    let mut actors = graph
        .nodes()
        .filter_map(|(id, node)| (node.parent == root_node && !groups.contains(&id)).then_some(id))
        .collect::<Vec<_>>();
    // newSequenceDiagram sorts objects by their earliest non-glob MapKey.
    // An actor created only through a glob sorts after declared actors,
    // regardless of its position in Graph.Objects or ChildrenArray.
    d2_rs_core::go_sort::sort_by(&mut actors, |left, right| {
        note_line(graph, *left) < note_line(graph, *right)
    });
    // LayoutNested retains special diagram roots, even after extracting their
    // contents. They remain in ChildrenArray, where newSequenceDiagram rejects
    // nested sequence actors in declaration order, before the empty-actor check.
    if let Some(actor) = actors
        .iter()
        .copied()
        .filter(|id| graph.is_sequence_diagram(*id))
        .min_by_key(|id| note_line(graph, *id))
    {
        let node = graph.node(actor).expect("sequence actor");
        let name = root_node
            .and_then(|root| graph.node(root))
            .and_then(|root| node.external_id.strip_prefix(&root.external_id))
            .and_then(|suffix| suffix.strip_prefix('.'))
            .unwrap_or(&node.external_id);
        return Err(LayoutError::SequenceDiagram(format!(
            "actors in sequence diagrams cannot themselves be sequence diagrams: {name}"
        )));
    }
    if actors.is_empty() {
        return Err(LayoutError::SequenceDiagram(
            "no actors declared in sequence diagram".to_owned(),
        ));
    }
    let actor_layouts = if layout_nested_actors {
        layout_actor_subgraphs(graph, options, &actors, engine.clone(), cancellation)?
    } else {
        BTreeMap::new()
    };
    let extracted_actor_nodes = actor_layouts
        .keys()
        .flat_map(|actor| super::ordered_descendants(graph, *actor))
        .collect::<std::collections::BTreeSet<_>>();
    // LayoutNested removes special diagram contents, retaining their roots.
    // Those placeholders (and unextracted empty diagrams) still participate
    // in newSequenceDiagram's note/span classification under ordinary actors.
    let scope_nodes = all_scope_nodes
        .into_iter()
        .filter(|id| !extracted_actor_nodes.contains(id))
        .collect::<Vec<_>>();
    let notes = scope_nodes
        .iter()
        .copied()
        .filter(|id| is_sequence_note(graph, *id, root_node))
        .collect::<Vec<_>>();
    let actor_sizes = actors
        .iter()
        .map(|id| {
            let size = actor_layouts
                .get(id)
                .and_then(|layout| layout.boxes.get(id))
                .map_or_else(
                    || graph.node(*id).expect("actor exists").size,
                    |rect| rect.size,
                );
            (*id, size)
        })
        .collect::<BTreeMap<_, _>>();
    let mut boxes = BTreeMap::new();
    let root_origin = root_node
        .and_then(|root| graph.node(root))
        .and_then(|node| node.locked_position)
        .unwrap_or_default();
    // d2sequence lays actors out against a shared bottom baseline. The
    // sequence container contributes its 12px inset and a root title reserves
    // its measured height before the actor/message area begins.
    let root_label_height = root_node
        .and_then(|root| graph.node(root))
        .and_then(|node| node.label_size)
        .filter(|size| size.width > 0.0 && size.height > 0.0)
        .map_or(0.0, |size| size.height);
    let mut actor_center_x = root_origin.x + GROUP_CONTAINER_PADDING;
    let mut actor_centers = BTreeMap::new();
    let mut max_actor_height: f64 = 0.0;
    let mut actor_widths = BTreeMap::new();
    for actor_id in actors.iter().copied() {
        let actor = graph.node(actor_id).expect("actor exists");
        let size = actor_sizes[&actor_id];
        let mut width = size.width;
        let mut height = size.height;
        if width < MIN_ACTOR_WIDTH {
            if matches!(
                actor.shape,
                d2_rs_core::ShapeKind::Person
                    | d2_rs_core::ShapeKind::Oval
                    | d2_rs_core::ShapeKind::Square
                    | d2_rs_core::ShapeKind::Circle
            ) {
                let factor = MIN_ACTOR_WIDTH / width.max(1.0);
                height *= factor;
            }
            width = MIN_ACTOR_WIDTH;
        }
        actor_widths.insert(actor_id, width);
        max_actor_height = max_actor_height.max(height);
    }
    let mut actor_steps = Vec::new();
    for pair in actors.windows(2) {
        let width = actor_widths[&pair[0]];
        // newSequenceDiagram computes each gap while walking actors: the
        // current actor has been widened to MIN_ACTOR_WIDTH, but the next
        // actor still has its measured width at this point in the Go loop.
        let next_width = actor_sizes[&pair[1]].width;
        actor_steps.push((width / 2.0 + next_width / 2.0 + HORIZONTAL_PAD).max(MIN_ACTOR_DISTANCE));
    }
    for note_id in &notes {
        let note = graph.node(*note_id).expect("note exists");
        if let Some(actor) = direct_actor(graph, *note_id, root_node)
            && let Some(rank) = actors.iter().position(|id| *id == actor)
        {
            // Upstream handles both adjacent gaps inside its non-last-actor
            // branch. A note on the final actor does not enlarge either gap.
            if rank < actor_steps.len() {
                actor_steps[rank] = actor_steps[rank].max(note.size.width / 2.0 + HORIZONTAL_PAD);
                if rank > 0 {
                    actor_steps[rank - 1] =
                        actor_steps[rank - 1].max(note.size.width / 2.0 + HORIZONTAL_PAD);
                }
            }
        }
    }
    for (edge_id, edge) in graph.edges() {
        if !edge_belongs_to_sequence_scope(graph, edge, root_node)
            || edge_removed_with_extracted_actor(graph, edge, &actor_layouts)
        {
            continue;
        }
        let label_width = graph
            .edge_label(edge_id)
            .map_or(0.0, |label| label.size.width);
        let Some(source_actor) = direct_actor(graph, edge.source, root_node) else {
            continue;
        };
        let Some(target_actor) = direct_actor(graph, edge.target, root_node) else {
            continue;
        };
        let Some(source_rank) = actors.iter().position(|id| *id == source_actor) else {
            continue;
        };
        let Some(target_rank) = actors.iter().position(|id| *id == target_actor) else {
            continue;
        };
        if source_rank != target_rank {
            let per_rank = label_width / (source_rank.abs_diff(target_rank) as f64).max(1.0);
            let first = source_rank.min(target_rank);
            let last = source_rank.max(target_rank).min(actor_steps.len());
            for step in &mut actor_steps[first..last] {
                *step = (*step).max(per_rank + LABEL_HORIZONTAL_PAD);
            }
        } else if let Some(step) = actor_steps.get_mut(source_rank) {
            // Same-actor messages reserve their label width plus four times
            // lib/label.PADDING (5px) in the gap to the next actor.
            *step = (*step).max(label_width + 20.0);
        }
    }
    for (index, actor_id) in actors.iter().copied().enumerate() {
        let actor = graph.node(actor_id).expect("actor exists");
        let actor_size = actor_sizes[&actor_id];
        let width = actor_widths[&actor_id];
        let height = if actor_size.width < MIN_ACTOR_WIDTH
            && matches!(
                actor.shape,
                d2_rs_core::ShapeKind::Person
                    | d2_rs_core::ShapeKind::Oval
                    | d2_rs_core::ShapeKind::Square
                    | d2_rs_core::ShapeKind::Circle
            ) {
            actor_size.height * (MIN_ACTOR_WIDTH / actor_size.width.max(1.0))
        } else {
            actor_size.height
        };
        if index == 0 {
            actor_center_x += width / 2.0;
        } else {
            actor_center_x += actor_steps[index - 1];
        }
        let center_x = actor_center_x;
        let origin = actor.locked_position.unwrap_or(Point {
            // Go's sequence adapter rounds only the horizontal top-left;
            // preserving that detail keeps half-pixel actor widths stable.
            x: (center_x - width / 2.0).round(),
            // Go places actors locally before sd.shift adds the container
            // inset, after the height and label subtractions.
            y: max_actor_height
                + VERTICAL_PADDING
                + root_label_height
                - height
                // Source placeActors aligns the label below image/person
                // actors to the common baseline, subtracting its measured
                // height even when an explicit icon position is retained.
                - if matches!(actor.shape, d2_rs_core::ShapeKind::Image | d2_rs_core::ShapeKind::Person)
                    && graph.node_labels.get(&actor_id).is_none_or(|label| !label.is_empty())
                { actor.label_size.map_or(0.0, |size| size.height) } else { 0.0 }
                + (root_origin.y + GROUP_CONTAINER_PADDING),
        });
        let rect = Rect {
            origin,
            size: Size { width, height },
        };
        actor_centers.insert(actor_id, rect.center());
        boxes.insert(actor_id, rect);
        max_actor_height = max_actor_height.max(height);
    }

    let mut nested_boxes = Vec::new();
    let mut nested_routes = BTreeMap::new();
    for (actor, local) in &actor_layouts {
        let (Some(local_actor), Some(actor_rect)) = (local.boxes.get(actor), boxes.get(actor))
        else {
            continue;
        };
        let delta = Point {
            x: actor_rect.origin.x - local_actor.origin.x,
            y: actor_rect.origin.y - local_actor.origin.y,
        };
        for (id, rect) in &local.boxes {
            if *id == *actor || !is_descendant_or_self(graph, *id, Some(*actor)) {
                continue;
            }
            let mut rect = *rect;
            rect.origin.x += delta.x;
            rect.origin.y += delta.y;
            nested_boxes.push((*id, rect));
        }
        for (edge, route) in &local.routes {
            nested_routes.insert(
                *edge,
                route
                    .iter()
                    .map(|point| Point {
                        x: point.x + delta.x,
                        y: point.y + delta.y,
                    })
                    .collect(),
            );
        }
    }

    let message_y = root_origin.y
        + GROUP_CONTAINER_PADDING
        + VERTICAL_PADDING
        + root_label_height
        + max_actor_height
        + message_step();
    let mut routes = BTreeMap::new();
    let mut message_rows = BTreeMap::new();
    let mut message_offset = message_y;
    let mut message_edges = graph
        .edges()
        .filter(|(_, edge)| {
            edge_belongs_to_sequence_scope(graph, edge, root_node)
                && !edge_removed_with_extracted_actor(graph, edge, &actor_layouts)
        })
        .collect::<Vec<_>>();
    d2_rs_core::go_sort::sort_by(&mut message_edges, |left, right| {
        edge_line(graph, left.0) < edge_line(graph, right.0)
    });
    // d2sequence.addLifelineEdges derives the common endpoint from the last
    // message in this source-order list, not from every group or route bound.
    // Keep the identity before consuming the list below so the sequence root
    // can mirror getHeight(), which returns that endpoint verbatim.
    let last_message_id = message_edges.last().map(|(id, _)| *id);
    let mut first_messages = BTreeMap::new();
    let mut last_messages = BTreeMap::new();
    for (edge_id, edge) in &message_edges {
        for endpoint in [edge.source, edge.target] {
            first_messages.entry(endpoint).or_insert(*edge_id);
            last_messages.insert(endpoint, *edge_id);
        }
    }
    for (edge_id, edge) in message_edges {
        let Some(source_actor) = direct_actor(graph, edge.source, root_node) else {
            continue;
        };
        let Some(target_actor) = direct_actor(graph, edge.target, root_node) else {
            continue;
        };
        // routeMessages: getCenter walks parents to the first positioned
        // object. An endpoint below no actor (an undeclared name inside a
        // group) reaches the unpositioned root.
        for (endpoint, actor) in [(edge.source, source_actor), (edge.target, target_actor)] {
            if !actor_centers.contains_key(&actor) {
                let local = graph.node(endpoint).map_or("", |node| {
                    let parent = node.parent.and_then(|id| graph.node(id));
                    parent
                        .and_then(|parent| node.external_id.strip_prefix(&parent.external_id))
                        .and_then(|rest| rest.strip_prefix('.'))
                        .unwrap_or(&node.external_id)
                });
                return Err(LayoutError::SequenceDiagram(format!(
                    "could not find center of {local}. Is it declared as an actor?"
                )));
            }
        }
        let source = boxes
            .get(&edge.source)
            .copied()
            .map_or(actor_centers[&source_actor], |rect| rect.center());
        let target = boxes
            .get(&edge.target)
            .copied()
            .map_or(actor_centers[&target_actor], |rect| rect.center());
        let label_size = graph.edge_label(edge_id).map(|label| label.size);
        let label_width = label_size.map_or(0.0, |size| size.width);
        let label_height = label_size.map_or(0.0, |size| size.height);
        let sibling_scope = source_actor == target_actor;
        let self_message = edge.source == edge.target;
        let note_offset = notes
            .iter()
            .filter(|note| note_line(graph, **note) < edge_line(graph, edge_id))
            .map(|note| graph.node(*note).map_or(0.0, |node| node.size.height) + message_step())
            .sum::<f64>();
        let start_y = message_offset
            + note_offset
            + if self_message || sibling_scope {
                0.0
            } else {
                (label_height / 2.0).floor()
            };
        let route = if self_message || sibling_scope {
            let mid_x = source.x + SELF_MESSAGE_HORIZONTAL_TRAVEL.max(label_width / 2.0 + 10.0);
            let end_y = start_y + label_height.max(MIN_MESSAGE_DISTANCE) * 1.5;
            vec![
                Point {
                    x: source.x,
                    y: start_y,
                },
                Point {
                    x: mid_x,
                    y: start_y,
                },
                Point { x: mid_x, y: end_y },
                Point {
                    x: target.x,
                    y: end_y,
                },
            ]
        } else {
            vec![
                Point {
                    x: source.x,
                    y: start_y,
                },
                Point {
                    x: target.x,
                    y: start_y,
                },
            ]
        };
        message_rows.insert(edge_id, start_y);
        routes.insert(edge_id, route);
        let tail_y = routes[&edge_id].last().map_or(start_y, |point| point.y);
        let ordinary_label_half = if self_message || sibling_scope {
            0.0
        } else {
            (label_height / 2.0).floor()
        };
        message_offset = tail_y + ordinary_label_half + message_step() - note_offset;
    }

    // Notes are anchored to their actor rank and stacked in source order. This
    // mirrors D2's vertical index without depending on pointer identity.
    for note_id in notes.iter().copied() {
        let Some(actor) = direct_actor(graph, note_id, root_node) else {
            continue;
        };
        let Some(center) = actor_centers.get(&actor).copied() else {
            continue;
        };
        let note_index = note_line(graph, note_id);
        let mut y = message_y;
        for edge_id in message_rows.keys() {
            if edge_line(graph, *edge_id) < note_index {
                let edge = &graph.edges[edge_id.0 as usize];
                let label_height = graph
                    .edge_label(*edge_id)
                    .map_or(0.0, |label| label.size.height);
                y += message_step()
                    + if edge.source == edge.target {
                        label_height.max(MIN_MESSAGE_DISTANCE) * 1.5
                    } else {
                        label_height
                    };
            }
        }
        for prior in notes
            .iter()
            .copied()
            .filter(|id| note_line(graph, *id) < note_index)
        {
            y += graph.node(prior).map_or(0.0, |node| node.size.height) + message_step();
        }
        let node = graph.node(note_id).expect("note exists");
        boxes.insert(
            note_id,
            Rect {
                origin: Point {
                    x: center.x - node.size.width / 2.0,
                    y,
                },
                size: node.size,
            },
        );
    }

    // Place nested spans after routes are available, deepest first so parents
    // can encompass their already-positioned children.
    let mut spans = scope_nodes
        .iter()
        .copied()
        .filter(|id| {
            Some(*id) != root_node
                && !actors.contains(id)
                && !groups.contains(id)
                && !notes.contains(id)
                && !is_group_member_endpoint(graph, *id, root_node)
                && direct_actor(graph, *id, root_node).is_some()
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|id| std::cmp::Reverse(depth(graph, *id)));
    for span in spans.iter().copied() {
        let Some(actor) = direct_actor(graph, span, root_node) else {
            continue;
        };
        let center_x = actor_centers.get(&actor).map_or(0.0, |center| center.x);
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        if let Some(edge_id) = first_messages.get(&span)
            && let Some(route) = routes.get(edge_id)
        {
            let edge = &graph.edges[edge_id.0 as usize];
            min_y = if edge.source == edge.target || edge.source == span {
                route.first().map_or(f64::INFINITY, |p| p.y)
            } else {
                route.last().map_or(f64::INFINITY, |p| p.y)
            };
        }
        if let Some(edge_id) = last_messages.get(&span)
            && let Some(route) = routes.get(edge_id)
        {
            let edge = &graph.edges[edge_id.0 as usize];
            max_y = if edge.source == edge.target || edge.target == span {
                route.last().map_or(f64::NEG_INFINITY, |p| p.y)
            } else {
                route.first().map_or(f64::NEG_INFINITY, |p| p.y)
            };
        }
        for (id, rect) in boxes.iter() {
            if graph
                .node(*id)
                .is_some_and(|node| node.parent == Some(span))
            {
                min_y = if min_y.is_nan() || rect.origin.y.is_nan() {
                    f64::NAN
                } else {
                    min_y.min(rect.origin.y)
                };
                max_y = if max_y.is_nan() || rect.bottom().is_nan() {
                    f64::NAN
                } else {
                    max_y.max(rect.bottom())
                };
            }
        }
        let top = min_y - SPAN_MESSAGE_PAD;
        let height = (max_y - top + SPAN_MESSAGE_PAD).max(MIN_SPAN_HEIGHT);
        // Source span.Level()-sd.root.Level()-2 counts the document root.
        // Graph omits that root, so a top-level actor has depth zero.
        let first_span_depth = root_node.map_or(1, |root| depth(graph, root) + 2);
        let width = SPAN_BASE_WIDTH
            + depth(graph, span).saturating_sub(first_span_depth) as f64 * SPAN_DEPTH_GROWTH_FACTOR;
        boxes.insert(
            span,
            Rect {
                origin: Point {
                    x: center_x - width / 2.0,
                    y: top,
                },
                size: Size { width, height },
            },
        );
    }

    // Adjust route endpoints from actor centers to span borders.
    for (edge_id, edge) in graph.edges() {
        if edge_removed_with_extracted_actor(graph, edge, &actor_layouts) {
            continue;
        }
        let Some(route) = routes.get_mut(&edge_id) else {
            continue;
        };
        let source_rank = direct_actor(graph, edge.source, root_node)
            .and_then(|actor| actors.iter().position(|id| *id == actor));
        let target_rank = direct_actor(graph, edge.target, root_node)
            .and_then(|actor| actors.iter().position(|id| *id == actor));
        if let Some(rect) = boxes.get(&edge.source).copied()
            && edge.source != direct_actor(graph, edge.source, root_node).unwrap_or(edge.source)
            && let Some(point) = route.first_mut()
        {
            point.x += if source_rank <= target_rank {
                rect.size.width / 2.0
            } else {
                -rect.size.width / 2.0
            };
        }
        if let Some(rect) = boxes.get(&edge.target).copied()
            && edge.target != direct_actor(graph, edge.target, root_node).unwrap_or(edge.target)
            && let Some(point) = route.last_mut()
        {
            point.x += if source_rank < target_rank {
                -rect.size.width / 2.0
            } else {
                rect.size.width / 2.0
            };
        }
    }

    // d2sequence.placeGroup bounds owned messages/non-glob notes and direct
    // child groups. Empty groups retain unbounded boxes until final validation.
    let mut groups_by_depth = groups.clone();
    groups_by_depth.sort_by_key(|id| std::cmp::Reverse(depth(graph, *id)));
    for group in groups_by_depth.iter().copied() {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (edge_id, edge) in graph.edges() {
            if !edge_removed_with_extracted_actor(graph, edge, &actor_layouts)
                && edge_belongs_to_group(graph, edge_id, edge, group)
                && let Some(route) = routes.get(&edge_id)
            {
                for point in route {
                    min_x = min_x.min(point.x - HORIZONTAL_PAD);
                    max_x = max_x.max(point.x + HORIZONTAL_PAD);
                    let label_height = graph
                        .edge_label(edge_id)
                        .map_or(0.0, |label| label.size.height / 2.0);
                    let edge_pad =
                        (label_height + GROUP_CONTAINER_PADDING).max(MIN_MESSAGE_DISTANCE / 2.0);
                    min_y = min_y.min(point.y - edge_pad);
                    max_y = max_y.max(point.y + edge_pad);
                }
            }
        }
        for note in &notes {
            let owned = if graph.sequence_note_scopes.is_empty() {
                is_descendant_or_self(graph, *note, Some(group))
            } else {
                graph.sequence_note_scopes.get(note).is_some_and(|scopes| {
                    scopes
                        .iter()
                        .any(|scope| is_descendant_or_self(graph, *scope, Some(group)))
                })
            };
            if owned && let Some(rect) = boxes.get(note) {
                min_x = min_x.min(rect.origin.x - HORIZONTAL_PAD);
                min_y = min_y.min(rect.origin.y - MIN_MESSAGE_DISTANCE / 2.0);
                max_x = max_x.max(rect.right() + HORIZONTAL_PAD);
                max_y = max_y.max(rect.bottom() + MIN_MESSAGE_DISTANCE / 2.0);
            }
        }
        for child in &groups {
            if graph
                .node(*child)
                .is_some_and(|node| node.parent == Some(group))
                && let Some(rect) = boxes.get(child)
            {
                // Go math.Min/Max propagate NaN (Rust f64::min/max do not).
                let go_min = |a: f64, b: f64| {
                    if a.is_nan() || b.is_nan() {
                        f64::NAN
                    } else {
                        a.min(b)
                    }
                };
                let go_max = |a: f64, b: f64| {
                    if a.is_nan() || b.is_nan() {
                        f64::NAN
                    } else {
                        a.max(b)
                    }
                };
                min_x = go_min(min_x, rect.origin.x - GROUP_CONTAINER_PADDING);
                min_y = go_min(min_y, rect.origin.y - GROUP_CONTAINER_PADDING);
                max_x = go_max(max_x, rect.right() + GROUP_CONTAINER_PADDING);
                max_y = go_max(max_y, rect.bottom() + GROUP_CONTAINER_PADDING);
            }
        }
        boxes.insert(
            group,
            Rect {
                origin: Point { x: min_x, y: min_y },
                size: Size {
                    width: max_x - min_x,
                    height: max_y - min_y,
                },
            },
        );
    }

    // A labeled group reserves a title band at its top and pushes later
    // messages/children down, matching d2sequence.adjustGroupLabel.
    for group in groups_by_depth.iter().copied() {
        adjust_group_label(
            graph,
            group,
            &groups,
            &spans,
            &notes,
            &mut boxes,
            &mut routes,
        );
    }

    // d2sequence.getWidth starts at the final actor, includes every message
    // route, and additionally includes the right half of a self-message label
    // centered on the outer segment of its elbow. Coordinates already carry
    // the sequence's left padding; the root calculation below adds the right.
    let mut content_right = actors
        .last()
        .and_then(|id| boxes.get(id))
        .map_or(root_origin.x, |rect| rect.right());
    for (edge_id, route) in &routes {
        for point in route {
            content_right = content_right.max(point.x);
        }
        if graph
            .edges
            .get(edge_id.0 as usize)
            .is_some_and(|edge| edge.source == edge.target)
            && let Some(elbow) = route.get(1)
        {
            let label_width = graph
                .edge_label(*edge_id)
                .map_or(0.0, |label| label.size.width);
            content_right = content_right.max(elbow.x + label_width / 2.0);
        }
    }
    let lifeline_end = last_message_id
        .and_then(|id| routes.get(&id))
        .into_iter()
        .flatten()
        .map(|point| point.y)
        .chain(
            notes
                .iter()
                .filter_map(|id| boxes.get(id).map(|rect| rect.bottom())),
        )
        .chain(
            actors
                .iter()
                .filter_map(|id| boxes.get(id).map(|rect| rect.bottom())),
        )
        .fold(root_origin.y, f64::max)
        + message_step();
    if let Some(root) = root_node {
        boxes.insert(
            root,
            Rect {
                origin: root_origin,
                size: Size {
                    width: (content_right - root_origin.x + GROUP_CONTAINER_PADDING).max(1.0),
                    height: (lifeline_end - root_origin.y + GROUP_CONTAINER_PADDING).max(1.0),
                },
            },
        );
    }
    for (id, rect) in nested_boxes {
        boxes.insert(id, rect);
    }
    routes.extend(nested_routes);
    let mut node_labels: BTreeMap<_, _> = graph
        .nodes()
        .map(|(id, node)| {
            (
                id,
                NodeLabelState {
                    font_size_committed: false,
                    size: node.label_size,
                    font_size: node.font_size,
                    position: if Some(id) == root_node {
                        // `d2sequence.Layout` marks its graph root label as
                        // inside-top-center before the sequence graph is
                        // injected into an enclosing grid. Keeping the label
                        // inside avoids treating a nested sequence cell like
                        // an ordinary outside-labeled container.
                        LabelPosition::InsideTopCenter
                    } else if groups.contains(&id) {
                        LabelPosition::InsideTopLeft
                    } else if notes.contains(&id) {
                        LabelPosition::InsideMiddleCenter
                    } else if actors.contains(&id) {
                        // placeActors defaults a missing ordinary icon label
                        // independently from the icon's own position.
                        if matches!(
                            node.shape,
                            d2_rs_core::ShapeKind::Image | d2_rs_core::ShapeKind::Person
                        ) {
                            if node.icon_position.is_none() {
                                LabelPosition::OutsideBottomCenter
                            } else {
                                node.label_position
                            }
                        } else if node.has_icon {
                            if node.label_position == LabelPosition::Unset {
                                LabelPosition::OutsideTopCenter
                            } else {
                                node.label_position
                            }
                        } else if node.icon_position.is_none() {
                            LabelPosition::InsideMiddleCenter
                        } else {
                            node.label_position
                        }
                    } else {
                        node.label_position
                    },
                    icon_position: if actors.contains(&id)
                        && node.has_icon
                        && !matches!(
                            node.shape,
                            d2_rs_core::ShapeKind::Image | d2_rs_core::ShapeKind::Person
                        )
                        && node.icon_position.is_none()
                    {
                        Some(LabelPosition::InsideMiddleCenter)
                    } else {
                        node.icon_position
                    },
                },
            )
        })
        .collect();
    // addLifelineEdges dereferences LabelPosition even for an empty label.
    // placeActors intentionally leaves it nil when an authored icon position
    // suppresses the default. Fail from the actual resulting state, before
    // publishing lifelines, rather than silently accepting that source fault.
    for actor in &actors {
        if node_labels
            .get(actor)
            .map(|state| state.position)
            .unwrap_or(LabelPosition::Unset)
            == LabelPosition::Unset
        {
            return Err(LayoutError::D2NilPointerFault {
                source_site: "d2sequence.sequenceDiagram.addLifelineEdges:425",
            });
        }
    }
    // Lifelines belong to the parent sequence invocation, before
    // LayoutNested injects the extracted grid root's non-nil attributes.
    let lifeline_labels = actors
        .iter()
        .filter_map(|id| node_labels.get(id).copied().map(|state| (*id, state)))
        .collect::<BTreeMap<_, _>>();
    // Reinject label state along with geometry from extracted actor scopes.
    for (actor, local) in &actor_layouts {
        for (id, label) in &local.node_labels {
            if is_descendant_or_self(graph, *id, Some(*actor)) {
                let mut restored = *label;
                if id == actor
                    && let Some(parent_state) = node_labels.get(id)
                {
                    // InjectNested(isRoot=true) only copies non-nil position
                    // pointers. A nil extracted root must not erase the
                    // position placeActors just assigned in the parent.
                    if restored.position == LabelPosition::Unset {
                        restored.position = parent_state.position;
                    }
                    if restored.icon_position.is_none() {
                        restored.icon_position = parent_state.icon_position;
                    }
                }
                node_labels.insert(*id, restored);
            }
        }
    }
    // newSequenceDiagram clears span Label.Value before reinjection. These
    // measurements must not become invisible obstacles in parent routing.
    for span in &spans {
        if let Some(label) = node_labels.get_mut(span) {
            label.size = None;
            label.position = LabelPosition::Unset;
        }
    }
    let edge_labels = graph
        .edges()
        .filter_map(|(id, edge)| {
            let mut label = graph.edge_label(id)?.clone();
            // d2sequence.placeMessages commits centered label state on its
            // own messages; extracted actor graphs retain their engine state.
            if !label.text.is_empty()
                && edge_belongs_to_sequence_scope(graph, edge, root_node)
                && !edge_removed_with_extracted_actor(graph, edge, &actor_layouts)
            {
                label.position = LabelPosition::InsideMiddleCenter;
            }
            Some((id, label))
        })
        .collect();
    let mut result = LayoutResult {
        removed_edges: actor_layouts
            .values()
            .flat_map(|local| local.removed_edges.iter().copied())
            .collect(),
        source_edge_storage: Default::default(),
        synthetic_root: None,
        route_curves: Default::default(),
        sequence_lifelines: Vec::new(),
        edge_label_percentage_commits: Default::default(),
        boxes,
        routes,
        edge_labels,
        node_labels,
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
            warnings: vec!["Sequence layout uses the in-process d2sequence adapter.".to_owned()],
        },
    };
    if root_node.is_none() {
        result.synthetic_root = Some(d2_rs_core::SyntheticRootState {
            top_left: Some(root_origin),
            size: Size {
                width: (content_right - root_origin.x + GROUP_CONTAINER_PADDING).max(1.0),
                height: (lifeline_end - root_origin.y + GROUP_CONTAINER_PADDING).max(1.0),
            },
            label_position: Some(LabelPosition::InsideTopCenter),
            icon_position: None,
        });
    }
    let extracted_messages = graph
        .edges()
        .filter_map(|(id, edge)| {
            edge_removed_with_extracted_actor(graph, edge, &actor_layouts).then_some(id)
        })
        .collect();
    result.sequence_lifelines =
        fixed_lifeline_routes_with_labels(graph, &result, &lifeline_labels, &extracted_messages)
            .into_iter()
            .filter(|(actor, _)| actors.contains(actor))
            .map(|(actor, route)| {
                let node = graph.node(actor).expect("actor exists");
                let parent = node.parent.and_then(|id| graph.node(id));
                let prefix = parent.map_or(String::new(), |p| format!("{}.", p.external_id));
                let local = node
                    .external_id
                    .strip_prefix(&prefix)
                    .unwrap_or(&node.external_id);
                let target = format!("{local}-lifeline-end");
                let hash = target.bytes().fold(2_166_136_261u32, |h, b| {
                    (h ^ u32::from(b)).wrapping_mul(16_777_619)
                });
                d2_rs_core::SequenceLifeline {
                    actor,
                    target_id: format!("{target}-{hash}"),
                    route,
                }
            })
            .collect();
    for (actor, local) in &actor_layouts {
        let (Some(before), Some(after)) = (local.boxes.get(actor), result.boxes.get(actor)) else {
            continue;
        };
        for mut line in local.sequence_lifelines.clone() {
            for point in &mut line.route {
                point.x += after.origin.x - before.origin.x;
                point.y += after.origin.y - before.origin.y;
            }
            result.sequence_lifelines.push(line);
        }
    }
    super::reroute_sequence_actor_crossings(
        graph,
        &mut result,
        root_node,
        &actor_layouts.keys().copied().collect::<Vec<_>>(),
        options,
        engine,
        cancellation,
    )
    .map_err(super::LayoutDispatchError::into_core_error)?;
    Ok(result)
}

/// Fixed lifeline topology imported by TALA's D2 adapter as self-loops.
/// The synthetic destination has no box and is not an obstacle node.
#[cfg(test)]
pub(super) fn fixed_lifeline_routes(
    graph: &Graph,
    result: &LayoutResult,
) -> Vec<(NodeId, Vec<Point>)> {
    fixed_lifeline_routes_with_labels(graph, result, &result.node_labels, &BTreeSet::new())
}

fn fixed_lifeline_routes_with_labels(
    graph: &Graph,
    result: &LayoutResult,
    label_states: &BTreeMap<NodeId, NodeLabelState>,
    extracted_messages: &BTreeSet<EdgeId>,
) -> Vec<(NodeId, Vec<Point>)> {
    let mut scopes = graph
        .nodes()
        .filter_map(|(id, _)| graph.is_sequence_diagram(id).then_some(Some(id)))
        .collect::<Vec<_>>();
    if graph.root_sequence_diagram {
        scopes.insert(0, None);
    }
    let mut output = Vec::new();
    for scope in scopes {
        let mut actors = graph
            .nodes()
            .filter_map(|(id, node)| {
                (node.parent == scope && !is_sequence_group(graph, id, scope)).then_some(id)
            })
            .collect::<Vec<_>>();
        // addLifelineEdges walks the same sorted actors as placeActors.
        d2_rs_core::go_sort::sort_by(&mut actors, |left, right| {
            note_line(graph, *left) < note_line(graph, *right)
        });
        let mut end_y = actors
            .iter()
            .filter_map(|id| result.boxes.get(id).map(|rect| rect.bottom()))
            .fold(f64::NEG_INFINITY, f64::max);
        let mut messages = graph
            .edges()
            .filter(|(id, edge)| {
                if extracted_messages.contains(id) {
                    return false;
                }
                if !edge_belongs_to_sequence_scope(graph, edge, scope) {
                    return false;
                }
                let a = direct_actor(graph, edge.source, scope);
                let b = direct_actor(graph, edge.target, scope);
                if a.is_none() || b.is_none() {
                    return false;
                }
                !(a == b
                    && a.is_some_and(|id| {
                        edge.source != id
                            && edge.target != id
                            && graph.node(id).is_some_and(|node| {
                                node.grid_rows.is_some() || node.grid_columns.is_some()
                            })
                    }))
            })
            .collect::<Vec<_>>();
        // Original addLifelineEdges uses the last message after the same
        // earliest eligible declaration-line sort as routeMessages.
        d2_rs_core::go_sort::sort_by(&mut messages, |left, right| {
            edge_line(graph, left.0) < edge_line(graph, right.0)
        });
        let last_message = messages.last().copied();
        if let Some((id, _)) = last_message {
            for point in result.routes.get(&id).into_iter().flatten() {
                end_y = end_y.max(point.y);
            }
        }
        for (id, _) in graph.nodes() {
            if sequence_owner(graph, id) == scope
                && is_sequence_note(graph, id, scope)
                && direct_actor(graph, id, scope).is_some_and(|actor| {
                    graph
                        .node(actor)
                        .is_some_and(|node| node.grid_rows.is_none() && node.grid_columns.is_none())
                })
                && let Some(rect) = result.boxes.get(&id)
            {
                end_y = end_y.max(rect.bottom());
            }
        }
        if !end_y.is_finite() {
            continue;
        }
        end_y += message_step();
        for actor in actors {
            let Some(rect) = result.boxes.get(&actor) else {
                continue;
            };
            let label_pad = label_states
                .get(&actor)
                .filter(|label| {
                    label.position == LabelPosition::OutsideBottomCenter
                        && graph
                            .node_label_bytes
                            .get(&actor)
                            .is_none_or(|text| !text.is_empty())
                })
                .and_then(|label| label.size)
                .map_or(0.0, |size| size.height + 5.0);
            output.push((
                actor,
                vec![
                    Point {
                        x: rect.center().x,
                        y: rect.bottom() + label_pad,
                    },
                    Point {
                        x: rect.center().x,
                        y: end_y,
                    },
                ],
            ));
        }
    }
    output
}

fn layout_actor_subgraphs(
    graph: &Graph,
    options: &LayoutOptions,
    actors: &[NodeId],
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
) -> Result<BTreeMap<NodeId, LayoutResult>, LayoutError> {
    let mut layouts = BTreeMap::new();
    for actor in actors.iter().copied() {
        let Some(actor_node) = graph.node(actor) else {
            continue;
        };
        // LayoutNested descends through ordinary sequence actors, but it only
        // extracts special diagrams. Running Dagre on an ordinary actor's
        // children changes the actor into a generic compound; its descendants
        // are sequence spans and notes instead.
        if actor_node.grid_rows.is_none()
            && actor_node.grid_columns.is_none()
            && !graph.is_sequence_diagram(actor)
        {
            continue;
        }
        if !graph.nodes().any(|(_, node)| node.parent == Some(actor)) {
            continue;
        }
        let mut node_ids = vec![actor];
        node_ids.extend(super::ordered_descendants(graph, actor));
        let (mut projected, projected_nodes, mut projected_edges) =
            super::project_graph(graph, &node_ids, false);
        // LayoutNested gives an extracted special diagram its own graph root.
        // It does not inherit the enclosing document's sequence root shape.
        // Keeping that flag recursively dispatches the same actor as another
        // root sequence instead of laying out its grid.
        projected.root_sequence_diagram = false;
        let keep_edges = projected
            .edges
            .iter()
            .enumerate()
            .filter_map(|(index, edge)| {
                (edge.source != NodeId(0) && edge.target != NodeId(0)).then_some(index)
            })
            .collect::<Vec<_>>();
        if keep_edges.len() != projected.edges.len() {
            projected.edges = keep_edges
                .iter()
                .filter_map(|index| projected.edges.get(*index).cloned())
                .collect();
            projected.sequence_edge_lines = keep_edges
                .iter()
                .enumerate()
                .filter_map(|(index, old)| {
                    projected
                        .sequence_edge_lines
                        .get(&EdgeId(*old as u32))
                        .map(|line| (EdgeId(index as u32), *line))
                })
                .collect();
            retain_edge_data(&mut projected.tala_edge_ids, &keep_edges);
            retain_edge_data(&mut projected.tala_edge_invisible, &keep_edges);
            retain_edge_data(&mut projected.tala_edge_label_fixed, &keep_edges);
            retain_edge_data(&mut projected.edge_indices, &keep_edges);
            retain_edge_data(&mut projected.edge_arrows, &keep_edges);
            retain_edge_data(&mut projected.edge_arrowheads, &keep_edges);
            retain_edge_data(&mut projected.edge_arrowhead_declarations, &keep_edges);
            retain_edge_data(&mut projected.edge_labels, &keep_edges);
            retain_edge_data(&mut projected.edge_label_percentage_presence, &keep_edges);
            retain_edge_data(&mut projected.edge_font_sizes, &keep_edges);
            retain_edge_data(&mut projected.edge_label_bold, &keep_edges);
            retain_edge_data(&mut projected.edge_arrowhead_labels, &keep_edges);
            retain_edge_data(&mut projected.edge_styles, &keep_edges);
            retain_edge_data(&mut projected.edge_links, &keep_edges);
            retain_edge_data(&mut projected.edge_tooltips, &keep_edges);
            retain_edge_data(&mut projected.edge_icons, &keep_edges);
            retain_edge_data(&mut projected.edge_icon_positions, &keep_edges);
            retain_edge_data(&mut projected.edge_icon_border_radii, &keep_edges);
            retain_edge_data(&mut projected.edge_is_curves, &keep_edges);
            retain_edge_data(&mut projected.edge_table_columns, &keep_edges);
            retain_edge_data(&mut projected.edge_routes, &keep_edges);
            retain_edge_data(&mut projected.d2_input_routes, &keep_edges);
            retain_edge_data(&mut projected.d2_edge_validation, &keep_edges);
            retain_edge_data(&mut projected.edge_min_widths, &keep_edges);
            retain_edge_data(&mut projected.edge_min_heights, &keep_edges);
            projected_edges = keep_edges
                .iter()
                .filter_map(|index| projected_edges.get(*index).copied())
                .collect();
            for edges in projected.sequence_group_edges.values_mut() {
                *edges = edges
                    .iter()
                    .filter_map(|edge| {
                        keep_edges
                            .iter()
                            .position(|index| *index == edge.0 as usize)
                            .map(|index| EdgeId(index as u32))
                    })
                    .collect();
            }
            projected
                .sequence_group_edges
                .retain(|_, edges| !edges.is_empty());
        }
        projected.direction = graph
            .node(actor)
            .and_then(|node| node.direction)
            .unwrap_or(graph.direction);
        let dispatcher = LayoutDispatcher;
        let local = if graph.is_sequence_diagram(actor) {
            layout_scope_with_cancellation(
                &projected,
                options,
                NodeId(0),
                engine.clone(),
                cancellation,
            )
        } else if actor_node.grid_rows.is_some() || actor_node.grid_columns.is_some() {
            dispatcher
                .layout_grid_diagram(&projected, engine.clone(), options, cancellation)
                .map_err(super::LayoutDispatchError::into_core_error)
        } else {
            dispatcher
                .layout_with_cancellation(&projected, engine.clone(), options, cancellation)
                .map_err(super::LayoutDispatchError::into_core_error)
        }?;
        let local = super::remap_result(local, &projected_nodes, &projected_edges);
        layouts.insert(actor, local);
    }
    Ok(layouts)
}

fn edge_removed_with_extracted_actor(
    graph: &Graph,
    edge: &Edge,
    extracted_actors: &BTreeMap<NodeId, LayoutResult>,
) -> bool {
    extracted_actors.keys().any(|actor| {
        (edge.source != *actor && is_descendant_or_self(graph, edge.source, Some(*actor)))
            || (edge.target != *actor && is_descendant_or_self(graph, edge.target, Some(*actor)))
    })
}

fn edge_belongs_to_sequence_scope(graph: &Graph, edge: &Edge, root: Option<NodeId>) -> bool {
    let source_owner = sequence_owner(graph, edge.source);
    let target_owner = sequence_owner(graph, edge.target);
    match root {
        Some(root) => source_owner == Some(root) && target_owner == Some(root),
        None => source_owner.is_none() && target_owner.is_none(),
    }
}

fn edge_belongs_to_group(graph: &Graph, edge_id: EdgeId, _edge: &Edge, group: NodeId) -> bool {
    // Source Edge.ContainedBy walks reference scopes, not endpoint ancestry.
    // An empty ownership map is valid for document-root declarations; it does
    // not authorize reclassifying their implicit actor paths as groups.
    graph.sequence_group_edges.iter().any(|(owner, edges)| {
        edges.contains(&edge_id) && is_descendant_or_self(graph, *owner, Some(group))
    })
}

/// Replaces generic-engine geometry with source-shaped sequence geometry for
/// the deepest explicit sequence scopes in a mixed document.
///
/// D2 measures detached sequence contents before laying out their parent,
/// then restores those contents into the parent's resulting rectangle.
/// Preserve that rectangle here, including dimensions assigned by a grid.
pub fn apply_nested_scopes(
    graph: &Graph,
    result: LayoutResult,
    options: &LayoutOptions,
    engine: EngineKind,
) -> Result<LayoutResult, LayoutError> {
    apply_nested_scopes_with_cancellation(graph, result, options, engine, None)
}

/// Restores nested sequence scopes without losing errors or their context.
pub fn apply_nested_scopes_with_cancellation(
    graph: &Graph,
    mut result: LayoutResult,
    options: &LayoutOptions,
    engine: EngineKind,
    cancellation: Option<&CancellationToken>,
) -> Result<LayoutResult, LayoutError> {
    let mut roots = graph
        .nodes()
        .filter_map(|(id, _)| {
            (graph.is_sequence_diagram(id)
                && graph.nodes().any(|(_, child)| child.parent == Some(id)))
            .then_some(id)
        })
        .filter(|id| !has_sequence_descendant(graph, *id))
        .collect::<Vec<_>>();
    roots.sort_by_key(|id| std::cmp::Reverse(depth(graph, *id)));
    for root in roots {
        let local =
            layout_scope_with_cancellation(graph, options, root, engine.clone(), cancellation)?;
        result.removed_edges.extend(&local.removed_edges);
        let Some(local_root) = local.boxes.get(&root).copied() else {
            continue;
        };
        let anchor = result.boxes.get(&root).copied().unwrap_or(local_root);
        let dx = anchor.origin.x - local_root.origin.x;
        let dy = anchor.origin.y - local_root.origin.y;
        result
            .sequence_lifelines
            .retain(|line| !is_descendant_or_self(graph, line.actor, Some(root)));
        for mut line in local.sequence_lifelines {
            for point in &mut line.route {
                point.x += dx;
                point.y += dy;
            }
            result.sequence_lifelines.push(line);
        }

        for (id, rect) in local.boxes {
            if is_descendant_or_self(graph, id, Some(root)) {
                // InjectNested restores the sequence contents without
                // replacing dimensions assigned by the enclosing layout.
                // In particular, grid cells can stretch a sequence root.
                if id == root {
                    result.boxes.insert(id, anchor);
                    continue;
                }
                result.boxes.insert(
                    id,
                    Rect {
                        origin: Point {
                            x: rect.origin.x + dx,
                            y: rect.origin.y + dy,
                        },
                        size: rect.size,
                    },
                );
            }
        }
        for (edge_id, route) in local.routes {
            let Some((_, edge)) = graph.edges().find(|(id, _)| *id == edge_id) else {
                continue;
            };
            if is_descendant_or_self(graph, edge.source, Some(root))
                && is_descendant_or_self(graph, edge.target, Some(root))
            {
                result.routes.insert(
                    edge_id,
                    route
                        .into_iter()
                        .map(|point| Point {
                            x: point.x + dx,
                            y: point.y + dy,
                        })
                        .collect(),
                );
            }
        }
        for (id, state) in local.node_labels {
            if is_descendant_or_self(graph, id, Some(root)) {
                result.node_labels.insert(id, state);
            }
        }
        for (id, label) in local.edge_labels {
            if let Some((_, edge)) = graph.edges().find(|(edge_id, _)| *edge_id == id)
                && is_descendant_or_self(graph, edge.source, Some(root))
                && is_descendant_or_self(graph, edge.target, Some(root))
            {
                result.edge_labels.insert(id, label);
                if local.edge_label_percentage_commits.contains(&id) {
                    result.edge_label_percentage_commits.insert(id);
                }
            }
        }
    }
    Ok(result)
}

fn has_sequence_descendant(graph: &Graph, root: NodeId) -> bool {
    graph.nodes().any(|(id, node)| {
        id != root
            && graph.is_sequence_diagram(id)
            && is_descendant_or_self(graph, id, Some(root))
            && node.parent.is_some()
    })
}

fn note_line(graph: &Graph, id: NodeId) -> i64 {
    graph
        .sequence_node_lines
        .get(&id)
        .copied()
        .unwrap_or(i32::MAX as i64)
}

fn edge_line(graph: &Graph, id: EdgeId) -> i64 {
    graph
        .sequence_edge_lines
        .get(&id)
        .copied()
        .unwrap_or(i32::MAX as i64)
}

fn message_step() -> f64 {
    MIN_MESSAGE_DISTANCE + VERTICAL_PADDING
}

fn depth(graph: &Graph, id: NodeId) -> usize {
    let mut depth = 0;
    let mut parent = graph.node(id).and_then(|node| node.parent);
    while let Some(current) = parent {
        depth += 1;
        parent = graph.node(current).and_then(|node| node.parent);
    }
    depth
}

fn is_descendant_or_self(graph: &Graph, id: NodeId, ancestor: Option<NodeId>) -> bool {
    let Some(ancestor) = ancestor else {
        return true;
    };
    let mut current = Some(id);
    while let Some(candidate) = current {
        if candidate == ancestor {
            return true;
        }
        current = graph.node(candidate).and_then(|node| node.parent);
    }
    false
}

/// Resolved sequence object shape seen by a later parent edge router.
/// newSequenceDiagram changes notes to pages and ordinary actor descendants
/// to spans. Extracted grid actor contents keep their own diagram shapes.
pub(crate) fn routing_shape(graph: &Graph, id: NodeId) -> Option<d2_rs_core::ShapeKind> {
    let root = match sequence_owner(graph, id) {
        Some(root) => Some(root),
        None if graph.root_sequence_diagram => None,
        None => return None,
    };
    let actor = direct_actor(graph, id, root)?;
    if actor == id {
        return None;
    }
    let actor_node = graph.node(actor)?;
    if actor_node.grid_rows.is_some() || actor_node.grid_columns.is_some() {
        return None;
    }
    let mut ancestor = Some(id);
    while let Some(current) = ancestor {
        if Some(current) == root {
            break;
        }
        if is_sequence_group(graph, current, root) {
            return None;
        }
        ancestor = graph.node(current).and_then(|node| node.parent);
    }
    if is_sequence_note(graph, id, root) {
        return Some(d2_rs_core::ShapeKind::Page);
    }
    if is_group_member_endpoint(graph, id, root) {
        return None;
    }
    Some(d2_rs_core::ShapeKind::RealSquare)
}

/// Return the nearest enclosing sequence scope for a node. A sequence root
/// owns itself only for projection purposes; callers looking for outer roots
/// use the parent chain and therefore receive `None` for a top-level scope.
fn sequence_owner(graph: &Graph, id: NodeId) -> Option<NodeId> {
    let mut current = graph.node(id).and_then(|node| node.parent);
    while let Some(candidate) = current {
        if graph.is_sequence_diagram(candidate) {
            return Some(candidate);
        }
        current = graph.node(candidate).and_then(|node| node.parent);
    }
    None
}

/// Returns the actor directly below a sequence scope for any descendant.
fn direct_actor(graph: &Graph, id: NodeId, root: Option<NodeId>) -> Option<NodeId> {
    if root == Some(id) || !is_descendant_or_self(graph, id, root) {
        return None;
    }
    let mut current = id;
    while let Some(parent) = graph.node(current).and_then(|node| node.parent) {
        if Some(parent) == root {
            // The parser represents messages written inside a sequence group
            // as descendants such as `group.a`. D2's sequence layout still
            // routes those endpoints on the root actor `a`; resolve the
            // terminal name when the immediate root child is a group vessel.
            if let Some(actor) = group_member_actor(graph, id, current, root) {
                return Some(actor);
            }
            return Some(current);
        }
        current = parent;
    }
    if root.is_none()
        && let Some(actor) = group_member_actor(graph, id, current, root)
    {
        return Some(actor);
    }
    root.is_none().then_some(current)
}

fn group_member_actor(
    graph: &Graph,
    endpoint: NodeId,
    group: NodeId,
    root: Option<NodeId>,
) -> Option<NodeId> {
    // Source compilation already performed EnsureChild's declaration-order
    // actor lookup. Only legacy measured graphs need this name-based fallback.
    if graph.sequence_node_edge_refs.contains_key(&group) {
        return None;
    }
    if !is_sequence_group(graph, group, root) {
        return None;
    }
    // Group members can be nested below additional edge groups and spans
    // (`group.unfortunate outcome.alice.a`). Walk the endpoint's ancestors and
    // match the first actor-like name against the root actor's terminal path
    // component. Comparing the complete external ID would fail as soon as the
    // sequence itself lived below a named container.
    let mut current = Some(endpoint);
    while let Some(candidate) = current {
        if Some(candidate) == root {
            break;
        }
        // EnsureChild binds actors while compiling their references. A scope
        // already owning messages cannot become an actor proxy through a
        // later same-name actor; getCenter follows its actual parents.
        if graph
            .sequence_group_edges
            .get(&candidate)
            .is_some_and(|edges| !edges.is_empty())
        {
            current = graph.node(candidate).and_then(|node| node.parent);
            continue;
        }
        let terminal = graph
            .node(candidate)
            .and_then(|node| node.external_id.rsplit('.').next());
        if let Some(terminal) = terminal
            && let Some((actor, _)) = graph.nodes().find(|(actor, node)| {
                node.parent == root
                    && *actor != group
                    && !is_sequence_group(graph, *actor, root)
                    && node.external_id.rsplit('.').next() == Some(terminal)
            })
        {
            return Some(actor);
        }
        current = graph.node(candidate).and_then(|node| node.parent);
    }
    None
}

fn is_group_member_endpoint(graph: &Graph, id: NodeId, root: Option<NodeId>) -> bool {
    let mut current = id;
    while let Some(parent) = graph.node(current).and_then(|node| node.parent) {
        if Some(parent) == root {
            return is_sequence_group(graph, current, root)
                && group_member_actor(graph, id, current, root).is_some();
        }
        current = parent;
    }
    root.is_none()
        && is_sequence_group(graph, current, root)
        && group_member_actor(graph, id, current, root).is_some()
}

fn root_actor_with_terminal(
    graph: &Graph,
    root: Option<NodeId>,
    terminal: &str,
    excluded: Option<NodeId>,
) -> Option<NodeId> {
    graph.nodes().find_map(|(id, node)| {
        (node.parent == root
            && Some(id) != excluded
            && !graph.is_sequence_diagram(id)
            && node.external_id.rsplit('.').next() == Some(terminal))
        .then_some(id)
    })
}

fn has_direct_child(graph: &Graph, id: NodeId) -> bool {
    graph.nodes().any(|(_, node)| node.parent == Some(id))
}

/// Mirrors d2graph.Object.IsSequenceDiagramNote for the measured graph.
fn is_sequence_note(graph: &Graph, id: NodeId, root: Option<NodeId>) -> bool {
    let Some(actor) = direct_actor(graph, id, root) else {
        return false;
    };
    Some(id) != root
        && id != actor
        && !has_direct_child(graph, id)
        && !is_group_member_endpoint(graph, id, root)
        && !graph.sequence_has_edge_reference(id)
        && !graph.sequence_contains_object(id)
        && !graph.sequence_group_edges.iter().any(|(owner, edges)| {
            !edges.is_empty() && is_descendant_or_self(graph, *owner, Some(id))
        })
        && graph
            .node(id)
            .is_some_and(|node| node.external_id.contains('.'))
}

/// Mirrors the source group predicate: a group has descendants carrying
/// messages, but the group node itself is not an endpoint.
fn is_sequence_group(graph: &Graph, id: NodeId, root: Option<NodeId>) -> bool {
    if Some(id) == root || !is_descendant_or_self(graph, id, root) {
        return false;
    }
    // LayoutNested extracts a grid's descendants before the sequence pass.
    // Keep references on retained objects (an actor can have been declared
    // again inside this grid), but exclude extracted objects and edges from
    // IsSequenceDiagramGroup's live graph inventory. Actor layout/reinjection
    // below still uses the full graph to restore the grid's geometry.
    let extracted_grid = graph
        .node(id)
        .is_some_and(|node| node.grid_rows.is_some() || node.grid_columns.is_some())
        && has_direct_child(graph, id);
    let live_object =
        |object| !extracted_grid || object == id || !is_descendant_or_self(graph, object, Some(id));
    let live_edge = |edge: &Edge| live_object(edge.source) && live_object(edge.target);
    // d2graph/seqdiagram.go:IsSequenceDiagramGroup rejects message endpoints
    // before examining children. Do that before reconstructing actor-proxy
    // identities, which otherwise scans all root actor names for each endpoint.
    if graph
        .edges()
        .any(|(_, edge)| live_edge(edge) && (edge.source == id || edge.target == id))
    {
        return false;
    }
    // Source sequence reference resolution reattaches actor proxies to the
    // root actor; the proxy is not another group vessel. Legacy measured
    // graphs infer actor aliases from their terminal names; compiled graphs
    // retain actual parents and declaration reference scopes.
    if !graph.sequence_node_edge_refs.contains_key(&id)
        && graph
            .node(id)
            .and_then(|node| node.external_id.rsplit('.').next())
            .is_some_and(|terminal| {
                root_actor_with_terminal(graph, root, terminal, Some(id)).is_some()
            })
    {
        return false;
    }
    let contains_reference_edge = |container| {
        graph.sequence_group_edges.iter().any(|(owner, edges)| {
            is_descendant_or_self(graph, *owner, Some(container))
                && edges
                    .iter()
                    .any(|edge| graph.edges.get(edge.0 as usize).is_some_and(&live_edge))
        })
    };
    let contains_object = if extracted_grid {
        graph.sequence_object_scopes.iter().any(|(object, scopes)| {
            live_object(*object)
                && scopes
                    .iter()
                    .any(|scope| is_descendant_or_self(graph, *scope, Some(id)))
        })
    } else {
        graph.sequence_contains_object(id)
    };
    if !contains_reference_edge(id) && !contains_object {
        return false;
    }
    // d2graph.IsSequenceDiagramGroup checks child reference scopes. A group
    // with no children can still own outward-declared messages; a containing
    // group can own messages through its nested groups.
    graph
        .nodes()
        .filter(|(child, node)| node.parent == Some(id) && live_object(*child))
        .all(|(child, node)| {
            let actor_proxy = !graph.sequence_node_edge_refs.contains_key(&id)
                && !sequence_belongs_to_grid_extraction(graph, id)
                && node.external_id.rsplit('.').next().is_some_and(|terminal| {
                    root_actor_with_terminal(graph, root, terminal, Some(id)).is_some()
                });
            actor_proxy || contains_reference_edge(child)
        })
}

fn validate(graph: &Graph) -> Result<(), LayoutError> {
    for (id, node) in graph.nodes() {
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
    use super::{EngineKind, MIN_SPAN_HEIGHT, SPAN_BASE_WIDTH, layout, layout_scope};
    use d2_rs_core::{Edge, EdgeId, Graph, LabelPosition, LayoutOptions, Node, Size};

    #[test]
    fn empty_sequence_descendants_participate_in_note_and_span_layout() {
        // d2layouts.go:211-215 retains empty special diagrams. The actor's
        // child then enters newSequenceDiagram's normal note/span queue.
        for implicit in [false, true] {
            for message in [false, true] {
                let mut graph = Graph {
                    root_sequence_diagram: implicit,
                    ..Graph::default()
                };
                let size = Size {
                    width: 100.0,
                    height: 40.0,
                };
                let root = if implicit {
                    None
                } else {
                    let root = graph.add_node(Node::new("scope", size));
                    graph.set_sequence_diagram(root, true);
                    Some(root)
                };
                let prefix = if implicit { "" } else { "scope." };
                let actor = graph.add_node(Node {
                    parent: root,
                    ..Node::new(format!("{prefix}actor"), size)
                });
                let child = graph.add_node(Node {
                    parent: Some(actor),
                    ..Node::new(format!("{prefix}actor.child"), size)
                });
                let other = graph.add_node(Node {
                    parent: root,
                    ..Node::new(format!("{prefix}other"), size)
                });
                if message {
                    graph.add_edge(Edge {
                        source: child,
                        target: other,
                    });
                }
                graph.set_sequence_diagram(child, true);
                let mut ordinary = graph.clone();
                ordinary.set_sequence_diagram(child, false);
                for engine in [EngineKind::Dagre, EngineKind::Elk, EngineKind::Tala] {
                    let result = super::LayoutDispatcher
                        .layout(&graph, engine.clone(), &LayoutOptions::default())
                        .unwrap();
                    let expected = super::LayoutDispatcher
                        .layout(&ordinary, engine, &LayoutOptions::default())
                        .unwrap();
                    assert!(result.boxes.contains_key(&child));
                    assert_eq!(result.boxes, expected.boxes);
                    assert_eq!(result.node_labels, expected.node_labels);
                    assert_eq!(result.routes, expected.routes);
                    assert_eq!(result.sequence_lifelines, expected.sequence_lifelines);
                }
            }
        }
    }

    #[test]
    fn sequence_actors_use_non_glob_declaration_order_for_placement_and_lifelines() {
        // d2sequence/sequence_diagram.go newSequenceDiagram/getObjEarliestLineNum:
        // `a.*; b` creates a first, but its only reference is a glob, so b
        // precedes a in the actor and synthetic lifeline arrays.
        for implicit in [false, true] {
            let mut graph = Graph::default();
            let size = Size {
                width: 100.0,
                height: 66.0,
            };
            let root = if implicit {
                graph.root_sequence_diagram = true;
                None
            } else {
                let root = graph.add_node(Node::new("scope", size));
                graph.set_sequence_diagram(root, true);
                Some(root)
            };
            let a = graph.add_node(Node {
                parent: root,
                ..Node::new("a", size)
            });
            let b = graph.add_node(Node {
                parent: root,
                ..Node::new("b", size)
            });
            graph.sequence_node_lines.insert(b, 2);
            for engine in [EngineKind::Dagre, EngineKind::Elk, EngineKind::Tala] {
                let result = super::LayoutDispatcher
                    .layout(&graph, engine, &LayoutOptions::default())
                    .unwrap();
                assert!(result.boxes[&b].origin.x < result.boxes[&a].origin.x);
                assert_eq!(
                    result
                        .sequence_lifelines
                        .iter()
                        .map(|line| line.actor)
                        .collect::<Vec<_>>(),
                    vec![b, a]
                );
            }
        }
    }

    #[test]
    fn empty_nested_sequences_are_rejected_in_declaration_order() {
        // d2layouts.LayoutNested leaves empty special diagrams in place;
        // d2sequence.newSequenceDiagram rejects them before the empty-actor
        // check, using getObjEarliestLineNum and the extracted scope's AbsID.
        for implicit in [false, true] {
            let mut graph = Graph::default();
            let size = Size {
                width: 100.0,
                height: 40.0,
            };
            let root = if implicit {
                graph.root_sequence_diagram = true;
                None
            } else {
                let root = graph.add_node(Node::new("scope", size));
                graph.set_sequence_diagram(root, true);
                Some(root)
            };
            let prefix = if implicit { "" } else { "scope." };
            let mut later = Node::new(format!("{prefix}later"), size);
            later.parent = root;
            let later = graph.add_node(later);
            graph.set_sequence_diagram(later, true);
            graph.sequence_node_lines.insert(later, 9);
            let mut first = Node::new(format!("{prefix}first"), size);
            first.parent = root;
            let first = graph.add_node(first);
            graph.set_sequence_diagram(first, true);
            graph.sequence_node_lines.insert(first, 2);
            for engine in [EngineKind::Dagre, EngineKind::Elk, EngineKind::Tala] {
                let error = super::LayoutDispatcher
                    .layout(&graph, engine, &LayoutOptions::default())
                    .unwrap_err();
                assert!(
                    error.to_string().ends_with(
                        "actors in sequence diagrams cannot themselves be sequence diagrams: first"
                    ),
                    "{error}"
                );
            }
            graph.set_sequence_diagram(first, false);
            graph.set_sequence_diagram(later, false);
            assert!(
                super::LayoutDispatcher
                    .layout(&graph, EngineKind::Dagre, &LayoutOptions::default())
                    .is_ok()
            );
        }
    }

    #[test]
    fn nested_sequence_contents_are_validated_before_outer_actors() {
        // LayoutNested extracts populated sequences before calling the outer
        // newSequenceDiagram. An error in a deeper sequence therefore takes
        // precedence over an earlier empty sequence actor in the parent.
        for implicit in [false, true] {
            for invalid_inner in [false, true] {
                let mut graph = Graph {
                    root_sequence_diagram: implicit,
                    ..Default::default()
                };
                let size = Size {
                    width: 100.0,
                    height: 40.0,
                };
                let root = if implicit {
                    None
                } else {
                    let root = graph.add_node(Node::new("scope", size));
                    graph.set_sequence_diagram(root, true);
                    Some(root)
                };
                let prefix = if implicit { "" } else { "scope." };
                let empty = graph.add_node(Node {
                    parent: root,
                    ..Node::new(format!("{prefix}empty"), size)
                });
                graph.set_sequence_diagram(empty, true);
                graph.sequence_node_lines.insert(empty, 1);
                let nested = graph.add_node(Node {
                    parent: root,
                    ..Node::new(format!("{prefix}nested"), size)
                });
                graph.set_sequence_diagram(nested, true);
                graph.sequence_node_lines.insert(nested, 2);
                let children = ["first", "second"].map(|name| {
                    graph.add_node(Node {
                        parent: Some(nested),
                        ..Node::new(format!("{prefix}nested.{name}"), size)
                    })
                });
                if invalid_inner {
                    graph.set_sequence_diagram(children[0], true);
                } else {
                    graph.add_edge(Edge {
                        source: children[0],
                        target: children[1],
                    });
                }
                for engine in [EngineKind::Dagre, EngineKind::Elk, EngineKind::Tala] {
                    let error = super::LayoutDispatcher
                        .layout(&graph, engine, &LayoutOptions::default())
                        .unwrap_err();
                    let actor = if invalid_inner { "first" } else { "empty" };
                    assert_eq!(
                        error.to_string(),
                        format!(
                            "actors in sequence diagrams cannot themselves be sequence diagrams: {actor}"
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn extracted_grid_descendant_notes_do_not_extend_outer_lifelines() {
        let mut graph = Graph::default();
        graph.root_sequence_diagram = true;
        let size = Size {
            width: 100.0,
            height: 40.0,
        };
        let mut grid = Node::new("grid", size);
        grid.grid_columns = Some(1);
        let grid = graph.add_node(grid);
        let mut note = Node::new("grid.note", size);
        note.parent = Some(grid);
        let note = graph.add_node(note);
        let other = graph.add_node(Node::new("other", size));
        graph.add_edge(Edge {
            source: grid,
            target: other,
        });
        let mut result = layout(&graph, &LayoutOptions::default()).unwrap();
        result.boxes.get_mut(&note).unwrap().origin.y = 10000.0;
        let lines = super::fixed_lifeline_routes(&graph, &result);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|(_, route)| route[1].y < 10000.0));
    }

    #[test]
    fn fixed_lifelines_use_the_last_message_not_the_highest_previous_route() {
        let mut graph = Graph::default();
        graph.root_sequence_diagram = true;
        let size = Size {
            width: 100.0,
            height: 40.0,
        };
        let a = graph.add_node(Node::new("a", size));
        let b = graph.add_node(Node::new("b", size));
        let earlier = graph.add_edge(Edge {
            source: a,
            target: b,
        });
        let last = graph.add_edge(Edge {
            source: b,
            target: a,
        });
        let mut result = layout(&graph, &LayoutOptions::default()).unwrap();
        result
            .routes
            .insert(earlier, vec![d2_rs_core::Point { x: 0.0, y: 1000.0 }]);
        result
            .routes
            .insert(last, vec![d2_rs_core::Point { x: 0.0, y: 300.0 }]);
        let lines = super::fixed_lifeline_routes(&graph, &result);
        assert_eq!(lines.len(), 2);
        for (actor, route) in lines {
            assert_eq!(route[0].y, result.boxes[&actor].bottom());
            assert_eq!(route[1].y, 370.0);
        }
    }

    #[test]
    fn sequence_root_height_uses_lifeline_end_not_tall_group_bounds() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let actor_size = Size {
            width: 100.0,
            height: 40.0,
        };
        let source = graph.add_node(Node::new("a", actor_size));
        let target = graph.add_node(Node::new("b", actor_size));
        let mut outer_node = Node::new("outer", actor_size);
        outer_node.label_size = Some(Size {
            width: 20.0,
            height: 21.0,
        });
        let outer = graph.add_node(outer_node);
        let mut inner_node = Node::new("outer.inner", actor_size);
        inner_node.parent = Some(outer);
        inner_node.label_size = Some(Size {
            width: 20.0,
            height: 21.0,
        });
        let inner = graph.add_node(inner_node);
        let message = graph.add_edge(Edge { source, target });
        graph.set_edge_label(
            message,
            Some(d2_rs_core::EdgeLabel {
                text: "six\nline\nmessage\nlabel\nfor\ngroup".to_owned(),
                size: Size {
                    width: 60.0,
                    height: 101.0,
                },
                position: LabelPosition::OutsideTopCenter,
                percentage: 0.5,
            }),
        );
        graph.sequence_group_edges.insert(outer, vec![message]);
        graph.sequence_group_edges.insert(inner, vec![message]);

        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        let lifeline_end = result.sequence_lifelines[0].route[1].y;
        let root = result.synthetic_root.as_ref().unwrap();
        assert_eq!(
            root.size.height,
            lifeline_end + super::GROUP_CONTAINER_PADDING
        );
        assert!(result.boxes[&outer].bottom() > lifeline_end);
    }

    #[test]
    fn sequence_projection_extracts_crossing_edges_without_root_aliases() {
        let mut graph = Graph::default();
        let size = Size {
            width: 100.0,
            height: 40.0,
        };
        let root = graph.add_node(Node::new("sequence", size));
        graph.set_sequence_diagram(root, true);
        let mut actor = Node::new("sequence.actor", size);
        actor.parent = Some(root);
        let actor = graph.add_node(actor);
        let outside = graph.add_node(Node::new("outside", size));
        let crossing = graph.add_edge(Edge {
            source: outside,
            target: actor,
        });
        let direct = graph.add_edge(Edge {
            source: outside,
            target: root,
        });
        for engine in [EngineKind::Tala, EngineKind::Dagre] {
            let projected = super::project_for_engine_with_cache(
                &graph,
                &LayoutOptions::default(),
                None,
                engine,
                None,
            )
            .unwrap()
            .unwrap();
            assert_eq!(projected.edge_map, vec![direct]);
            assert!(!projected.edge_map.contains(&crossing));
            assert_eq!(projected.graph.edges.len(), 1);
        }
    }

    #[test]
    fn grid_extraction_retains_nested_sequence_objects_and_routes() {
        let mut graph = Graph::default();
        let size = Size {
            width: 100.0,
            height: 40.0,
        };
        let mut grid = Node::new("grid", size);
        grid.grid_rows = Some(2);
        let grid = graph.add_node(grid);
        let mut sequence = Node::new("grid.sequence", size);
        sequence.parent = Some(grid);
        let sequence = graph.add_node(sequence);
        graph.set_sequence_diagram(sequence, true);
        let mut actor = Node::new("grid.sequence.actor", size);
        actor.parent = Some(sequence);
        let actor = graph.add_node(actor);
        let mut sibling = Node::new("grid.sibling", size);
        sibling.parent = Some(grid);
        let sibling = graph.add_node(sibling);
        let crossing = graph.add_edge(Edge {
            source: sibling,
            target: actor,
        });
        for engine in [EngineKind::Tala, EngineKind::Dagre] {
            // The grid owns this whole graph at the parent's extraction phase.
            assert!(
                super::project_for_engine_with_cache(
                    &graph,
                    &LayoutOptions::default(),
                    None,
                    engine.clone(),
                    None,
                )
                .unwrap()
                .is_none()
            );
            // An unrelated sequence is extracted without losing grid contents.
            let mut mixed = graph.clone();
            let other = mixed.add_node(Node::new("other", size));
            mixed.set_sequence_diagram(other, true);
            let mut child = Node::new("other.child", size);
            child.parent = Some(other);
            let child = mixed.add_node(child);
            let projection = super::project_for_engine_with_cache(
                &mixed,
                &LayoutOptions::default(),
                None,
                engine,
                None,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                projection.locals.keys().copied().collect::<Vec<_>>(),
                [other]
            );
            assert!(projection.node_map.contains(&actor));
            assert!(!projection.node_map.contains(&child));
            assert!(projection.edge_map.contains(&crossing));
            let edge = &projection.graph.edges[0];
            assert_eq!(projection.node_map[edge.source.0 as usize], sibling);
            assert_eq!(projection.node_map[edge.target.0 as usize], actor);
        }
    }

    #[test]
    fn sequence_reinjection_preserves_parent_container_dimensions() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node::new(
            "sequence",
            Size {
                width: 1.0,
                height: 1.0,
            },
        ));
        graph.set_sequence_diagram(root, true);
        let mut actor = Node::new(
            "sequence.actor",
            Size {
                width: 100.0,
                height: 40.0,
            },
        );
        actor.parent = Some(root);
        let actor = graph.add_node(actor);
        let local =
            layout_scope(&graph, &LayoutOptions::default(), root, EngineKind::Tala).unwrap();
        let original_actor = local.boxes[&actor];
        let original_line = local.sequence_lifelines[0].clone();
        let mut parent = local.clone();
        let anchor = d2_rs_core::Rect {
            origin: d2_rs_core::Point { x: 50.0, y: 70.0 },
            size: Size {
                width: 500.0,
                height: 600.0,
            },
        };
        parent.boxes.insert(root, anchor);
        let restored_scope = super::apply_nested_scopes(
            &graph,
            parent.clone(),
            &LayoutOptions::default(),
            EngineKind::Tala,
        )
        .unwrap();
        assert_eq!(restored_scope.boxes[&root], anchor);
        assert_eq!(restored_scope.boxes[&actor].size, original_actor.size);
        let restored = super::apply_projection_locals(&graph, parent, [(root, local)].into());
        assert_eq!(restored.boxes[&root], anchor);
        assert_eq!(restored.sequence_lifelines.len(), 1);
        assert_eq!(restored.sequence_lifelines[0].actor, actor);
        assert_eq!(
            restored.sequence_lifelines[0].target_id,
            original_line.target_id
        );
        assert_eq!(
            restored.sequence_lifelines[0].route[0].x,
            original_line.route[0].x + 50.0
        );
        assert_eq!(
            restored.sequence_lifelines[0].route[1].y,
            original_line.route[1].y + 70.0
        );
        assert_eq!(
            restored.boxes[&actor].origin.x,
            original_actor.origin.x + 50.0
        );
        assert_eq!(
            restored.boxes[&actor].origin.y,
            original_actor.origin.y + 70.0
        );
    }

    #[test]
    fn sequence_actor_label_and_icon_positions_follow_go_placement() {
        let mut graph = Graph::default();
        graph.root_sequence_diagram = true;
        let size = Size {
            width: 100.0,
            height: 40.0,
        };
        let plain = graph.add_node(Node::new("plain", size));
        let mut person = Node::new("person", size);
        person.shape = d2_rs_core::ShapeKind::Person;
        let person = graph.add_node(person);
        let mut icon = Node::new("icon", size);
        icon.has_icon = true;
        let icon = graph.add_node(icon);
        let mut explicit = Node::new("explicit", size);
        explicit.icon_position = Some(LabelPosition::InsideTopRight);
        explicit.label_position = LabelPosition::OutsideLeftMiddle;
        let explicit = graph.add_node(explicit);
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(
            result.node_labels[&plain].position,
            LabelPosition::InsideMiddleCenter
        );
        assert_eq!(
            result.node_labels[&person].position,
            LabelPosition::OutsideBottomCenter
        );
        assert_eq!(
            result.node_labels[&icon].position,
            LabelPosition::OutsideTopCenter
        );
        assert_eq!(
            result.node_labels[&icon].icon_position,
            Some(LabelPosition::InsideMiddleCenter)
        );
        assert_eq!(
            result.node_labels[&explicit].position,
            LabelPosition::OutsideLeftMiddle
        );
        assert_eq!(
            result.node_labels[&explicit].icon_position,
            Some(LabelPosition::InsideTopRight)
        );
    }

    #[test]
    fn places_sequence_actors_and_message_rows() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node::new(
            "sequence",
            Size {
                width: 1.0,
                height: 1.0,
            },
        ));
        graph.set_sequence_diagram(root, true);
        let first = graph.add_node(Node::new(
            "first",
            Size {
                width: 80.0,
                height: 30.0,
            },
        ));
        let second = graph.add_node(Node::new(
            "second",
            Size {
                width: 80.0,
                height: 30.0,
            },
        ));
        graph.node_mut(first).unwrap().parent = Some(root);
        graph.node_mut(second).unwrap().parent = Some(root);
        graph.add_edge(Edge {
            source: first,
            target: second,
        });
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(result.boxes.len(), 3);
        assert_eq!(result.routes.len(), 1);
        assert_eq!(
            result.boxes[&first].origin,
            d2_rs_core::Point { x: 12.0, y: 52.0 }
        );
        assert_eq!(
            result.boxes[&second].origin,
            d2_rs_core::Point { x: 162.0, y: 52.0 }
        );
        assert_eq!(result.routes[&EdgeId(0)][0].y, 152.0);
        assert!(result.boxes[&second].origin.x > result.boxes[&first].origin.x);
    }

    #[test]
    fn nested_sequence_owns_its_messages_and_root_label_position() {
        let mut graph = Graph::default();
        let outer = graph.add_node(Node::new(
            "outer",
            Size {
                width: 1.0,
                height: 1.0,
            },
        ));
        graph.set_sequence_diagram(outer, true);
        let mut first = Node::new(
            "outer.first",
            Size {
                width: 80.0,
                height: 30.0,
            },
        );
        first.parent = Some(outer);
        let first = graph.add_node(first);
        let mut second = Node::new(
            "outer.second",
            Size {
                width: 80.0,
                height: 30.0,
            },
        );
        second.parent = Some(outer);
        let second = graph.add_node(second);
        let mut nested = Node::new(
            "outer.nested",
            Size {
                width: 1.0,
                height: 1.0,
            },
        );
        nested.parent = Some(outer);
        nested.label_size = Some(Size {
            width: 64.0,
            height: 21.0,
        });
        let nested = graph.add_node(nested);
        graph.set_sequence_diagram(nested, true);
        let mut inner_first = Node::new(
            "outer.nested.inner-first",
            Size {
                width: 80.0,
                height: 30.0,
            },
        );
        inner_first.parent = Some(nested);
        let inner_first = graph.add_node(inner_first);
        let mut inner_second = Node::new(
            "outer.nested.inner-second",
            Size {
                width: 80.0,
                height: 30.0,
            },
        );
        inner_second.parent = Some(nested);
        let inner_second = graph.add_node(inner_second);

        let outer_message = graph.add_edge(Edge {
            source: first,
            target: second,
        });
        let inner_message = graph.add_edge(Edge {
            source: inner_first,
            target: inner_second,
        });
        let cross_scope_message = graph.add_edge(Edge {
            source: inner_second,
            target: second,
        });

        // d2layouts.LayoutNested retains the nested root as a placeholder;
        // d2sequence.newSequenceDiagram rejects it as an actor (:98-100).
        // Pinned Go rejects the equivalent source even with crossing messages.
        for engine in [EngineKind::Dagre, EngineKind::Elk, EngineKind::Tala] {
            let error = layout_scope(&graph, &LayoutOptions::default(), outer, engine).unwrap_err();
            assert_eq!(
                error.to_string(),
                "actors in sequence diagrams cannot themselves be sequence diagrams: nested"
            );
        }

        let nested_layout =
            layout_scope(&graph, &LayoutOptions::default(), nested, EngineKind::Dagre).unwrap();
        assert!(nested_layout.routes.contains_key(&inner_message));
        assert!(!nested_layout.routes.contains_key(&outer_message));
        assert!(!nested_layout.routes.contains_key(&cross_scope_message));
        assert_eq!(
            nested_layout.node_labels[&nested].position,
            LabelPosition::InsideTopCenter
        );
    }

    #[test]
    fn actor_gaps_read_next_width_before_its_minimum_width_adjustment() {
        // d2sequence.newSequenceDiagram processes actors in source order.
        // Both cases straddle the 150px minimum gap, exposing a 10px / 3px
        // drift if all actor widths are widened before computing the gaps.
        for (first_width, second_width, expected_width, expected_second_x) in
            [(140.0, 80.0, 294.0, 182.0), (126.0, 94.0, 287.0, 175.0)]
        {
            let mut graph = Graph::default();
            let root = graph.add_node(Node::new(
                "sequence",
                Size {
                    width: 1.0,
                    height: 1.0,
                },
            ));
            graph.set_sequence_diagram(root, true);
            let first = graph.add_node(Node::new(
                "first",
                Size {
                    width: first_width,
                    height: 30.0,
                },
            ));
            let second = graph.add_node(Node::new(
                "second",
                Size {
                    width: second_width,
                    height: 30.0,
                },
            ));
            graph.node_mut(first).unwrap().parent = Some(root);
            graph.node_mut(second).unwrap().parent = Some(root);
            let result = layout(&graph, &LayoutOptions::default()).unwrap();
            assert_eq!(result.boxes[&root].size.width, expected_width);
            assert_eq!(result.boxes[&second].origin.x, expected_second_x);
            assert_eq!(result.boxes[&second].size.width, 100.0);
        }
    }

    #[test]
    fn final_actor_note_does_not_expand_the_preceding_gap() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node::new(
            "sequence",
            Size {
                width: 1.0,
                height: 1.0,
            },
        ));
        graph.set_sequence_diagram(root, true);
        let first = graph.add_node(Node::new(
            "first",
            Size {
                width: 100.0,
                height: 30.0,
            },
        ));
        let second = graph.add_node(Node::new(
            "second",
            Size {
                width: 100.0,
                height: 30.0,
            },
        ));
        graph.node_mut(first).unwrap().parent = Some(root);
        graph.node_mut(second).unwrap().parent = Some(root);
        let note = graph.add_node(Node::new(
            "second.note",
            Size {
                width: 500.0,
                height: 24.0,
            },
        ));
        graph.node_mut(note).unwrap().parent = Some(second);
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(result.boxes[&second].origin.x, 162.0);
        assert_eq!(result.boxes[&root].size.width, 274.0);
    }

    #[test]
    fn same_actor_label_reserves_space_before_the_next_actor() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node::new(
            "sequence",
            Size {
                width: 1.0,
                height: 1.0,
            },
        ));
        graph.set_sequence_diagram(root, true);
        let first = graph.add_node(Node::new(
            "first",
            Size {
                width: 100.0,
                height: 30.0,
            },
        ));
        let second = graph.add_node(Node::new(
            "second",
            Size {
                width: 100.0,
                height: 30.0,
            },
        ));
        graph.node_mut(first).unwrap().parent = Some(root);
        graph.node_mut(second).unwrap().parent = Some(root);
        let edge = graph.add_edge(Edge {
            source: first,
            target: first,
        });
        graph.set_edge_label(
            edge,
            Some(d2_rs_core::EdgeLabel {
                text: "wide self message".to_owned(),
                size: Size {
                    width: 230.0,
                    height: 20.0,
                },
                position: d2_rs_core::LabelPosition::OutsideTopCenter,
                percentage: 0.5,
            }),
        );
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(
            result.edge_labels[&edge].position,
            d2_rs_core::LabelPosition::InsideMiddleCenter
        );
        assert_eq!(result.edge_labels[&edge].percentage, 0.5);
        assert_eq!(result.boxes[&second].origin.x, 262.0);
        assert_eq!(result.boxes[&root].size.width, 374.0);
    }

    #[test]
    fn parent_router_sees_span_and_note_shapes_but_preserves_grid_actor_contents() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node::new("sequence", Size::default()));
        graph.set_sequence_diagram(root, true);
        let mut actor = Node::new("sequence.actor", Size::default());
        actor.parent = Some(root);
        let actor = graph.add_node(actor);
        let mut span = Node::new("sequence.actor.span", Size::default());
        span.parent = Some(actor);
        let span = graph.add_node(span);
        graph.add_edge(Edge {
            source: actor,
            target: span,
        });
        let mut note = Node::new("sequence.actor.note", Size::default());
        note.parent = Some(actor);
        let note = graph.add_node(note);
        let mut grid = Node::new("sequence.grid", Size::default());
        grid.parent = Some(root);
        grid.grid_columns = Some(1);
        let grid = graph.add_node(grid);
        let mut child = Node::new("sequence.grid.child", Size::default());
        child.parent = Some(grid);
        let child = graph.add_node(child);
        assert_eq!(super::routing_shape(&graph, actor), None);
        assert_eq!(
            super::routing_shape(&graph, span),
            Some(d2_rs_core::ShapeKind::RealSquare)
        );
        assert_eq!(
            super::routing_shape(&graph, note),
            Some(d2_rs_core::ShapeKind::Page)
        );
        assert_eq!(super::routing_shape(&graph, child), None);
    }

    #[test]
    fn self_message_elbows_and_bounds_use_label_width() {
        // Source routeMessages/getWidth: 80px minimum horizontal travel,
        // otherwise half the label width plus 10px; self-labels extend half
        // their width beyond the outer elbow. Tall labels do not widen it.
        for (label_size, expected_elbow_x, expected_width) in [
            (None, 142.0, 154.0),
            (
                Some(Size {
                    width: 60.0,
                    height: 20.0,
                }),
                142.0,
                184.0,
            ),
            (
                Some(Size {
                    width: 230.0,
                    height: 20.0,
                }),
                187.0,
                314.0,
            ),
            (
                Some(Size {
                    width: 20.0,
                    height: 300.0,
                }),
                142.0,
                164.0,
            ),
        ] {
            let mut graph = Graph::default();
            let root = graph.add_node(Node::new(
                "sequence",
                Size {
                    width: 1.0,
                    height: 1.0,
                },
            ));
            graph.set_sequence_diagram(root, true);
            let actor = graph.add_node(Node::new(
                "actor",
                Size {
                    width: 100.0,
                    height: 30.0,
                },
            ));
            graph.node_mut(actor).unwrap().parent = Some(root);
            let edge = graph.add_edge(Edge {
                source: actor,
                target: actor,
            });
            if let Some(size) = label_size {
                graph.set_edge_label(
                    edge,
                    Some(d2_rs_core::EdgeLabel {
                        text: "measured self message".to_owned(),
                        size,
                        position: d2_rs_core::LabelPosition::Unset,
                        percentage: 0.5,
                    }),
                );
            }
            let result = layout(&graph, &LayoutOptions::default()).unwrap();
            assert_eq!(result.routes[&edge][1].x, expected_elbow_x);
            assert_eq!(result.routes[&edge][2].x, expected_elbow_x);
            assert_eq!(result.boxes[&root].size.width, expected_width);
        }
    }

    #[test]
    fn descendant_message_bounds_include_route_without_self_label_extension() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node::new(
            "sequence",
            Size {
                width: 1.0,
                height: 1.0,
            },
        ));
        graph.set_sequence_diagram(root, true);
        let actor = graph.add_node(Node::new(
            "actor",
            Size {
                width: 100.0,
                height: 30.0,
            },
        ));
        graph.node_mut(actor).unwrap().parent = Some(root);
        let span = graph.add_node(Node::new(
            "actor.span",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        graph.node_mut(span).unwrap().parent = Some(actor);
        let edge = graph.add_edge(Edge {
            source: actor,
            target: span,
        });
        graph.set_edge_label(
            edge,
            Some(d2_rs_core::EdgeLabel {
                text: "measured descendant message".to_owned(),
                size: Size {
                    width: 230.0,
                    height: 20.0,
                },
                position: d2_rs_core::LabelPosition::Unset,
                percentage: 0.5,
            }),
        );
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(result.routes[&edge][1].x, 187.0);
        assert_eq!(result.boxes[&root].size.width, 199.0);
    }

    #[test]
    fn routes_self_and_descendant_messages_with_source_shaped_elbows() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node::new(
            "sequence",
            Size {
                width: 1.0,
                height: 1.0,
            },
        ));
        graph.set_sequence_diagram(root, true);
        let actor = graph.add_node(Node::new(
            "actor",
            Size {
                width: 80.0,
                height: 30.0,
            },
        ));
        let other = graph.add_node(Node::new(
            "other",
            Size {
                width: 80.0,
                height: 30.0,
            },
        ));
        graph.node_mut(actor).unwrap().parent = Some(root);
        graph.node_mut(other).unwrap().parent = Some(root);
        let descendant = graph.add_node(Node::new(
            "actor.child",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        graph.node_mut(descendant).unwrap().parent = Some(actor);
        let self_edge = graph.add_edge(Edge {
            source: actor,
            target: actor,
        });
        let descendant_edge = graph.add_edge(Edge {
            source: actor,
            target: descendant,
        });
        graph.add_edge(Edge {
            source: actor,
            target: other,
        });

        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(result.routes[&self_edge].len(), 4);
        assert_eq!(result.routes[&descendant_edge].len(), 4);
        assert!(result.boxes[&descendant].size.width >= SPAN_BASE_WIDTH);
        assert!(result.boxes[&descendant].size.height >= MIN_SPAN_HEIGHT);
        assert_eq!(result.routes[&EdgeId(2)].len(), 2);
    }

    #[test]
    fn sequence_spans_enclose_descendant_notes_without_messages() {
        let mut graph = Graph::default();
        let size = Size {
            width: 40.0,
            height: 30.0,
        };
        let root = graph.add_node(Node::new("sequence", size));
        graph.set_sequence_diagram(root, true);
        let actor = graph.add_node(Node::new("actor", size));
        graph.node_mut(actor).unwrap().parent = Some(root);
        let span = graph.add_node(Node::new("actor.span", size));
        graph.node_mut(span).unwrap().label_size = Some(Size {
            width: 25.0,
            height: 21.0,
        });
        graph.node_mut(span).unwrap().parent = Some(actor);
        let note = graph.add_node(Node::new("actor.span.note", size));
        graph.node_mut(note).unwrap().parent = Some(span);

        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(result.node_labels[&span].size, None);
        assert!(result.boxes.contains_key(&note));
        assert!(result.boxes[&span].size.width >= SPAN_BASE_WIDTH);
        assert!(result.boxes[&span].size.height >= MIN_SPAN_HEIGHT);
        assert!(result.boxes[&span].bottom() >= result.boxes[&note].bottom());
    }

    #[test]
    fn stacks_sequence_notes_below_message_rows() {
        let mut graph = Graph::default();
        let root = graph.add_node(Node::new(
            "sequence",
            Size {
                width: 1.0,
                height: 1.0,
            },
        ));
        graph.set_sequence_diagram(root, true);
        let actor = graph.add_node(Node::new(
            "actor",
            Size {
                width: 80.0,
                height: 30.0,
            },
        ));
        let other = graph.add_node(Node::new(
            "other",
            Size {
                width: 80.0,
                height: 30.0,
            },
        ));
        graph.node_mut(actor).unwrap().parent = Some(root);
        graph.node_mut(other).unwrap().parent = Some(root);
        let note = graph.add_node(Node::new(
            "actor.note",
            Size {
                width: 90.0,
                height: 24.0,
            },
        ));
        graph.node_mut(note).unwrap().parent = Some(actor);
        let edge = graph.add_edge(Edge {
            source: actor,
            target: other,
        });

        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert!(result.boxes[&note].origin.y > result.boxes[&actor].bottom());
        assert!(result.boxes[&note].origin.y >= result.routes[&edge][0].y);
    }

    #[test]
    fn resolves_group_member_endpoints_to_root_actors() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let actor_a = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let actor_b = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let group = graph.add_node(Node::new(
            "group",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let group_a = graph.add_node(Node::new(
            "group.a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let group_b = graph.add_node(Node::new(
            "group.b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        graph.node_mut(group_a).unwrap().parent = Some(group);
        graph.node_mut(group_b).unwrap().parent = Some(group);
        let inside = graph.add_edge(Edge {
            source: group_a,
            target: group_b,
        });
        let outside = graph.add_edge(Edge {
            source: actor_a,
            target: actor_b,
        });

        graph.sequence_group_edges.insert(group, vec![inside]);
        assert!(super::is_sequence_group(&graph, group, None));
        assert_eq!(super::direct_actor(&graph, group_a, None), Some(actor_a));
        assert_eq!(super::direct_actor(&graph, group_b, None), Some(actor_b));
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(result.routes[&inside].len(), 2);
        assert_eq!(result.routes[&outside].len(), 2);
        assert!(result.boxes.contains_key(&group));

        // Compiled source has already resolved EnsureChild identities. When
        // `group: {a -> b}` precedes `a -> b`, Go keeps group.a/group.b as
        // spans of the group actor; a later actor does not change their owner.
        // IsSequenceDiagramGroup checks each child's reference scopes, not
        // whether a root actor happens to share its name (d2graph/seqdiagram.go).
        for id in [actor_a, actor_b, group, group_a, group_b] {
            graph.sequence_node_edge_refs.insert(id, id != group);
        }
        graph.sequence_object_scopes.insert(group_a, vec![group]);
        graph.sequence_object_scopes.insert(group_b, vec![group]);
        assert!(!super::is_sequence_group(&graph, group, None));
        assert_eq!(super::direct_actor(&graph, group_a, None), Some(group));
        assert_eq!(super::direct_actor(&graph, group_b, None), Some(group));
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert_eq!(result.routes[&inside].len(), 4);
        assert_eq!(result.routes[&outside].len(), 2);
        for id in [group_a, group_b] {
            assert!(result.boxes[&id].origin.x.is_finite());
            assert!(result.boxes[&id].origin.y.is_finite());
        }
    }
    #[test]
    fn root_implicit_actor_paths_are_not_groups_without_owned_references() {
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let a = graph.add_node(Node::new(
            "a",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let b = graph.add_node(Node::new(
            "b",
            Size {
                width: 40.0,
                height: 30.0,
            },
        ));
        let p = graph.add_node(Node {
            parent: Some(a),
            ..Node::new(
                "a.p",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            )
        });
        let t = graph.add_node(Node {
            parent: Some(b),
            ..Node::new(
                "b.t",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            )
        });
        let q = graph.add_node(Node {
            parent: Some(p),
            ..Node::new(
                "a.p.q",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            )
        });
        let r = graph.add_node(Node {
            parent: Some(q),
            ..Node::new(
                "a.p.q.r",
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            )
        });
        let message = graph.add_edge(Edge {
            source: r,
            target: t,
        });
        assert!(graph.sequence_group_edges.is_empty());
        assert!(!super::is_sequence_group(&graph, a, None));
        assert!(!super::is_sequence_group(&graph, b, None));
        assert!(!super::edge_belongs_to_group(
            &graph,
            message,
            &Edge {
                source: p,
                target: t
            },
            a
        ));
        assert_eq!(super::direct_actor(&graph, p, None), Some(a));
        let result = layout(&graph, &LayoutOptions::default()).unwrap();
        assert!(result.routes.contains_key(&message));
        assert_eq!(result.boxes[&p].size.width, SPAN_BASE_WIDTH);
        assert_eq!(
            result.boxes[&q].size.width,
            SPAN_BASE_WIDTH + super::SPAN_DEPTH_GROWTH_FACTOR
        );
        assert_eq!(
            result.boxes[&r].size.width,
            SPAN_BASE_WIDTH + 2.0 * super::SPAN_DEPTH_GROWTH_FACTOR
        );
    }
}

#[cfg(test)]
mod extracted_actor_root_contract {
    #[test]
    fn document_sequence_root_does_not_reenter_an_extracted_actor_grid() {
        use super::*;
        use d2_rs_core::Node;
        let mut graph = Graph {
            root_sequence_diagram: true,
            ..Graph::default()
        };
        let size = Size {
            width: 80.0,
            height: 60.0,
        };
        let actor = graph.add_node(Node {
            grid_columns: Some(2),
            ..Node::new("a", size)
        });
        let other = graph.add_node(Node::new("b", size));
        let mut children = Vec::new();
        for group in ["cell", "second"] {
            let owner = graph.add_node(Node {
                parent: Some(actor),
                ..Node::new(format!("a.{group}"), size)
            });
            let x = graph.add_node(Node {
                parent: Some(owner),
                ..Node::new(format!("a.{group}.x"), size)
            });
            let y = graph.add_node(Node {
                parent: Some(owner),
                ..Node::new(format!("a.{group}.y"), size)
            });
            let e = graph.add_edge(Edge {
                source: x,
                target: y,
            });
            graph.sequence_group_edges.insert(owner, vec![e]);
            children.extend([owner, x, y]);
        }
        graph.add_edge(Edge {
            source: actor,
            target: other,
        });
        let result = layout_inner(
            &graph,
            &LayoutOptions::default(),
            None,
            EngineKind::Dagre,
            true,
            None,
        )
        .unwrap();
        assert_eq!(result.boxes.len(), graph.nodes().count());
        assert_eq!(result.routes.len(), graph.edges().count());
        for id in [actor, other].into_iter().chain(children) {
            let rect = result.boxes[&id];
            assert!(rect.origin.x.is_finite() && rect.origin.y.is_finite());
            assert!(rect.size.width.is_finite() && rect.size.height.is_finite());
        }
        for route in result.routes.values() {
            assert!(!route.is_empty());
            assert!(route.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
        }
        assert!(
            graph.root_sequence_diagram,
            "the parent document retains its sequence ownership"
        );
    }
}

// Every test here reads a regenerated oracle capture (docs/FIXTURES.md).
#[cfg(all(test, feature = "oracle-fixtures"))]
mod group_label_adjustment_source_contract {
    use super::*;
    use d2_rs_core::Node;

    #[test]
    fn complete_boxes_and_message_routes_match_original_go_targets() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sequence-group-label-adjustment-source-v090.json"
        ))
        .unwrap();
        let note_rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sequence-note-source-order-source-v090.json"
        ))
        .unwrap();
        let lifeline_rows: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!(
                "tests/fixtures/sequence-lifeline-source-order-source-v090.json"
            ))
            .unwrap();
        for row in rows
            .as_array()
            .unwrap()
            .iter()
            .chain(note_rows.as_array().unwrap())
            .chain(lifeline_rows.as_array().unwrap())
        {
            let mut graph = Graph {
                root_sequence_diagram: true,
                ..Default::default()
            };
            let mut ids = BTreeMap::new();
            for shape in row["shapes"].as_array().unwrap() {
                let name = shape["id"].as_str().unwrap();
                let mut node = Node::new(
                    name,
                    Size {
                        width: shape["width"].as_f64().unwrap(),
                        height: shape["height"].as_f64().unwrap(),
                    },
                );
                node.label_size = Some(Size {
                    width: shape["labelWidth"].as_f64().unwrap(),
                    height: shape["labelHeight"].as_f64().unwrap(),
                });
                node.font_size = Some(shape["fontSize"].as_u64().unwrap() as u32);
                node.parent = name.rsplit_once('.').and_then(|(p, _)| ids.get(p).copied());
                let id = graph.add_node(node);
                ids.insert(name.to_owned(), id);
                graph.set_node_label(id, shape["label"].as_str().unwrap());
            }
            let mut edge_ids = Vec::new();
            for connection in row["connections"].as_array().unwrap() {
                // Export includes synthetic lifelines; graph messages do not.
                if connection["dst"]
                    .as_str()
                    .unwrap()
                    .contains("-lifeline-end-")
                {
                    continue;
                }
                let edge = graph.add_edge(Edge {
                    source: ids[connection["src"].as_str().unwrap()],
                    target: ids[connection["dst"].as_str().unwrap()],
                });
                if let Some(text) = connection["label"]
                    .as_str()
                    .filter(|label| !label.is_empty())
                {
                    graph.set_edge_label(
                        edge,
                        Some(d2_rs_core::EdgeLabel {
                            text: text.to_owned(),
                            size: Size {
                                width: connection["labelWidth"].as_f64().unwrap(),
                                height: connection["labelHeight"].as_f64().unwrap(),
                            },
                            position: LabelPosition::Unset,
                            percentage: 0.5,
                        }),
                    );
                }
                edge_ids.push((edge, connection));
            }
            let references = row["reference_edges"].as_array().unwrap();
            assert_eq!(references.len(), edge_ids.len());
            for ((id, connection), reference) in edge_ids.iter().zip(references) {
                assert_eq!(connection["src"], reference["source"]);
                assert_eq!(connection["dst"], reference["target"]);
                for scope in reference["scopes"].as_array().unwrap() {
                    if let Some(owner) = ids.get(scope.as_str().unwrap()) {
                        graph
                            .sequence_group_edges
                            .entry(*owner)
                            .or_default()
                            .push(*id);
                    }
                }
            }
            if let Some(lines) = row.get("lines") {
                for (name, line) in lines["objects"].as_object().unwrap() {
                    graph
                        .sequence_node_lines
                        .insert(ids[name], line.as_i64().unwrap());
                }
                assert_eq!(lines["edges"].as_array().unwrap().len(), edge_ids.len());
                for ((id, connection), line) in
                    edge_ids.iter().zip(lines["edges"].as_array().unwrap())
                {
                    assert_eq!(connection["src"], line["source"]);
                    assert_eq!(connection["dst"], line["target"]);
                    graph
                        .sequence_edge_lines
                        .insert(*id, line["line"].as_i64().unwrap());
                }
            }
            let result = layout(&graph, &LayoutOptions::default()).unwrap();
            let lifelines: Vec<_> = row["connections"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| c["dst"].as_str().unwrap().contains("-lifeline-end-"))
                .collect();
            assert_eq!(result.sequence_lifelines.len(), lifelines.len());
            assert_eq!(
                edge_ids.len() + lifelines.len(),
                row["connections"].as_array().unwrap().len()
            );
            for expected in lifelines {
                let actor = ids[expected["src"].as_str().unwrap()];
                let actual = result
                    .sequence_lifelines
                    .iter()
                    .find(|l| l.actor == actor)
                    .unwrap();
                let route: Vec<_> = expected["route"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| Point {
                        x: p["x"].as_f64().unwrap(),
                        y: p["y"].as_f64().unwrap(),
                    })
                    .collect();
                assert_eq!(actual.route, route, "{} lifeline", row["name"]);
                assert_eq!(actual.target_id, expected["dst"].as_str().unwrap());
            }
            assert_eq!(result.boxes.len(), row["shapes"].as_array().unwrap().len());
            assert_eq!(result.routes.len(), edge_ids.len());
            for shape in row["shapes"].as_array().unwrap() {
                let rect = result.boxes[&ids[shape["id"].as_str().unwrap()]];
                assert_eq!(
                    [
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height
                    ],
                    [
                        shape["pos"]["x"].as_f64().unwrap(),
                        shape["pos"]["y"].as_f64().unwrap(),
                        shape["width"].as_f64().unwrap(),
                        shape["height"].as_f64().unwrap()
                    ],
                    "{} {}",
                    row["name"],
                    shape["id"]
                );
            }
            for (id, connection) in edge_ids {
                let expected: Vec<_> = connection["route"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| Point {
                        x: p["x"].as_f64().unwrap(),
                        y: p["y"].as_f64().unwrap(),
                    })
                    .collect();
                assert_eq!(
                    result.routes[&id], expected,
                    "{} edge {:?}",
                    row["name"], id
                );
            }
        }
    }
}

// Nested LayoutNested validates before InjectNested reconnects the extracted
// root. Its Object.AbsID therefore excludes the enclosing scope prefix.
fn validate_scope_positions(
    graph: &Graph,
    result: &LayoutResult,
    root: Option<NodeId>,
) -> Result<(), LayoutError> {
    let prefix = root
        .and_then(|id| graph.node(id))
        .map(|node| format!("{}.", node.external_id));
    for (id, node) in graph.nodes() {
        if Some(id) == root || !is_descendant_or_self(graph, id, root) {
            continue;
        }
        if let Some(rect) = result.boxes.get(&id)
            && (rect.origin.x.is_infinite() || rect.origin.y.is_infinite())
        {
            let object = prefix
                .as_ref()
                .and_then(|prefix| node.external_id.strip_prefix(prefix))
                .unwrap_or(&node.external_id)
                .to_owned();
            return Err(LayoutError::InvalidPosition {
                object,
                x: rect.origin.x,
                y: rect.origin.y,
            });
        }
    }
    Ok(())
}

// Every test here reads a regenerated oracle capture (docs/FIXTURES.md).
#[cfg(all(test, feature = "oracle-fixtures"))]
mod invalid_position_source_contract {
    use super::*;
    use d2_rs_core::{Edge, Node};

    #[test]
    fn empty_span_and_object_reference_group_preserve_source_coordinates() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sequence-invalid-positions-source-v090.json"
        ))
        .unwrap();
        for empty_span in [false, true] {
            let mut graph = Graph {
                root_sequence_diagram: true,
                ..Default::default()
            };
            let size = Size {
                width: 100.0,
                height: 66.0,
            };
            let a = graph.add_node(Node::new("a", size));
            let b = graph.add_node(Node::new("b", size));
            let phase = graph.add_node(Node::new("phase", size));
            let edge = graph.add_edge(Edge {
                source: a,
                target: b,
            });
            graph.sequence_node_edge_refs.insert(a, true);
            graph.sequence_node_edge_refs.insert(b, true);
            graph.sequence_node_edge_refs.insert(phase, false);
            graph.sequence_object_scopes.insert(a, vec![phase]);
            let index = if empty_span {
                graph.sequence_object_scopes.clear();
                let west = graph.add_node(Node {
                    parent: Some(phase),
                    ..Node::new("phase.west", size)
                });
                let note = graph.add_node(Node {
                    parent: Some(phase),
                    ..Node::new(
                        "phase.note",
                        Size {
                            width: 75.0,
                            height: 66.0,
                        },
                    )
                });
                graph.sequence_node_edge_refs.insert(west, false);
                graph.sequence_node_edge_refs.insert(note, false);
                graph.sequence_object_scopes.insert(west, vec![phase]);
                graph.sequence_object_scopes.insert(note, vec![phase]);
                graph.sequence_note_scopes.insert(note, vec![phase]);
                let owned = graph.add_edge(Edge {
                    source: a,
                    target: b,
                });
                graph.sequence_group_edges.insert(west, vec![owned]);
                graph.sequence_group_edges.insert(phase, vec![owned]);
                graph.sequence_edge_lines.insert(edge, 1);
                graph.sequence_edge_lines.insert(owned, 2);
                graph.sequence_node_lines.insert(note, 2);
                2
            } else {
                0
            };
            let result = layout(&graph, &LayoutOptions::default()).unwrap();
            let expected = &fixture["cases"][index];
            for position in expected["positions"].as_array().unwrap() {
                let id = graph
                    .nodes()
                    .find(|(_, n)| n.external_id == position["id"].as_str().unwrap())
                    .unwrap()
                    .0;
                let rect = result.boxes[&id];
                for (key, actual) in [
                    ("x", rect.origin.x),
                    ("y", rect.origin.y),
                    ("width", rect.size.width),
                    ("height", rect.size.height),
                ] {
                    let actual = if actual == f64::INFINITY {
                        "+Inf".to_owned()
                    } else if actual == f64::NEG_INFINITY {
                        "-Inf".to_owned()
                    } else {
                        format!("{actual:.6}")
                    };
                    assert_eq!(
                        actual,
                        position[key].as_str().unwrap(),
                        "{} {key}",
                        position["id"]
                    );
                }
            }
            assert_eq!(
                validate_scope_positions(&graph, &result, None)
                    .unwrap_err()
                    .to_string(),
                expected["error"].as_str().unwrap()
            );
        }
    }
}
