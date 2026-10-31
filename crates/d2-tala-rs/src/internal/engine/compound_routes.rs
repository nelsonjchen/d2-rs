// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Translation of upstream D2 `engine/compound_routes.go` (b34d2e936,
//! "tala: preserve enclosed routes when compound containers move").
//!
//! After the rigid-container compound proposal has replaced the ordinary
//! seed winner, routes that lie wholly inside one moved outer block are
//! translated back from the ordinary drawing instead of keeping the
//! proposal's independent reroute. Placement is never changed here.

use super::*;

#[cfg(test)]
#[path = "interior_api_tests.rs"]
pub(super) mod interior_api_tests;

/// `PreserveCompoundRoutes(ctx, before, selected)`: refine an already-selected
/// compound drawing. `before` is the ordinary seed winner's graph.
pub(super) fn preserve_compound_routes(
    input: &Graph,
    before: &ArenaGraph,
    selected: &ArenaGraph,
    cancellation: Option<&CancellationToken>,
) -> Result<ArenaGraph, LayoutError> {
    with_cancellation(cancellation, || {
        check_cancellation()?;
        // layoutgraph.Clone(selected): fresh cost cache and cleared fixed
        // label/icon bookkeeping, exactly as for the compound proposal.
        let mut candidate = selected.clone();
        snapshot::prepare_compound_candidate_state(&mut candidate);
        // Clone does not carry IsCurve; the source restores it from the
        // selected graph. The arena clone already retains it, so this copy is
        // explicit only to keep the edge pairing shape of the source.
        for (after_index, selected_index) in candidate
            .member_edge_indices()
            .zip(selected.member_edge_indices())
        {
            candidate.edge_records[after_index].is_curve =
                selected.edge_records[selected_index].is_curve;
        }
        preserve_compound_interiors(input, before, candidate)
    })
}

fn guard_error(error: limits::Error) -> LayoutError {
    match error {
        limits::Error::Context {
            cause: limits::ContextError::Cancelled,
            ..
        } => LayoutError::Cancelled,
        limits::Error::Context {
            cause: limits::ContextError::DeadlineExceeded,
            ..
        } => LayoutError::DeadlineExceeded,
        limits::Error::SeedInvariant(_) => LayoutError::SeedInvariant,
        error => LayoutError::EngineValidation(error.to_string()),
    }
}

/// `preserveCompoundInteriors(ctx, before, after)`.
fn preserve_compound_interiors(
    input: &Graph,
    before: &ArenaGraph,
    after: ArenaGraph,
) -> Result<ArenaGraph, LayoutError> {
    let context = gap_guard::ActiveContext::new();
    let mut guard =
        limits::WorkGuard::new(Some(&context), "CompoundRoutes", limits::MAX_ENGINE_WORK_UNITS)
            .map_err(guard_error)?;
    let mut pipeline = Pipeline::new(input, 0, true, true);
    pipeline.graph = after;
    pipeline.suppress_route_trace = true;
    restore_compound_interiors(before, &mut pipeline.graph, &mut guard)?;
    pipeline.emit_normalized_trace("CompoundRoutes/PreserveInteriors");
    // after.ResetPlacementCosts()
    pipeline.graph.cost_cache.reset_placement();
    // The selected external routes can use a previously unused side of an
    // interior node: labeling.Place reconsiders automatic labels and icons.
    pipeline.run_place_labels()?;
    pipeline.emit_normalized_trace("CompoundRoutes/PlaceLabels");
    // placement.Normalize(after); CompoundCandidate already excluded graphs
    // with fixed nodes, so this is the same call as its reroutePlaced tail.
    pipeline.graph.normalize_seed()?;
    pipeline.emit_normalized_trace("CompoundRoutes/Normalize");
    guard.finish().map_err(guard_error)?;
    Ok(pipeline.graph)
}

/// The node/edge restoration loops of `preserveCompoundInteriors`, before
/// cost reset, label placement and normalization.
pub(super) fn restore_compound_interiors(
    before: &ArenaGraph,
    after: &mut ArenaGraph,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<(), LayoutError> {
    // Clone resets placement bookkeeping for a new full layout. This trial
    // instead retains the authored label and icon constraints of placed nodes.
    for (node, original) in after.nodes.iter_mut().zip(before.nodes.iter()) {
        if original.label_position_fixed {
            node.label_position = original.label_position;
            node.label_position_fixed = true;
        }
        if original.icon_position_fixed {
            node.icon_position = original.icon_position;
            node.icon_position_fixed = true;
        }
    }
    // Clone and PlaceCompound preserve edge order: pair Graph.Edges by index.
    let after_edges = after.member_edge_indices().collect::<Vec<_>>();
    let before_edges = before.member_edge_indices().collect::<Vec<_>>();
    if after_edges.len() > before_edges.len() {
        // before.Edges[index] would be out of range in the source.
        return Err(LayoutError::SeedInvariant);
    }
    for (after_index, before_index) in after_edges.into_iter().zip(before_edges) {
        guard.check_context().map_err(guard_error)?;
        let original = &before.edge_records[before_index];
        if original.label_position_fixed && original.label.is_some() {
            let edge = &mut after.edge_records[after_index];
            edge.label = original.label.clone();
            edge.label_position_fixed = true;
        }
        if !compound_enclosed_route(before, original, guard)? {
            continue;
        }
        let old_root = compound_root(before, original.from);
        let new_root = compound_root(after, after.edge_records[after_index].from);
        let (Some(old_top_left), Some(new_top_left)) =
            (before.position(old_root), after.position(new_root))
        else {
            return Err(LayoutError::SeedInvariant);
        };
        let dx = new_top_left.x - old_top_left.x;
        let dy = new_top_left.y - old_top_left.y;
        let mut points = Vec::with_capacity(original.route.len());
        for point in original.route.read().iter() {
            guard.step().map_err(guard_error)?;
            points.push(Point {
                x: point.x + dx,
                y: point.y + dy,
            });
        }
        let edge = &mut after.edge_records[after_index];
        edge.route.replace(RoutePoints::literal(&points));
        edge.is_curve = original.is_curve;
        // LabelPercentage lives on the Rust label record; copying the whole
        // label covers both source assignments.
        if original.label.is_some() {
            edge.label = original.label.clone();
        }
    }
    Ok(())
}

/// `compoundRoot`: the outermost container of node.
fn compound_root(graph: &ArenaGraph, mut node: NodeId) -> NodeId {
    while let Some(parent) = graph.nodes[node.0 as usize].container {
        node = parent;
    }
    node
}

/// `compoundEnclosedRoute(edge, guard)`.
fn compound_enclosed_route(
    graph: &ArenaGraph,
    edge: &ArenaEdge,
    guard: &mut limits::WorkGuard<'_>,
) -> Result<bool, LayoutError> {
    let root = compound_root(graph, edge.from);
    if !graph.nodes[root.0 as usize].is_container
        || root != compound_root(graph, edge.to)
        || edge.route.len() < 2
    {
        return Ok(false);
    }
    let Some(root_top_left) = graph.position(root) else {
        return Err(LayoutError::SeedInvariant);
    };
    let root_size = graph.nodes[root.0 as usize].rect.size;
    let inside = |point: Option<Point>, width: f64, height: f64| {
        point.is_some_and(|point| {
            !point.x.is_nan()
                && !point.y.is_nan()
                && point.x >= root_top_left.x
                && point.y >= root_top_left.y
                && point.x + width <= root_top_left.x + root_size.width
                && point.y + height <= root_top_left.y + root_size.height
        })
    };
    for index in 0..edge.route.len() {
        guard.step().map_err(guard_error)?;
        if !inside(edge.route.slot(index).map(|point| point.get()), 0.0, 0.0) {
            return Ok(false);
        }
    }
    let route = edge.route.read();
    // Conservatively reuse only endpoints on box borders. Shape-specific
    // ports recessed inside an oval or other outline can be rebuilt.
    let attached = |node: NodeId, point: Point| -> Result<bool, LayoutError> {
        let Some(top_left) = graph.position(node) else {
            return Err(LayoutError::SeedInvariant);
        };
        let size = graph.nodes[node.0 as usize].rect.size;
        // Node.ContainsPointOnBox (isPointOnNode).
        let on_box = top_left.x <= point.x
            && top_left.x + size.width >= point.x
            && top_left.y <= point.y
            && top_left.y + size.height >= point.y;
        Ok(on_box
            && (point.x == top_left.x
                || point.x == top_left.x + size.width
                || point.y == top_left.y
                || point.y == top_left.y + size.height))
    };
    if !attached(edge.from, route[0])? || !attached(edge.to, route[route.len() - 1])? {
        return Ok(false);
    }
    if let Some(label) = edge.label.as_ref() {
        if label.position == LabelPosition::Unset {
            return Ok(false);
        }
        guard.add(route.len() as i64).map_err(guard_error)?;
        let top_left = labels::edge_label_top_left(
            edge,
            label.position,
            label.percentage,
            label.size.width,
            label.size.height,
        );
        if !inside(top_left, label.size.width, label.size.height) {
            return Ok(false);
        }
    }
    for (is_target, arrow_label) in [
        (false, edge.source_arrowhead_label.as_ref()),
        (true, edge.target_arrowhead_label.as_ref()),
    ] {
        let Some(arrow_label) = arrow_label else {
            continue;
        };
        guard.add(route.len() as i64).map_err(guard_error)?;
        // labelgeom.ArrowheadTopLeft: the renderer-compatible arrowhead
        // label position used by labeling.positionedArrowheadLabel.
        let top_left = labels::arrowhead_label_rect_for_route(edge, &route, is_target)
            .map(|rect| rect.origin);
        if !inside(top_left, arrow_label.size.width, arrow_label.size.height) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: Point = Point { x: 400.0, y: 0.0 };
    const DELTA: Point = Point { x: -300.0, y: 120.0 };

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    /// The bus branch of upstream `newCompoundInteriorFixture`: a routed,
    /// labelled interior edge wholly inside the rigid middle block.
    fn fixture(route: &[Point], label: Option<(f64, f64)>) -> (ArenaGraph, ArenaGraph) {
        let mut input = Graph::default();
        let mut add = |id: &str, parent: Option<NodeId>, width: f64, height: f64| {
            let mut node = crate::Node::new(id, Size { width, height });
            node.parent = parent;
            input.add_node(node)
        };
        let block = add("block", None, 900.0, 700.0);
        let hub = add("hub", Some(block), 80.0, 60.0);
        let upper = add("upper", Some(block), 100.0, 70.0);
        input.add_edge(crate::Edge {
            source: hub,
            target: upper,
        });
        let mut before = ArenaGraph::from_input(&input);
        for (node, top_left) in [
            (block, BLOCK),
            (hub, point(460.0, 200.0)),
            (upper, point(850.0, 100.0)),
        ] {
            before.set_position(node, top_left);
        }
        let edge = &mut before.edge_records[0];
        edge.route.replace(RoutePoints::literal(route));
        edge.label = label.map(|(percentage, height)| crate::EdgeLabel {
            text: "upper branch".into(),
            size: Size {
                width: 72.0,
                height,
            },
            position: LabelPosition::UnlockedTop,
            percentage,
        });
        // The selected proposal moved the block rigidly and rerouted it.
        let mut after = before.clone();
        snapshot::prepare_compound_candidate_state(&mut after);
        for node in [block, hub, upper] {
            let top_left = before.position(node).unwrap();
            after.set_position(
                node,
                point(top_left.x + DELTA.x, top_left.y + DELTA.y),
            );
        }
        after.edge_records[0]
            .route
            .replace(RoutePoints::literal(&[point(240.0, 350.0), point(550.0, 255.0)]));
        if let Some(label) = after.edge_records[0].label.as_mut() {
            label.percentage = 0.0;
        }
        (before, after)
    }

    fn restore(before: &ArenaGraph, after: &mut ArenaGraph) {
        let context = gap_guard::ActiveContext::new();
        let mut guard =
            limits::WorkGuard::new(Some(&context), "CompoundRoutes", limits::MAX_ENGINE_WORK_UNITS)
                .unwrap();
        restore_compound_interiors(before, after, &mut guard).unwrap();
    }

    const BUS: [Point; 4] = [
        Point { x: 540.0, y: 230.0 },
        Point { x: 750.0, y: 230.0 },
        Point { x: 750.0, y: 135.0 },
        Point { x: 850.0, y: 135.0 },
    ];

    #[test]
    fn enclosed_interior_route_and_label_translate_with_block() {
        let (before, mut after) = fixture(&BUS, Some((0.25, 16.0)));
        restore(&before, &mut after);
        let translated = BUS
            .iter()
            .map(|p| point(p.x + DELTA.x, p.y + DELTA.y))
            .collect::<Vec<_>>();
        assert_eq!(after.edge_records[0].route.read().clone(), translated);
        assert_eq!(after.edge_records[0].label.as_ref().unwrap().percentage, 0.25);
    }

    #[test]
    fn unsafe_interior_keeps_selected_route() {
        let selected = vec![point(240.0, 350.0), point(550.0, 255.0)];
        let cases: [(&str, Vec<Point>, Option<(f64, f64)>); 5] = [
            ("incomplete", BUS[..1].to_vec(), Some((0.25, 16.0))),
            // e.Points[0] = e.From.Center()
            (
                "detached",
                [&[point(500.0, 230.0)], &BUS[1..]].concat(),
                Some((0.25, 16.0)),
            ),
            (
                "escaped",
                vec![BUS[0], point(750.0, -180.0), point(850.0, -180.0), BUS[3]],
                Some((0.25, 16.0)),
            ),
            (
                "subpixel-escape",
                vec![BUS[0], point(750.0, -0.1), point(850.0, -0.1), BUS[3]],
                None,
            ),
            (
                "escaped-label",
                vec![
                    BUS[0],
                    point(700.0, 230.0),
                    point(700.0, 8.0),
                    point(850.0, 8.0),
                    BUS[3],
                ],
                Some((457.0 / 659.0, 16.0)),
            ),
        ];
        for (name, route, label) in cases {
            let (before, mut after) = fixture(&route, label);
            restore(&before, &mut after);
            assert_eq!(
                after.edge_records[0].route.read().clone(),
                selected,
                "{name}: unsafe route was reused by rigid translation"
            );
        }
    }
}
