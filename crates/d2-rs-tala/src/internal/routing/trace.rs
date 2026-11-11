// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Final clipping of route rays to exact rendered shape outlines.
//!
//! Shape-specific intersections replace box/port working endpoints, including
//! D2 rounding behavior and 3D or multiple-object modifiers.

use super::route_search_guard::{MAX_ROUTE_STAGE_WORK_UNITS, RouteStageWork};
use super::route_stage_guard::{self, RouteAtomicGuard, RouteStageError, RuntimeRouteContext};
use super::*;
use crate::EdgeId;
use crate::internal::owner_points::SharedOwnerPoint;
use crate::internal::{OwnerGraphId, bounds::modifier_element_adjustments, limits};

use d2_rs_core::shape_geometry::{ShapeBorderTrace, trace_to_shape_border};

fn line_intersection_point(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    d2_rs_core::shape_geometry::line_intersection_point(a, b, c, d)
}

fn actual_rect(graph: &ArenaGraph, node: NodeId) -> Rect {
    Rect {
        origin: graph.position(node).unwrap_or_else(|| {
            std::panic::panic_any(
                "runtime error: invalid memory address or nil pointer dereference",
            )
        }),
        size: graph.nodes[node.0 as usize].rect.size,
    }
}
// The source searches Top, Right, Left, Bottom, independently of port storage order.
fn port_orientation(graph: &ArenaGraph, node: NodeId, point: Point) -> Option<PortSide> {
    let n = &graph.nodes[node.0 as usize];
    let ports =
        ports_with_table_column_count(actual_rect(graph, node), n.shape, n.table_column_count);
    [
        PortSide::Top,
        PortSide::Right,
        PortSide::Left,
        PortSide::Bottom,
    ]
    .into_iter()
    .find(|&side| ports.iter().any(|p| p.side == side && p.point == point))
}
fn modifier_endpoint(
    graph: &ArenaGraph,
    node: NodeId,
    endpoint: &SharedOwnerPoint,
    previous: &SharedOwnerPoint,
    originals: &mut Vec<(SharedOwnerPoint, Point)>,
) {
    let (dx, dy) = modifier_element_adjustments(&graph.nodes[node.0 as usize]);
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    let top_left = graph
        .owner_node_point(node)
        .unwrap()
        .shared_point()
        .unwrap_or_else(|| {
            std::panic::panic_any(
                "runtime error: invalid memory address or nil pointer dereference",
            )
        });
    let r = actual_rect(graph, node);
    let start = endpoint.get();
    if !(start.x > r.origin.x + dx && start.y < r.origin.y + r.size.height - dy) {
        return;
    }
    let original = top_left.get();
    originals.push((top_left.clone(), original));
    let orientation = port_orientation(graph, node, endpoint.get());
    let candidate =
        if orientation == Some(PortSide::Right) || endpoint.get().x == original.x + r.size.width {
            let mut p = top_left.get();
            p.x += dx;
            top_left.set(p);
            let top_right = Point {
                x: top_left.get().x + r.size.width,
                y: top_left.get().y,
            };
            let bottom_right = Point {
                x: top_right.x,
                y: top_right.y + r.size.height,
            };
            line_intersection_point(endpoint.get(), previous.get(), top_right, bottom_right)
        } else if orientation == Some(PortSide::Top) || endpoint.get().y == original.y {
            let mut p = top_left.get();
            p.y -= dy;
            top_left.set(p);
            let top_right = Point {
                x: top_left.get().x + r.size.width,
                y: top_left.get().y,
            };
            line_intersection_point(endpoint.get(), previous.get(), top_left.get(), top_right)
        } else {
            None
        };
    if let Some(point) = candidate {
        endpoint.set(point);
    }
}
fn trace_endpoint(graph: &ArenaGraph, edge: EdgeId, node: NodeId, slot: usize, previous: usize) {
    let route = &graph.edge_records[edge.0 as usize].route;
    let point = route.slot(slot).unwrap();
    let before = point.get();
    // shape.TraceToShapeBorder returns rectangular border points unchanged
    // and truncates the shared border point in place when the ray misses.
    let ShapeBorderTrace::Traced {
        point: traced,
        intersected,
    } = trace_to_shape_border(
        graph.nodes[node.0 as usize].shape,
        actual_rect(graph, node),
        before,
        route.point(previous),
    )
    else {
        return;
    };
    if !intersected {
        point.set(Point {
            x: before.x as f32 as f64,
            y: before.y as f32 as f64,
        });
    }
    route.replace_point(slot, traced);
}
// trace.go: preserve shared TopLeft/route points and reverse modifier restoration.
fn trace_edge(graph: &mut ArenaGraph, edge: EdgeId) {
    let e = &graph.edge_records[edge.0 as usize];
    if e.route.len() < 2 {
        return;
    }
    let (from, to, last) = (e.from, e.to, e.route.len() - 1);
    let mut materialized = Vec::new();
    for node in [from, to] {
        if graph.materialize_owner_node_point(node).unwrap() {
            materialized.push(node);
        }
    }
    let mut originals = Vec::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let route = &graph.edge_records[edge.0 as usize].route;
        modifier_endpoint(
            graph,
            from,
            &route.slot(0).unwrap(),
            &route.slot(1).unwrap(),
            &mut originals,
        );
        modifier_endpoint(
            graph,
            to,
            &route.slot(last).unwrap(),
            &route.slot(last - 1).unwrap(),
            &mut originals,
        );
        trace_endpoint(graph, edge, from, 0, 1);
        trace_endpoint(graph, edge, to, last, last - 1);
    }));
    while let Some((point, value)) = originals.pop() {
        point.set(value);
    }
    for node in materialized {
        graph.release_materialized_owner_node_point(node);
    }
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}
pub(super) fn trace_edge_guarded(
    graph: &mut ArenaGraph,
    edge: EdgeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<(), RouteStageError> {
    guard.step()?;
    let e = &graph.edge_records[edge.0 as usize];
    if e.route.len() < 2 {
        return Ok(());
    }
    // Public atomic snapshot restores all captured identities on panic; the
    // private route helper additionally restores each endpoint's TopLeft.
    let nodes = [e.from, e.to];
    let mut materialized = Vec::new();
    let mut snapshots = Vec::new();
    for node in nodes {
        if graph.materialize_owner_node_point(node).unwrap() {
            materialized.push(node);
        }
    }
    for node in nodes {
        snapshots.push((node, graph.owner_node_point(node).unwrap().snapshot()));
    }
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| trace_edge(graph, edge)));
    if outcome.is_err() {
        for (node, snapshot) in snapshots {
            graph.restore_owner_node_point(node, &snapshot).unwrap();
        }
    }
    for node in materialized {
        graph.release_materialized_owner_node_point(node);
    }
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
    guard.step()
}
pub(super) fn trace_with_context(
    graph: &mut ArenaGraph,
    context: Option<&dyn limits::Context>,
    budget: &RouteStageWork,
) -> Result<(), RouteStageError> {
    route_stage_guard::run_atomic_route_stage_typed(
        graph,
        Some(OwnerGraphId(0)),
        context,
        "TraceEdgesToShapeBorder",
        &[],
        budget,
        |g, guard| {
            for edge in super::nudge::edges(g) {
                trace_edge_guarded(g, edge, guard)?;
            }
            Ok(())
        },
    )
}
/// Source atomic TraceEdgesToShapeBorder, retaining typed errors and work order.
pub(in crate::internal) fn trace_edges_to_shape_border_with_budget(
    graph: &mut ArenaGraph,
    budget: &RouteStageWork,
) -> Result<(), RouteStageError> {
    trace_with_context(graph, Some(&RuntimeRouteContext), budget)
}
pub(in crate::internal) fn trace_edges_to_shape_border(graph: &mut ArenaGraph) {
    let _ = trace_edges_to_shape_border_with_budget(
        graph,
        &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, Size};

    #[test]
    fn modifier_loop_traces_both_ends_against_the_same_shifted_node() {
        let mut input = Graph::default();
        let mut node = Node::new(
            "hexagon",
            Size {
                width: 100.0,
                height: 80.0,
            },
        );
        node.shape = ShapeKind::Hexagon;
        node.is_3d = true;
        let id = input.add_node(node);
        input.set_initial_position(id, Some(Point::default()));
        let edge = input.add_edge(Edge {
            source: id,
            target: id,
        });
        input.set_edge_route(
            edge,
            vec![
                Point { x: 13.0, y: 20.0 },
                Point { x: -17.0, y: 20.0 },
                Point { x: -17.0, y: -30.0 },
                Point { x: 25.0, y: -30.0 },
                Point { x: 25.0, y: 0.0 },
            ],
        );
        let mut graph = ArenaGraph::from_input(&input);
        trace_edges_to_shape_border(&mut graph);
        assert_eq!(
            graph.edge_records[edge.0 as usize].route.point(0),
            Point { x: 7.0, y: 20.0 }
        );
        assert_eq!(graph.position(id), Some(Point::default()));
        assert_eq!(graph.nodes[id.0 as usize].rect.origin, Point::default());
    }

    #[test]
    fn segment_intersection_rounds_displacement_before_origin() {
        let intersection = line_intersection_point(
            Point { x: 0.4, y: 0.0 },
            Point { x: 1.4, y: 1.0 },
            Point { x: 0.0, y: 0.4 },
            Point { x: 2.0, y: 0.4 },
        )
        .unwrap();

        assert_eq!(intersection, Point { x: 0.4, y: 0.0 });
    }

    #[test]
    fn short_nonzero_shape_trace_ray_is_extended_with_exact_axis_selection() {
        let mut input = Graph::default();
        let mut from = Node::new(
            "from",
            Size {
                width: 100.0,
                height: 10.0,
            },
        );
        from.shape = ShapeKind::Diamond;
        let from_id = input.add_node(from);
        let to_id = input.add_node(Node::new(
            "to",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.set_initial_position(from_id, Some(Point { x: 0.0, y: 0.0 }));
        input.set_initial_position(to_id, Some(Point { x: 200.0, y: 0.0 }));
        let edge = input.add_edge(Edge {
            source: from_id,
            target: to_id,
        });
        input.set_edge_route(
            edge,
            vec![
                Point { x: 0.0, y: 0.0 },
                Point {
                    x: -5.0e-10,
                    y: -1.0e-10,
                },
            ],
        );
        let mut graph = ArenaGraph::from_input(&input);
        trace_edges_to_shape_border(&mut graph);

        assert_eq!(
            graph.edge_records[edge.0 as usize].route.point(0),
            Point { x: 17.0, y: 3.0 }
        );
    }

    fn source_trace_fixture(shape: &str, modifier: bool) -> ArenaGraph {
        let value = serde_json::json!({"nodes":[{"id":1,"topLeft":{"x":0,"y":0},"width":100,"height":100,"shapeType":shape,"is3D":modifier},{"id":2,"topLeft":{"x":300,"y":0},"width":100,"height":100,"shapeType":shape,"is3D":modifier}],"edges":[{"id":11,"from":1,"to":2,"points":[{"x":100,"y":0},{"x":300,"y":100}]}],"containers":{"0":[1,2]}});
        ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&value).unwrap()).unwrap(),
        )
    }
    #[test]
    fn shape_trace_rectangles_preserve_endpoint_allocations() {
        let mut g = source_trace_fixture("Square", false);
        let header = g.edge_records[0].route.clone();
        let first = header.slot(0).unwrap();
        let last = header.slot(1).unwrap();
        let work = RouteStageWork::new(10000);
        trace_edges_to_shape_border_with_budget(&mut g, &work).unwrap();
        assert_eq!(work.used(), 20);
        assert!(
            header
                .header()
                .same_backing_allocation(&g.edge_records[0].route.header())
        );
        assert!(first.same_allocation(&header.slot(0).unwrap()));
        assert!(last.same_allocation(&header.slot(1).unwrap()));
    }
    #[test]
    fn shape_trace_modifiers_mutate_old_points_before_slot_replacement() {
        let mut g = source_trace_fixture("Circle", true);
        let old = g.edge_records[0].route.slot(0).unwrap();
        let header = g.edge_records[0].route.clone();
        trace_edges_to_shape_border_with_budget(&mut g, &RouteStageWork::new(10000)).unwrap();
        assert_eq!(old.get(), Point { x: 115.0, y: 8.0 });
        assert!(!old.same_allocation(&g.edge_records[0].route.slot(0).unwrap()));
        assert!(
            header
                .header()
                .same_backing_allocation(&g.edge_records[0].route.header())
        );
        assert_eq!(g.position(NodeId(0)), Some(Point { x: 0.0, y: 0.0 }));
    }
    #[test]
    fn shape_trace_no_intersection_truncates_the_retained_fallback_point() {
        let mut g = source_trace_fixture("Circle", false);
        let value = Point {
            x: 20.123456789,
            y: 10.987654321,
        };
        g.edge_records[0]
            .route
            .replace(RoutePoints::literal(&[value, value]));
        let old = g.edge_records[0].route.slot(0).unwrap();
        trace_edges_to_shape_border_with_budget(&mut g, &RouteStageWork::new(10000)).unwrap();
        assert_eq!(
            old.get(),
            Point {
                x: value.x as f32 as f64,
                y: value.y as f32 as f64
            }
        );
        assert_eq!(g.edge_records[0].route.point(0), Point { x: 20.0, y: 11.0 });
        assert!(!old.same_allocation(&g.edge_records[0].route.slot(0).unwrap()));
    }
    #[test]
    fn shape_trace_every_work_limit_restores_slots_and_mutated_points() {
        for limit in 0..20 {
            let mut g = source_trace_fixture("Circle", true);
            let header = g.edge_records[0].route.clone();
            let points = [header.slot(0).unwrap(), header.slot(1).unwrap()];
            let values = points.each_ref().map(|p| p.get());
            let error =
                trace_edges_to_shape_border_with_budget(&mut g, &RouteStageWork::new(limit))
                    .unwrap_err();
            assert!(error.is_route_work_limit(), "limit {limit}: {error}");
            assert!(
                header
                    .header()
                    .same_backing_allocation(&g.edge_records[0].route.header())
            );
            for i in 0..2 {
                assert_eq!(points[i].get(), values[i]);
                assert!(points[i].same_allocation(&g.edge_records[0].route.slot(i).unwrap()));
            }
        }
    }
    #[test]
    fn shape_trace_final_cancellation_restores_original_route_points() {
        struct Cancel(std::cell::Cell<usize>);
        impl limits::Context for Cancel {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                let n = self.0.get() + 1;
                self.0.set(n);
                (n == 11).then_some(limits::ContextError::Cancelled)
            }
        }
        let mut g = source_trace_fixture("Circle", true);
        let old = g.edge_records[0].route.slot(0).unwrap();
        let context = Cancel(std::cell::Cell::new(0));
        let error =
            trace_with_context(&mut g, Some(&context), &RouteStageWork::new(10000)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "TraceEdgesToShapeBorder: context canceled"
        );
        assert_eq!(context.0.get(), 11);
        assert!(old.same_allocation(&g.edge_records[0].route.slot(0).unwrap()));
        assert_eq!(old.get(), Point { x: 100.0, y: 0.0 });
    }
    #[test]
    fn shape_trace_cylinder_preserves_source_negative_zero() {
        let mut g = source_trace_fixture("Cylinder", false);
        g.edge_records[0].route.replace(RoutePoints::literal(&[
            Point { x: 50.0, y: 0.0 },
            Point { x: 50.0, y: -100.0 },
        ]));
        trace_edges_to_shape_border_with_budget(&mut g, &RouteStageWork::new(10000)).unwrap();
        assert_eq!(
            g.edge_records[0].route.point(0).y.to_bits(),
            (-0.0_f64).to_bits()
        );
    }
    #[test]
    fn shape_trace_large_circle_follows_go_architecture_fusion() {
        let mut g = source_trace_fixture("Circle", false);
        for node in &mut g.nodes {
            node.rect.size = Size {
                width: 1e20,
                height: 1e20,
            };
        }
        trace_edges_to_shape_border_with_budget(&mut g, &RouteStageWork::new(10000)).unwrap();
        // Go fuses the ellipse multiply-adds on arm64 only; amd64 rounds
        // each product and misses the huge circle (core pins both builds).
        let expected = if cfg!(target_arch = "aarch64") {
            0x43f158e460000000
        } else {
            300.0f64.to_bits()
        };
        assert_eq!(g.edge_records[0].route.point(1).x.to_bits(), expected);
    }
}
