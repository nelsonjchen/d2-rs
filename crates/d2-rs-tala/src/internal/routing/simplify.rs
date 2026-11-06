// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Removal of redundant bends and replaceable detours from routed polylines.
//!
//! Simplification retains endpoint ownership and accepts shortcuts only when
//! their replacement geometry remains legal in the active graph.

use super::*;

fn stage_charge(
    budget: Option<&dyn super::route_stage_guard::RouteBudget>,
    units: u64,
) -> Result<(), ()> {
    let Some(budget) = budget else {
        return Ok(());
    };
    budget.reserve(units)
}

pub(super) fn simplify_once_with_budget(
    graph: &ArenaGraph,
    edge_index: usize,
    budget: Option<&dyn super::route_stage_guard::RouteBudget>,
) -> Result<RoutePoints, ()> {
    let edge = &graph.edge_records[edge_index];
    let points = &edge.route;
    if points.len() < 5 {
        stage_charge(budget, 1)?;
        return Ok(points.clone());
    }

    let mut result = crate::internal::route_points::LocalRouteBuilder::new(points.slot(0));
    let mut index = 0;
    while index < points.len() - 4 {
        stage_charge(budget, 1)?;
        let [p1, p2, p3, p4, p5] = std::array::from_fn(|offset| points.point(index + offset));
        let vector1 = Point {
            x: p2.x - p1.x,
            y: p2.y - p1.y,
        };
        let vector2 = Point {
            x: p3.x - p2.x,
            y: p3.y - p2.y,
        };
        let vector3 = Point {
            x: p4.x - p3.x,
            y: p4.y - p3.y,
        };
        let vector4 = Point {
            x: p5.x - p4.x,
            y: p5.y - p4.y,
        };
        let horizontal = |a: Point, b: Point| a.y == b.y;
        let vertical = |a: Point, b: Point| a.x == b.x;
        let opposite_verticals = vertical(p2, p3)
            && vertical(p4, p5)
            && ((vector2.y > 0.0 && vector4.y < 0.0) || (vector2.y < 0.0 && vector4.y > 0.0));
        let same_horizontals = horizontal(p1, p2)
            && horizontal(p3, p4)
            && ((vector1.x > 0.0 && vector3.x > 0.0) || (vector1.x < 0.0 && vector3.x < 0.0));
        let opposite_horizontals = horizontal(p2, p3)
            && horizontal(p4, p5)
            && ((vector2.x > 0.0 && vector4.x < 0.0) || (vector2.x < 0.0 && vector4.x > 0.0));
        let same_verticals = vertical(p1, p2)
            && vertical(p3, p4)
            && ((vector1.y > 0.0 && vector3.y > 0.0) || (vector1.y < 0.0 && vector3.y < 0.0));
        let pattern =
            (opposite_verticals && same_horizontals) || (opposite_horizontals && same_verticals);

        let intersection = if horizontal(p1, p2) && vertical(p4, p5) {
            Some(Point { x: p4.x, y: p1.y })
        } else if vertical(p1, p2) && horizontal(p4, p5) {
            Some(Point { x: p1.x, y: p4.y })
        } else {
            None
        };
        let unobstructed = if pattern {
            if let Some(intersection) = intersection {
                // The reference keeps a route's endpoint approach direction
                // when a shortcut would reverse a port ray.  This is easy to
                // miss because the replacement still looks clear in the
                // bounding-box test; labels and non-rectangular shapes make
                // the reversal observable in the final route.
                let source = graph.edge_records[edge_index].from;
                let target = graph.edge_records[edge_index].to;
                let source_rect = graph.active_node_position(source).map(|origin| Rect {
                    origin,
                    size: graph.active_node_size(source),
                });
                let target_rect = graph.active_node_position(target).map(|origin| Rect {
                    origin,
                    size: graph.active_node_size(target),
                });
                let source_direction_ok = !(index == 0
                    && source_rect.is_some_and(|rect| contains_with_delta(rect, p1, 1.0))
                    && !same_route_direction(p1, p2, p1, intersection));
                let target_direction_ok = !(index + 4 == points.len() - 1
                    && target_rect.is_some_and(|rect| contains_with_delta(rect, p5, 1.0))
                    && !same_route_direction(p4, p5, intersection, p5));
                let segment_clear = |start: Point, end: Point| -> Result<bool, ()> {
                    let node_order = graph.graph_node_order();
                    let mut node_clear = true;
                    for node_id in node_order {
                        stage_charge(budget, 1)?;
                        let Some(origin) = graph.active_node_position(node_id) else {
                            continue;
                        };
                        let size = graph.active_node_size(node_id);
                        if size.width <= 0.0 || size.height <= 0.0 {
                            continue;
                        }
                        if node_id != source
                            && node_id != target
                            && (graph.active_is_descendant_of(source, node_id)
                                || graph.active_is_descendant_of(target, node_id))
                        {
                            continue;
                        }
                        if orthogonal_segment_enters_node(Rect { origin, size }, start, end) {
                            node_clear = false;
                            break;
                        }
                    }
                    let mut edges_clear = true;
                    'edges: for edge_id in &graph.edge_order.read() {
                        let other_index = edge_id.0 as usize;
                        let other = &graph.edge_records[other_index];
                        stage_charge(budget, 1)?;
                        if other_index == edge_index || other.route.len() < 2 {
                            continue;
                        }
                        for index in 0..other.route.len() - 1 {
                            stage_charge(budget, 1)?;
                            let segment = [other.route.point(index), other.route.point(index + 1)];
                            let clear = !segments_intersect(start, end, segment[0], segment[1]);
                            #[cfg(feature = "diagnostic-traces")]
                            if !clear && crate::internal::trace_env_enabled("TALA_TRACE_SIMPLIFY") {
                                eprintln!(
                                    "SIMPLIFY_BLOCK_EDGE_RUST edge={} other={} leg={:?}->{:?} other_seg={:?}->{:?}",
                                    graph.edge_records[edge_index].tala_id,
                                    other.tala_id,
                                    start,
                                    end,
                                    segment[0],
                                    segment[1]
                                );
                            }
                            if !clear {
                                edges_clear = false;
                                break 'edges;
                            }
                        }
                    }
                    Ok(node_clear && edges_clear)
                };
                // Source evaluates both obstacle scans for both legs even
                // when an endpoint approach or an earlier scan is blocked.
                let first_clear = segment_clear(p1, intersection)?;
                let second_clear = segment_clear(intersection, p5)?;
                source_direction_ok && target_direction_ok && first_clear && second_clear
            } else {
                false
            }
        } else {
            false
        };

        if unobstructed {
            result.append_value(intersection.unwrap());
            result.append_slot(points.slot(index + 4));
            index += 4;
            break;
        }
        result.append_slot(points.slot(index + 1));
        index += 1;
    }
    for i in index + 1..points.len() {
        stage_charge(budget, 1)?;
        result.append_slot(points.slot(i));
    }
    Ok(result.finish())
}

/// Go's simplify stage only rejects a replacement leg when it enters the
/// open interior of an unrelated node. Endpoint contact and segments that
/// merely run along a boundary are legal.
fn orthogonal_segment_enters_node(rect: Rect, start: Point, end: Point) -> bool {
    const EPSILON: f64 = 1e-6;
    let left = rect.origin.x + EPSILON;
    let right = rect.right() - EPSILON;
    let top = rect.origin.y + EPSILON;
    let bottom = rect.bottom() - EPSILON;
    if start.y == end.y {
        top < start.y && start.y < bottom && start.x.min(end.x) < right && start.x.max(end.x) > left
    } else if start.x == end.x {
        left < start.x && start.x < right && start.y.min(end.y) < bottom && start.y.max(end.y) > top
    } else {
        true
    }
}

fn same_route_direction(a: Point, b: Point, c: Point, d: Point) -> bool {
    let first = Point {
        x: b.x - a.x,
        y: b.y - a.y,
    };
    let second = Point {
        x: d.x - c.x,
        y: d.y - c.y,
    };
    first.x * second.x + first.y * second.y > 0.0 && first.x * second.y == first.y * second.x
}

/// Translation of recovered `Graph.simplifyEdgeRoutes`/`simplifyPoints`.
pub(in crate::internal) fn simplify_edge_routes(graph: &mut ArenaGraph) {
    let _ = simplify_edge_routes_with_budget_inner(graph, None);
}

pub(super) fn simplify_edge_routes_with_budget_inner(
    graph: &mut ArenaGraph,
    budget: Option<&dyn super::route_stage_guard::RouteBudget>,
) -> Result<(), ()> {
    for edge_id in graph.edge_order.read() {
        stage_charge(budget, 1)?;
        let edge_index = edge_id.0 as usize;
        loop {
            let simplified = simplify_once_with_budget(graph, edge_index, budget)?;
            if simplified.len() >= graph.edge_records[edge_index].route.len() {
                break;
            }
            graph.edge_records[edge_index].route.replace(simplified);
        }
    }
    Ok(())
}

/// Checked simplify entry point used by the production pipeline, which gives
/// each post-processing stage its own source 50M work envelope.
pub(in crate::internal) fn simplify_edge_routes_with_budget(
    graph: &mut ArenaGraph,
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), super::route_stage_guard::RouteStageError> {
    super::route_stage_guard::run_atomic_route_stage(
        graph,
        Some(crate::internal::OwnerGraphId(0)),
        Some(&super::route_stage_guard::RuntimeRouteContext),
        "SimplifyEdgeRoutes",
        &[],
        budget,
        |graph, guard| simplify_edge_routes_with_budget_inner(graph, Some(guard)),
    )
}

#[cfg(test)]
mod tests {
    use super::super::route_search_guard::RouteStageWork;
    use super::*;
    use crate::internal::owner_points::SharedOwnerPoint;
    use crate::{Edge, Graph, Node, Size};

    fn graph() -> ArenaGraph {
        let mut input = Graph::default();
        let source = input.add_node(Node::new(
            "source",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let target = input.add_node(Node::new(
            "target",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        input.add_edge(Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[0].position = Some(Point { x: 0.0, y: 0.0 });
        graph.nodes[1].position = Some(Point { x: 70.0, y: -30.0 });
        graph.edge_records[0].route.replace(RoutePoints::decoded(
            [
                Point { x: 20.0, y: 10.0 },
                Point { x: 40.0, y: 10.0 },
                Point { x: 40.0, y: 40.0 },
                Point { x: 80.0, y: 40.0 },
                Point { x: 80.0, y: -10.0 },
            ]
            .into_iter()
            .map(Some),
        ));
        graph
    }

    #[test]
    fn shortened_route_retains_selected_points_and_uses_local_buffer_growth() {
        let graph = graph();
        let old = graph.edge_records[0].route.clone();
        let budget = RouteStageWork::new(7);
        let result = simplify_once_with_budget(&graph, 0, Some(&budget)).unwrap();
        assert_eq!((result.len(), result.capacity(), budget.used()), (3, 3, 7));
        assert!(
            result
                .slot(0)
                .unwrap()
                .same_allocation(&old.slot(0).unwrap())
        );
        assert!(
            result
                .slot(2)
                .unwrap()
                .same_allocation(&old.slot(4).unwrap())
        );
        assert_eq!(result.point(1), Point { x: 80.0, y: 10.0 });
        assert!(!result.header().same_backing_allocation(&old.header()));
        result.edit_point(0, |p| p.x = 21.0);
        assert_eq!(old.point(0).x, 21.0);
        assert_eq!((old.len(), old.capacity()), (5, 8));
    }

    #[test]
    fn work_failure_after_publication_restores_original_backing_and_hidden_points() {
        let mut graph = graph();
        let original = graph.edge_records[0].route.clone();
        let hidden = SharedOwnerPoint::new(Point {
            x: 123.0,
            y: -456.0,
        });
        original
            .header()
            .write_capacity_slot(7, Some(hidden.clone()))
            .unwrap();
        let budget = RouteStageWork::new(8);
        assert!(simplify_edge_routes_with_budget(&mut graph, &budget).is_err());
        let restored = &graph.edge_records[0].route;
        assert_eq!(budget.used(), 8);
        assert_eq!((restored.len(), restored.capacity()), (5, 8));
        assert!(
            restored
                .header()
                .same_backing_allocation(&original.header())
        );
        assert!(
            restored
                .header()
                .read_capacity_slot(7)
                .unwrap()
                .unwrap()
                .same_allocation(&hidden)
        );
    }

    #[test]
    fn work_check_precedes_nullable_window_read_and_tail_charge_is_per_slot() {
        let graph = graph();
        graph.edge_records[0]
            .route
            .header()
            .write_capacity_slot(2, None)
            .unwrap();
        let budget = RouteStageWork::new(0);
        assert!(simplify_once_with_budget(&graph, 0, Some(&budget)).is_err());
        assert_eq!(budget.used(), 0);
        let budget = RouteStageWork::new(1);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                simplify_once_with_budget(&graph, 0, Some(&budget))
            }))
            .is_err()
        );
        assert_eq!(budget.used(), 1);

        let mut tail_graph = self::graph();
        tail_graph.edge_records[0]
            .route
            .append_value(Point { x: 90.0, y: -10.0 });
        tail_graph.edge_records[0].route.append_slot(None);
        // Seven window/obstacle visits, then two tail references. The nil
        // tail is copied without dereferencing it.
        let budget = RouteStageWork::new(8);
        assert!(simplify_once_with_budget(&tail_graph, 0, Some(&budget)).is_err());
        assert_eq!(budget.used(), 8);
        let budget = RouteStageWork::new(9);
        let result = simplify_once_with_budget(&tail_graph, 0, Some(&budget)).unwrap();
        assert_eq!((result.len(), result.capacity(), budget.used()), (5, 8, 9));
        assert!(result.slot(4).is_none());
    }

    #[test]
    fn local_buffer_transitions_preserve_pointer_slots_across_heap_growth() {
        use crate::internal::route_points::LocalRouteBuilder;
        for length in 1..=10 {
            let first = SharedOwnerPoint::new(Point { x: 1.0, y: 2.0 });
            let mut result = LocalRouteBuilder::new(Some(first.clone()));
            for _ in 1..length {
                result.append_slot(Some(first.clone()));
            }
            let result = result.finish();
            let expected = match length {
                1..=4 => length,
                5..=8 => 8,
                _ => 16,
            };
            assert_eq!(result.capacity(), expected);
            for slot in result.header().iter_slots() {
                assert!(slot.unwrap().same_allocation(&first));
            }
            for i in length..result.capacity() {
                assert!(result.header().read_capacity_slot(i).unwrap().is_none());
            }
        }
    }
}
