// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Straight-route fallback and cost estimation against accepted edges.
//!
//! This stage guarantees a final simple candidate where specialized
//! orthogonal routing did not publish one, subject to obstacle legality.

use super::*;

fn edge_length(points: &[Point]) -> f64 {
    let mut length = 0.0;
    for segment in points.windows(2) {
        length += euclidean_distance(segment[0], segment[1]);
    }
    length
}

fn edge_crossings(
    graph: &ArenaGraph,
    edge_index: usize,
    points: &[Point],
    routes: &[Vec<Point>],
) -> usize {
    graph
        .member_edge_entries()
        // TALA's `Edges.getEdgeCrossingCount` excludes only the edge being
        // scored. Its `nonSharedCrossingCount` name does not mean that edges
        // sharing an endpoint are omitted; those edges still participate in
        // the segment predicate and can affect cluster-branch candidate
        // selection.
        .filter(|(other_index, _)| *other_index != edge_index)
        .map(|(other_index, _)| {
            let other_points = &routes[other_index];
            points
                .windows(2)
                .enumerate()
                .map(|(index, segment)| {
                    other_points
                        .windows(2)
                        .filter(|other_segment| {
                            if !non_parallel_intersection(
                                segment[0],
                                segment[1],
                                other_segment[0],
                                other_segment[1],
                            ) {
                                return false;
                            }
                            // TALA's nonSharedCrossingCount excludes a
                            // crossing where a vertical segment shares its
                            // x coordinate, or a horizontal segment shares
                            // its y coordinate, with either endpoint of the
                            // other segment. This is deliberately narrower
                            // than the general segment-intersection helper.
                            let segment_start = points[index];
                            if segment[1].x == segment_start.x
                                && (other_segment[0].x == segment_start.x
                                    || other_segment[1].x == segment_start.x)
                            {
                                return false;
                            }
                            if segment[1].y == segment_start.y
                                && (other_segment[0].y == segment_start.y
                                    || other_segment[1].y == segment_start.y)
                            {
                                return false;
                            }
                            true
                        })
                        .count()
                })
                .sum::<usize>()
        })
        .sum()
}

pub(super) fn non_parallel_intersection(a: Point, b: Point, c: Point, d: Point) -> bool {
    let orientation = |first: Point, second: Point, third: Point| {
        (second.y - first.y) * (third.x - second.x) - (second.x - first.x) * (third.y - second.y)
    };
    let equal_signs = |first: f64, second: f64| {
        (first > 0.0 && second > 0.0)
            || (first == 0.0 && second == 0.0)
            || (first < 0.0 && second < 0.0)
    };
    let first = orientation(a, b, c);
    let second = orientation(a, b, d);
    if equal_signs(first, second) {
        return false;
    }
    let third = orientation(c, d, a);
    let fourth = orientation(c, d, b);
    !equal_signs(third, fourth)
}

pub(super) fn estimated_edge_cost(graph: &ArenaGraph, edge_index: usize, points: &[Point]) -> f64 {
    let routes = graph
        .edge_records
        .iter()
        .map(|edge| edge.route.read().clone())
        .collect::<Vec<_>>();
    estimated_edge_cost_against(graph, edge_index, points, &routes)
}

pub(super) fn estimated_edge_cost_against(
    graph: &ArenaGraph,
    edge_index: usize,
    points: &[Point],
    routes: &[Vec<Point>],
) -> f64 {
    const TURN_PENALTY: f64 = 1.4;
    const CROSSING_COST: f64 = 0.48_f64 * 0.48 * 0.48;
    let length = edge_length(points);
    let crossing_count = edge_crossings(graph, edge_index, points, routes);

    length * TURN_PENALTY.powi(points.len().saturating_sub(2) as i32)
        + CROSSING_COST * crossing_count as f64
}

/// Go estimateRouteCostGuarded over the explicitly selected edge inventory.
/// Work is checked before each length segment, edge visit, and crossing pair.
pub(super) fn estimated_selected_edge_cost_with_budget(
    edge_index: usize,
    points: &[Point],
    routes: &[Vec<Point>],
    other_edges: &[usize],
    budget: &super::route_search_guard::LocalRouteSearchWork,
) -> Option<f64> {
    estimated_selected_edge_cost_with_work(edge_index, points, routes, other_edges, budget)
}
pub(super) fn estimated_selected_edge_cost_with_work(
    edge_index: usize,
    points: &[Point],
    routes: &[Vec<Point>],
    other_edges: &[usize],
    budget: &dyn super::route_line::LineWork,
) -> Option<f64> {
    let mut length = 0.0;
    for segment in points.windows(2) {
        budget.step().ok()?;
        length += euclidean_distance(segment[0], segment[1]);
    }
    let mut crossings = 0usize;
    for &other_index in other_edges {
        budget.step().ok()?;
        if other_index == edge_index {
            continue;
        }
        for segment in points.windows(2) {
            for other in routes[other_index].windows(2) {
                budget.step().ok()?;
                if !non_parallel_intersection(segment[0], segment[1], other[0], other[1]) {
                    continue;
                }
                if segment[1].x == segment[0].x
                    && (other[0].x == segment[0].x || other[1].x == segment[0].x)
                {
                    continue;
                }
                if segment[1].y == segment[0].y
                    && (other[0].y == segment[0].y || other[1].y == segment[0].y)
                {
                    continue;
                }
                crossings += 1;
            }
        }
    }
    Some(length * 1.4_f64.powi(points.len().saturating_sub(2) as i32) + 0.110592 * crossings as f64)
}

/// Translation of the represented flat-graph portion of recovered
/// `Graph.StraightEdgesFallback` and `RouteLine`. Tree edges, SQL-table column
/// edges, and loops retain their routed form; eligible edges switch only
/// when the recovered direct-line cost is strictly lower.
pub(in crate::internal) fn straight_edges_fallback(graph: &mut ArenaGraph) {
    for edge_index in graph.member_edge_indices() {
        let edge = &graph.edge_records[edge_index];
        if edge.from == edge.to
            // Recovered `Graph.StraightEdgesFallback` excludes every edge
            // touching a cluster before it considers direct-line cost. The
            // cluster membership is durable graph state; it is not inferred
            // from the edge's final geometry.
            || graph.nodes[edge.from.0 as usize].cluster.is_some()
            || graph.nodes[edge.to.0 as usize].cluster.is_some()
            || graph.is_tree_edge(crate::EdgeId(edge_index as u32))
            // The release checks the source-side hierarchy carrier here.
            || graph.nodes[edge.from.0 as usize].hierarchy.is_some()
            || edge.has_table_column()
        {
            continue;
        }
        try_straight_edge_fallback(graph, edge_index);
    }
}

/// Checked `StraightEdgesFallback` boundary. The Go stage reserves the
/// quadratic crossing-estimate envelope before mutating any route and rolls
/// the graph back if the context or work limit terminates the attempt.
pub(in crate::internal) fn straight_edges_fallback_with_budget(
    graph: &mut ArenaGraph,
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), ()> {
    let snapshot_routing_costs = graph.raw_routing_costs();
    let snapshot = graph.clone();
    let result = (|| {
        let edge_count = graph.edge_order.len() as u64;
        for edge_index in graph.member_edge_indices() {
            super::route_search_guard::reserve_checked(budget, 1)?;
            let edge = &graph.edge_records[edge_index];
            let eligible = edge.from != edge.to
                && graph.nodes[edge.from.0 as usize].cluster.is_none()
                && graph.nodes[edge.to.0 as usize].cluster.is_none()
                && !graph.is_tree_edge(crate::EdgeId(edge_index as u32))
                && graph.nodes[edge.from.0 as usize].hierarchy.is_none()
                && !edge.has_table_column();
            if eligible {
                let scan = (edge.route.len().max(1) as u64).saturating_mul(edge_count.max(1));
                super::route_search_guard::reserve_checked(budget, scan)?;
            }
        }
        straight_edges_fallback(graph);
        if crate::internal::cancellation_requested() {
            return Err(());
        }
        Ok(())
    })();
    if result.is_err() {
        *graph = snapshot;
        graph.restore_routing_costs(snapshot_routing_costs);
    }
    result
}

/// Direct translation of recovered `Graph.tryStraightEdgeFallback`. The
/// standalone caller owns eligibility and skips only table-column edges.
pub(in crate::internal) fn try_straight_edge_fallback(graph: &mut ArenaGraph, edge_index: usize) {
    let all_edge_indices = graph.member_edge_indices().collect::<Vec<_>>();
    let edge = &graph.edge_records[edge_index];
    let original_cost = estimated_edge_cost(graph, edge_index, &edge.route.read());
    let Some(line) =
        route_line::route_line_allow_matching_overlap(graph, edge_index, &all_edge_indices, None)
    else {
        return;
    };
    let mut line_cost = line.cost;
    if edge.route.len() == 4 {
        let gap = ((edge.route.point(2).x - edge.route.point(1).x).powi(2)
            + (edge.route.point(2).y - edge.route.point(1).y).powi(2))
        .sqrt();
        if gap <= 5.0 {
            line_cost *= 0.25;
        }
    }
    if crate::internal::trace_env_enabled("WEFTAN_TRACE_STRAIGHT_EDGE") {
        eprintln!(
            "STRAIGHT_RUST edge={} from={} to={} original={:.17e} line={:.17e} points={:?} line_points={:?}",
            edge_index,
            graph.nodes[edge.from.0 as usize].tala_id,
            graph.nodes[edge.to.0 as usize].tala_id,
            original_cost,
            line_cost,
            edge.route.read(),
            vec![line.source.point, line.target.point]
        );
    }
    if line_cost < original_cost {
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_STRAIGHT_EDGE") {
            eprintln!(
                "STRAIGHT_REPLACE_RUST edge={} from={} to={} original={:.17e} line={:.17e}",
                edge_index,
                graph.nodes[edge.from.0 as usize].tala_id,
                graph.nodes[edge.to.0 as usize].tala_id,
                original_cost,
                line_cost
            );
        }
        graph.edge_records[edge_index]
            .route
            .replace(RoutePoints::literal(
                &(vec![line.source.point, line.target.point]),
            ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentAlignment, Edge, Graph, Insets, LabelPosition, Node, ShapeKind, Size};

    fn test_node(name: &str) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.into(),
            size: Size {
                width: 10.0,
                height: 10.0,
            },
            declared_size: None,
            d2_authored_geometry: None,
            label_size: None,
            font_size: None,
            label_position: LabelPosition::Unset,
            parent: None,
            locked_position: None,
            constrained_x: None,
            constrained_y: None,
            near: None,
            fixed_width: false,
            fixed_height: false,
            direction: None,
            force_hierarchy: false,
            grid_rows: None,
            grid_columns: None,
            canvas_position: None,
            content_insets: Insets::uniform(0.0),
            layout_margins: Insets::uniform(0.0),
            external_label: None,
            icon_position: None,
            has_icon: false,
            label_aware_grid: false,
            packed_grid: false,
            content_alignment: ContentAlignment::Padding,
            port_spread: 0.0,
            person: false,
            is_3d: false,
            is_multiple: false,
            shape: ShapeKind::Rectangle,
        }
    }

    #[test]
    fn selected_cost_checks_length_edge_and_crossing_work_boundaries() {
        let points = [Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 }];
        let routes = vec![
            vec![],
            vec![
                Point { x: 5.0, y: -5.0 },
                Point { x: 5.0, y: 5.0 },
                Point { x: 6.0, y: 5.0 },
            ],
        ];
        let work = super::super::route_search_guard::LocalRouteSearchWork::new(4);
        let cost = estimated_selected_edge_cost_with_budget(0, &points, &routes, &[1], &work);
        assert_eq!(cost.unwrap().to_bits(), (10.0_f64 + 0.110592).to_bits());
        assert_eq!(work.used(), 4);
        for limit in 0..4 {
            let work = super::super::route_search_guard::LocalRouteSearchWork::new(limit);
            assert!(
                estimated_selected_edge_cost_with_budget(0, &points, &routes, &[1], &work)
                    .is_none()
            );
            assert_eq!(work.used(), limit);
        }
        let work = super::super::route_search_guard::LocalRouteSearchWork::new(2);
        assert_eq!(
            estimated_selected_edge_cost_with_budget(0, &points, &routes, &[0], &work),
            Some(10.0)
        );
        assert_eq!(work.used(), 2);
    }

    #[test]
    fn crossing_cost_includes_edges_that_share_an_endpoint() {
        let mut input = Graph::default();
        let source = input.add_node(test_node("source"));
        let first_target = input.add_node(test_node("first-target"));
        let second_target = input.add_node(test_node("second-target"));
        let first_edge = input.add_edge(Edge {
            source,
            target: first_target,
        });
        let second_edge = input.add_edge(Edge {
            source,
            target: second_target,
        });
        input.set_edge_route(
            first_edge,
            vec![Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 }],
        );
        input.set_edge_route(
            second_edge,
            vec![Point { x: 5.0, y: -5.0 }, Point { x: 5.0, y: 5.0 }],
        );

        let arena = ArenaGraph::from_input(&input);
        let routes = arena
            .member_edges()
            .map(|edge| edge.route.read().clone())
            .collect::<Vec<_>>();

        assert_eq!(edge_crossings(&arena, 0, &routes[0], &routes), 1);
    }

    #[test]
    fn route_length_matches_go_axis_distance_for_extreme_deltas() {
        let huge_horizontal = [Point { x: 0.0, y: 7.0 }, Point { x: 1.0e200, y: 7.0 }];
        let tiny_vertical = [
            Point { x: 4.0, y: 0.0 },
            Point {
                x: 4.0,
                y: 1.0e-200,
            },
        ];

        assert_eq!(
            edge_length(&huge_horizontal).to_bits(),
            1.0e200_f64.to_bits()
        );
        assert_eq!(
            edge_length(&tiny_vertical).to_bits(),
            1.0e-200_f64.to_bits()
        );
    }

    #[test]
    fn non_parallel_crossing_excludes_collinear_and_same_side_segments() {
        assert!(non_parallel_intersection(
            Point { x: 0.0, y: 0.0 },
            Point { x: 10.0, y: 0.0 },
            Point { x: 5.0, y: -5.0 },
            Point { x: 5.0, y: 5.0 },
        ));
        assert!(!non_parallel_intersection(
            Point { x: 0.0, y: 0.0 },
            Point { x: 10.0, y: 0.0 },
            Point { x: 2.0, y: 0.0 },
            Point { x: 8.0, y: 0.0 },
        ));
        assert!(!non_parallel_intersection(
            Point { x: 0.0, y: 0.0 },
            Point { x: 10.0, y: 0.0 },
            Point { x: 2.0, y: 1.0 },
            Point { x: 8.0, y: 1.0 },
        ));
    }
}
