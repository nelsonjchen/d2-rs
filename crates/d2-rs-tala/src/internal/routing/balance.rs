// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Balancing of compatible parallel orthogonal route segments.
//!
//! Movable segments are grouped into channels and shifted toward regular
//! spacing without crossing obstacles or violating endpoint constraints.

use super::super::ClusterArrangement;
use super::*;
use crate::LabelPosition;
use crate::internal::quality;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BalanceSegment {
    edge: usize,
    first: usize,
    second: usize,
}

fn segment_points(graph: &ArenaGraph, segment: BalanceSegment) -> (Point, Point) {
    (
        graph.edge_records[segment.edge].route.point(segment.first),
        graph.edge_records[segment.edge].route.point(segment.second),
    )
}

fn collect_balance_segments(
    graph: &ArenaGraph,
    edge_indices: &[usize],
    vertical: bool,
) -> Vec<BalanceSegment> {
    let mut result = Vec::new();
    for edge_index in edge_indices {
        for index in 0..graph.edge_records[*edge_index]
            .route
            .len()
            .saturating_sub(1)
        {
            let first = graph.edge_records[*edge_index].route.point(index);
            let second = graph.edge_records[*edge_index].route.point(index + 1);
            if (vertical && first.x == second.x) || (!vertical && first.y == second.y) {
                result.push(BalanceSegment {
                    edge: *edge_index,
                    first: index,
                    second: index + 1,
                });
            }
        }
    }
    result
}

fn canonical_segment(
    graph: &ArenaGraph,
    segment: BalanceSegment,
    vertical: bool,
) -> (Point, Point) {
    let (first, second) = segment_points(graph, segment);
    if (vertical && first.y <= second.y) || (!vertical && first.x <= second.x) {
        (first, second)
    } else {
        (second, first)
    }
}

fn segments_overlap_along_length(
    first: (Point, Point),
    second: (Point, Point),
    vertical: bool,
    buffer: f64,
) -> bool {
    if vertical {
        first.0.y < second.1.y + buffer && second.0.y < first.1.y + buffer
    } else {
        first.0.x < second.1.x + buffer && second.0.x < first.1.x + buffer
    }
}

fn segment_coordinate(segment: (Point, Point), vertical: bool) -> f64 {
    if vertical { segment.0.x } else { segment.0.y }
}

/// Go's `map[float64]struct{}` treats negative and positive zero as the same
/// key. Preserve that when using rounded lane coordinates as set keys.
fn rounded_balance_lane_key(coordinate: f64) -> u64 {
    let rounded = coordinate.round();
    if rounded == 0.0 {
        0.0_f64.to_bits()
    } else {
        rounded.to_bits()
    }
}

/// `geo.Segment.GetBounds` uses an inclusive movement band, unlike
/// `geo.Segment.Overlaps`: an obstacle whose nearest endpoint is exactly
/// `buffer` units from the moving segment still constrains its range.
fn intersects_movement_band(
    moving: (Point, Point),
    obstacle: (Point, Point),
    vertical: bool,
    buffer: f64,
) -> bool {
    if vertical {
        obstacle.1.y >= moving.0.y - buffer && obstacle.0.y <= moving.1.y + buffer
    } else {
        obstacle.1.x >= moving.0.x - buffer && obstacle.0.x <= moving.1.x + buffer
    }
}

fn uses_top_down_route_flavor(graph: &ArenaGraph) -> bool {
    graph.nodes.iter().any(|node| {
        node.grid_rows.is_some()
            || node.grid_columns.is_some()
            || node.label_aware_grid
            || node.packed_grid
    })
}

pub(in crate::internal) fn uses_ordinary_scope_routing(graph: &ArenaGraph) -> bool {
    graph.nodes.iter().filter(|node| node.is_container).count() == 1
        && graph
            .nodes
            .iter()
            .filter(|node| node.is_container)
            .all(|node| node.container.is_none())
        && !uses_top_down_route_flavor(graph)
        && graph.member_edges().all(|edge| {
            edge.source_arrowhead_label.is_none() && edge.target_arrowhead_label.is_none()
        })
}

fn node_locked_segments(graph: &ArenaGraph, vertical: bool) -> Vec<(Point, Point)> {
    let mut result = Vec::new();
    // Go's nodeSegmentsGuarded walks current Graph.Nodes. Retained arena
    // entries can include retired vessels and absorbed members; their stale
    // boxes must not constrain a live route. Resolve visible carriers using
    // the same current aggregate geometry as the numerical graph view.
    for &node in graph.graph_node_order_cow().iter() {
        let Some(origin) = graph.active_node_position(node) else {
            continue;
        };
        let rect = Rect {
            origin,
            size: graph.active_node_size(node),
        };
        if vertical {
            result.push((
                rect.origin,
                Point {
                    x: rect.origin.x,
                    y: rect.bottom(),
                },
            ));
            result.push((
                Point {
                    x: rect.right(),
                    y: rect.origin.y,
                },
                Point {
                    x: rect.right(),
                    y: rect.bottom(),
                },
            ));
        } else {
            result.push((
                rect.origin,
                Point {
                    x: rect.right(),
                    y: rect.origin.y,
                },
            ));
            result.push((
                Point {
                    x: rect.origin.x,
                    y: rect.bottom(),
                },
                Point {
                    x: rect.right(),
                    y: rect.bottom(),
                },
            ));
        }
    }
    result
}

fn movement_bounds(
    graph: &ArenaGraph,
    segment: BalanceSegment,
    vertical: bool,
    locked: &[(Point, Point)],
) -> (f64, f64) {
    let current = canonical_segment(graph, segment, vertical);
    let coordinate = segment_coordinate(current, vertical);
    // Go's routeSegmentBounds treats a point segment as unconstrained and
    // gives it the standard +/-100 movement window. Scanning obstacles here
    // would incorrectly pin a zero-length route leg to nearby geometry.
    if current.0 == current.1 {
        return (coordinate - 100.0, coordinate + 100.0);
    }
    let mut floor = f64::NEG_INFINITY;
    let mut ceil = f64::INFINITY;
    for locked_segment in locked {
        let locked_segment = if (vertical && locked_segment.0.y <= locked_segment.1.y)
            || (!vertical && locked_segment.0.x <= locked_segment.1.x)
        {
            *locked_segment
        } else {
            (locked_segment.1, locked_segment.0)
        };
        if !intersects_movement_band(current, locked_segment, vertical, 40.0) {
            continue;
        }
        let locked_coordinate = segment_coordinate(locked_segment, vertical);
        // D2's Segment.GetBounds treats a coincident obstacle as the floor.
        // That detail is significant for routes launched along node borders.
        if locked_coordinate <= coordinate {
            floor = floor.max(locked_coordinate);
        } else {
            ceil = ceil.min(locked_coordinate);
        }
    }
    if floor.is_infinite() {
        floor = coordinate - 100.0;
    }
    if ceil.is_infinite() {
        ceil = coordinate + 100.0;
    }
    (floor, ceil)
}

fn evenly_distribute(floor: f64, ceil: f64, count: usize) -> Vec<f64> {
    if floor >= ceil || count == 0 {
        return Vec::new();
    }
    let increment = ((ceil - floor) / (count + 1) as f64).floor();
    if increment == 0.0 {
        return Vec::new();
    }
    // Keep Go's `floor + float64(index)*increment` evaluation order. Repeated
    // addition can produce different IEEE-754 bits for large ranges.
    (1..=count)
        .map(|index| floor + index as f64 * increment)
        .collect()
}

fn set_segment_coordinate(
    graph: &mut ArenaGraph,
    segment: BalanceSegment,
    vertical: bool,
    value: f64,
) {
    if vertical {
        graph.edge_records[segment.edge]
            .route
            .point_mut(segment.first)
            .x = value;
        graph.edge_records[segment.edge]
            .route
            .point_mut(segment.second)
            .x = value;
    } else {
        graph.edge_records[segment.edge]
            .route
            .point_mut(segment.first)
            .y = value;
        graph.edge_records[segment.edge]
            .route
            .point_mut(segment.second)
            .y = value;
    }
}

fn fixed_port_point_ids(graph: &ArenaGraph, regular: &[usize]) -> BTreeSet<(usize, usize)> {
    regular
        .iter()
        .filter(|&&edge_index| has_fixed_balancing_ports(graph, edge_index))
        .flat_map(|&edge_index| {
            let point_count = graph.edge_records[edge_index].route.len();
            (0..point_count)
                .filter(move |&point_index| {
                    point_index < 2 || point_index >= point_count.saturating_sub(2)
                })
                .map(move |point_index| (edge_index, point_index))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BalanceOrder {
    Preserved,
    Reversed,
    ContactChanged,
}

fn segment_axis_interval(segment: (Point, Point), vertical: bool) -> (f64, f64) {
    if vertical {
        (segment.0.y.min(segment.1.y), segment.0.y.max(segment.1.y))
    } else {
        (segment.0.x.min(segment.1.x), segment.0.x.max(segment.1.x))
    }
}

fn segment_coordinate_for_balance(segment: (Point, Point), vertical: bool) -> f64 {
    if vertical { segment.0.x } else { segment.0.y }
}

/// Match TALA's route-order guard. A segment in the narrowest movement range
/// must not cross a route from another range, or create a new collinear contact.
fn check_balance_order(
    graph: &ArenaGraph,
    batch: &[usize],
    batch_set: &BTreeSet<usize>,
    segments: &[BalanceSegment],
    special_segments: &[(Point, Point)],
    proposed: &[f64],
    vertical: bool,
) -> BalanceOrder {
    let moving = batch.iter().enumerate().any(|(index, segment_index)| {
        let old = segment_coordinate_for_balance(
            canonical_segment(graph, segments[*segment_index], vertical),
            vertical,
        );
        proposed[index] != old
    });
    if !moving {
        return BalanceOrder::Preserved;
    }

    let mut order = BalanceOrder::Preserved;
    for other_index in 0..segments.len() {
        if batch_set.contains(&other_index) {
            continue;
        }
        let other = canonical_segment(graph, segments[other_index], vertical);
        let other_interval = segment_axis_interval(other, vertical);
        for (batch_position, segment_index) in batch.iter().enumerate() {
            let segment = canonical_segment(graph, segments[*segment_index], vertical);
            if segments[*segment_index].edge == segments[other_index].edge {
                continue;
            }
            let old = segment_coordinate_for_balance(segment, vertical);
            if proposed[batch_position] == old {
                continue;
            }
            let interval = segment_axis_interval(segment, vertical);
            if interval.1 < other_interval.0 || other_interval.1 < interval.0 {
                continue;
            }
            let position = segment_coordinate_for_balance(other, vertical);
            if old == position || proposed[batch_position] == position {
                return BalanceOrder::ContactChanged;
            }
            if (old < position) != (proposed[batch_position] < position) {
                order = BalanceOrder::Reversed;
            }
        }
    }

    for other in special_segments {
        let other_interval = segment_axis_interval(*other, vertical);
        for (batch_position, segment_index) in batch.iter().enumerate() {
            let segment = canonical_segment(graph, segments[*segment_index], vertical);
            let old = segment_coordinate_for_balance(segment, vertical);
            if proposed[batch_position] == old {
                continue;
            }
            let interval = segment_axis_interval(segment, vertical);
            if interval.1 < other_interval.0 || other_interval.1 < interval.0 {
                continue;
            }
            let position = segment_coordinate_for_balance(*other, vertical);
            if old == position || proposed[batch_position] == position {
                return BalanceOrder::ContactChanged;
            }
            if (old < position) != (proposed[batch_position] < position) {
                order = BalanceOrder::Reversed;
            }
        }
    }
    order
}

fn shared_balancing_cluster(
    graph: &ArenaGraph,
    first: BalanceSegment,
    second: BalanceSegment,
    vertical: bool,
) -> bool {
    let first_edge = &graph.edge_records[first.edge];
    let second_edge = &graph.edge_records[second.edge];
    let second_clusters = [
        graph.nodes[second_edge.from.0 as usize].cluster,
        graph.nodes[second_edge.to.0 as usize].cluster,
    ];
    let cluster_index = [
        graph.nodes[first_edge.from.0 as usize].cluster,
        graph.nodes[first_edge.to.0 as usize].cluster,
    ]
    .into_iter()
    .flatten()
    .find(|cluster| second_clusters.contains(&Some(*cluster)));
    let Some(cluster) = cluster_index.map(|index| &graph.clusters[index]) else {
        return false;
    };
    if cluster.arrangement != cluster.desired_arrangement {
        return false;
    }

    let first_coordinate = segment_coordinate(canonical_segment(graph, first, vertical), vertical);
    let second_coordinate =
        segment_coordinate(canonical_segment(graph, second, vertical), vertical);
    (first_coordinate - second_coordinate).abs() <= 1.0
        && matches!(
            (vertical, cluster.arrangement),
            (true, ClusterArrangement::Column) | (false, ClusterArrangement::Row)
        )
}

fn deduplicate_route(points: &mut Vec<Point>) {
    if points.len() <= 1 {
        return;
    }
    points.dedup();
    if points.len() == 1 {
        let mut second = points[0];
        second.x += 1.0;
        points.push(second);
    }
}

fn non_shared_crossing(first: (Point, Point), second: (Point, Point)) -> bool {
    if !super::channel_safety::crossing(first.0, first.1, second.0, second.1) {
        return false;
    }
    // The recovered cost scorer does not count an intersection at the end of
    // an axis-aligned segment when it is also a bend on the other route.
    if first.0.x == first.1.x && (first.0.x == second.0.x || first.0.x == second.1.x) {
        return false;
    }
    if first.0.y == first.1.y && (first.0.y == second.0.y || first.0.y == second.1.y) {
        return false;
    }
    true
}

fn collinear_overlap(first: (Point, Point), second: (Point, Point)) -> f64 {
    if first.0.x == first.1.x && second.0.x == second.1.x && first.0.x == second.0.x {
        return (first.0.y.max(first.1.y).min(second.0.y.max(second.1.y))
            - first.0.y.min(first.1.y).max(second.0.y.min(second.1.y)))
        .max(0.0);
    }
    if first.0.y == first.1.y && second.0.y == second.1.y && first.0.y == second.0.y {
        return (first.0.x.max(first.1.x).min(second.0.x.max(second.1.x))
            - first.0.x.min(first.1.x).max(second.0.x.min(second.1.x)))
        .max(0.0);
    }
    0.0
}

/// Match TALA's crossing-removal fallback for a reversed balancing batch.
/// The normal ordering guard rejects a route that crosses an existing lane;
/// Go then accepts the proposal only when its complete tentative geometry
/// removes crossings without creating collinear contact. Rust stores points
/// by value, so the candidate is materialized per edge before comparison.
fn reversal_removes_crossings(
    graph: &ArenaGraph,
    segments: &[BalanceSegment],
    batch: &[usize],
    proposed: &[f64],
    vertical: bool,
) -> bool {
    let mut candidates = graph
        .edge_records
        .iter()
        .map(|edge| edge.route.read().clone())
        .collect::<Vec<_>>();
    let mut updates = BTreeMap::<(usize, usize), Point>::new();
    for (batch_index, segment_index) in batch.iter().copied().enumerate() {
        let segment = segments[segment_index];
        for point_index in [segment.first, segment.second] {
            let mut point = graph.edge_records[segment.edge].route.point(point_index);
            if vertical {
                point.x = proposed[batch_index];
            } else {
                point.y = proposed[batch_index];
            }
            if let Some(previous) = updates.insert((segment.edge, point_index), point)
                && previous != point
            {
                return false;
            }
        }
    }
    let mut affected = Vec::new();
    for (edge_index, points) in candidates.iter_mut().enumerate() {
        for (point_index, point) in points.iter_mut().enumerate() {
            if let Some(replacement) = updates.get(&(edge_index, point_index)) {
                *point = *replacement;
            }
        }
        if points != &graph.edge_records[edge_index].route.read() {
            affected.push(edge_index);
        }
    }
    let mut improved = false;
    for (affected_position, edge_index) in affected.iter().copied().enumerate() {
        for other_index in graph.member_edge_indices() {
            if edge_index == other_index
                || (affected_position > 0 && affected[..affected_position].contains(&other_index))
            {
                continue;
            }
            let before_edge = &graph.edge_records[edge_index].route.read();
            let before_other = &graph.edge_records[other_index].route.read();
            let after_edge = &candidates[edge_index];
            let after_other = &candidates[other_index];
            let mut before_count = 0;
            let mut after_count = 0;
            for (first_index, before_pair) in before_edge.windows(2).enumerate() {
                for (second_index, before_other_pair) in before_other.windows(2).enumerate() {
                    let after_pair = (after_edge[first_index], after_edge[first_index + 1]);
                    let after_other_pair =
                        (after_other[second_index], after_other[second_index + 1]);
                    before_count += usize::from(non_shared_crossing(
                        (before_pair[0], before_pair[1]),
                        (before_other_pair[0], before_other_pair[1]),
                    ));
                    after_count += usize::from(non_shared_crossing(after_pair, after_other_pair));
                    if collinear_overlap(
                        (before_pair[0], before_pair[1]),
                        (before_other_pair[0], before_other_pair[1]),
                    ) == 0.0
                        && collinear_overlap(after_pair, after_other_pair) > 0.0
                    {
                        return false;
                    }
                }
            }
            if after_count > before_count {
                return false;
            }
            improved |= after_count < before_count;
        }
    }
    improved
}

/// Translation of recovered `Graph.balanceEdgeSegments` for ordinary arena
/// edges. It distributes overlapping parallel segments between the nearest
/// node/special-edge bounds while preserving shared segment coordinates.
pub(in crate::internal) fn is_special_edge_for_balancing(
    graph: &ArenaGraph,
    edge_index: usize,
) -> bool {
    let edge = &graph.edge_records[edge_index];
    if edge.route.len() < 2 {
        return true;
    }
    let first = edge.route.point(0);
    let last = edge.route.point(edge.route.len() - 1);
    graph.tree_routing_nodes.contains_key(&edge.from)
        || graph.tree_routing_nodes.contains_key(&edge.to)
        || edge.from == edge.to
        || edge.has_table_column()
        || graph.nodes[edge.from.0 as usize].shape == ShapeKind::Diamond
        || graph.nodes[edge.to.0 as usize].shape == ShapeKind::Diamond
        || (first.x - last.x).abs() == 1.0
        || (first.y - last.y).abs() == 1.0
}

pub(in crate::internal) fn balance_edge_segments(graph: &mut ArenaGraph) {
    super::balance_owner::balance_edge_segments(graph);
}

pub(in crate::internal) fn balance_edge_segments_with_budget(
    graph: &mut ArenaGraph,
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), super::route_stage_guard::RouteStageError> {
    super::balance_owner::balance_edge_segments_with_budget(graph, budget)
}

fn balance_port_interiors(graph: &mut ArenaGraph) {
    let mut selected = Vec::new();
    let mut locked = Vec::new();
    for edge_index in graph
        .edge_order
        .read()
        .iter()
        .map(|edge_id| edge_id.0 as usize)
    {
        let edge = &graph.edge_records[edge_index];
        let from_tree = graph.tree_routing_nodes.contains_key(&edge.from);
        let to_tree = graph.tree_routing_nodes.contains_key(&edge.to);
        let fixed_port = has_fixed_balancing_ports(graph, edge_index);
        if fixed_port
            && !edge.is_curve
            && !from_tree
            && !to_tree
            && edge.from != edge.to
            && edge.route.len() >= 6
        {
            selected.push(edge_index);
        } else {
            locked.push(edge_index);
        }
    }
    if selected.is_empty() {
        return;
    }

    let before = selected
        .iter()
        .map(|&edge| (edge, graph.edge_records[edge].route.read().clone()))
        .collect::<BTreeMap<_, _>>();
    // Go's fixed-port pass admits its tentative channels only when the full
    // quality inspection does not worsen obstructions, crossings, text
    // occlusion, or route length. Keep that acceptance boundary alongside
    // the route-clear check; otherwise a crossing-free move can still change
    // the selected route in a way the upstream pass rolls back.
    let before_quality = inspect_balance_quality(graph);
    balance_regular_edges(graph, &locked, &selected);
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2RS_TRACE_PORT_BALANCE").is_some() {
        eprintln!(
            "{}",
            serde_json::json!({"event": "port-balance-candidate", "edges": selected.iter().map(|&index| {
            let edge = &graph.edge_records[index];
            serde_json::json!({"id": edge.tala_id.to_string(), "before": before[&index], "candidate": edge.route.read()})
        }).collect::<Vec<_>>() })
        );
    }
    for &edge_index in &selected {
        let edge = &graph.edge_records[edge_index];
        if !changed_route_is_clear(
            graph,
            edge_index,
            &edge.route.read(),
            &before[&edge_index],
            true,
        ) {
            for (&index, points) in &before {
                graph.edge_records[index]
                    .route
                    .replace(RoutePoints::literal(points));
            }
            return;
        }
    }
    let after_quality = inspect_balance_quality(graph);
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2RS_TRACE_PORT_BALANCE").is_some() {
        eprintln!(
            "{}",
            serde_json::json!({"event": "port-balance-quality", "before": [before_quality.route_obstructions as f64, before_quality.crossings, before_quality.text_occlusions as f64, before_quality.route_length], "after": [after_quality.route_obstructions as f64, after_quality.crossings, after_quality.text_occlusions as f64, after_quality.route_length]})
        );
    }
    if after_quality.route_obstructions > before_quality.route_obstructions
        || after_quality.crossings > before_quality.crossings
        || after_quality.text_occlusions > before_quality.text_occlusions
        || after_quality.route_length > before_quality.route_length * 1.1
    {
        for (&index, points) in &before {
            graph.edge_records[index]
                .route
                .replace(RoutePoints::literal(points));
        }
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

fn orthogonal_segment_enters_node(rect: Rect, start: Point, end: Point) -> bool {
    const EPSILON: f64 = 1e-6;
    let left = rect.origin.x + EPSILON;
    let right = rect.right() - EPSILON;
    let top = rect.origin.y + EPSILON;
    let bottom = rect.bottom() - EPSILON;
    if start.y == end.y {
        return top < start.y
            && start.y < bottom
            && start.x.min(end.x) < right
            && start.x.max(end.x) > left;
    }
    if start.x == end.x {
        return left < start.x
            && start.x < right
            && start.y.min(end.y) < bottom
            && start.y.max(end.y) > top;
    }
    true
}

/// Equivalent to Go's `changedRouteIsClear`: only newly-created legs are
/// checked, while preserving fixed-port endpoint positions and signed
/// approach directions.
fn changed_route_is_clear(
    graph: &ArenaGraph,
    edge_index: usize,
    candidate: &[Point],
    before: &[Point],
    fixed_ports: bool,
) -> bool {
    if candidate.len() < 2 || before.len() < 2 {
        return false;
    }
    let edge = &graph.edge_records[edge_index];
    if fixed_ports
        && (candidate[0] != before[0]
            || candidate[candidate.len() - 1] != before[before.len() - 1]
            || !same_route_direction(before[0], before[1], candidate[0], candidate[1])
            || !same_route_direction(
                before[before.len() - 2],
                before[before.len() - 1],
                candidate[candidate.len() - 2],
                candidate[candidate.len() - 1],
            ))
    {
        return false;
    }
    for segment in candidate.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if before
            .windows(2)
            .any(|old| old[0] == start && old[1] == end)
        {
            continue;
        }
        if start.x != end.x && start.y != end.y {
            return false;
        }
        // Source lineIntersectsUnrelatedNode visits current Graph.Nodes.
        // Detached retained vessels cannot obstruct a live route; active
        // numerical carriers resolve to their actual distinct vessels.
        for node_id in graph.original_owner_member_nodes() {
            let node = &graph.nodes[node_id.0 as usize];
            if node_id != edge.from
                && node_id != edge.to
                && (graph.is_descendant_of(edge.from, node_id)
                    || graph.is_descendant_of(edge.to, node_id))
            {
                continue;
            }
            let Some(origin) = node.position else {
                continue;
            };
            let rect = Rect {
                origin,
                size: node.rect.size,
            };
            if rect.size.width > 0.0
                && rect.size.height > 0.0
                && orthogonal_segment_enters_node(rect, start, end)
            {
                #[cfg(feature = "diagnostic-traces")]
                if std::env::var_os("D2RS_TRACE_PORT_BALANCE").is_some() {
                    eprintln!(
                        "{}",
                        serde_json::json!({"event": "port-balance-obstruction", "edge": edge.tala_id.to_string(), "node": node.tala_id.to_string(), "node_is_member": graph.original_owner_member_nodes().contains(&node_id), "box": [origin.x, origin.y, rect.size.width, rect.size.height], "segment": [start,end]})
                    );
                }
                return false;
            }
        }
    }
    true
}

fn inspect_balance_quality(graph: &ArenaGraph) -> quality::Metrics {
    // quality.Inspect uses the same Graph.Nodes inventory for geometry,
    // labels and the scale that normalizes route length. Preserve canonical
    // endpoint/parent identity when compacting this current member view.
    let members = graph.original_owner_member_nodes();
    let indices = members
        .iter()
        .enumerate()
        .map(|(index, &node)| (node, index))
        .collect::<BTreeMap<_, _>>();
    let nodes = members
        .iter()
        .map(|&node_id| {
            let node = &graph.nodes[node_id.0 as usize];
            quality::Node {
                rect: Rect {
                    origin: node.position.unwrap_or_default(),
                    size: node.rect.size,
                },
                parent: node
                    .container
                    .and_then(|parent| indices.get(&parent).copied()),
                invisible: node.is_invisible,
                rectangular: matches!(
                    node.shape,
                    ShapeKind::Rectangle
                        | ShapeKind::Square
                        | ShapeKind::RealSquare
                        | ShapeKind::Image
                        | ShapeKind::Text
                        | ShapeKind::Code
                        | ShapeKind::Class
                        | ShapeKind::SqlTable
                ),
                container: node.is_container,
            }
        })
        .collect::<Vec<_>>();
    let edges = graph
        .edge_order
        .read()
        .iter()
        .map(|edge_id| {
            let edge = &graph.edge_records[edge_id.0 as usize];
            quality::Edge {
                from: indices[&edge.from],
                to: indices[&edge.to],
                points: edge.route.read().clone(),
                invisible: edge.is_invisible(),
                curve: edge.is_curve,
            }
        })
        .collect::<Vec<_>>();
    // Match quality.Inspect's label inventory.  The fixed-port pass is
    // admitted or rolled back based on text occlusions as well as route
    // geometry; omitting labels here makes a candidate appear cheaper than
    // the upstream Go pass and can publish a route that TALA restores.
    let mut labels = Vec::new();
    let mut append_label = |rect: Option<Rect>, owner| {
        if let Some(rect) = rect
            && rect.size.width > 0.0
            && rect.size.height > 0.0
        {
            labels.push(quality::Label { rect, owner });
        }
    };
    for (index, &node_id) in members.iter().enumerate() {
        let node = &graph.nodes[node_id.0 as usize];
        if node.is_invisible {
            continue;
        }
        if node.label_position != LabelPosition::Unset {
            append_label(
                crate::internal::labels::positioned_node_label_rect(graph, node_id),
                quality::Owner::Node(index),
            );
        }
        if node.has_icon && node.shape != ShapeKind::Image {
            append_label(
                crate::internal::labels::positioned_icon_label_rect(graph, node_id),
                quality::Owner::Node(index),
            );
        }
    }
    for (index, &edge_id) in graph.edge_order.read().iter().enumerate() {
        let edge = &graph.edge_records[edge_id.0 as usize];
        if edge.is_invisible() || edge.route.len() < 2 {
            continue;
        }
        if let Some(edge_label) = &edge.label
            && edge_label.position != LabelPosition::Unset
        {
            append_label(
                crate::internal::labels::label_rect(
                    edge,
                    edge_label.position,
                    edge_label.percentage,
                ),
                quality::Owner::Edge(index),
            );
        }
        for target in [false, true] {
            append_label(
                crate::internal::labels::arrowhead_label_rect_for_route(
                    edge,
                    &edge.route.read(),
                    target,
                ),
                quality::Owner::Edge(index),
            );
        }
    }
    quality::inspect(&nodes, &edges, &labels, |_| Ok::<(), ()>(()))
        .expect("balance quality inspection is finite after a routed graph")
}

/// Recovered `Graph.balanceRegularEdges` with caller-owned special and regular
/// slices. Standalone routing passes every fixed edge as special and every
/// requested edge as regular without the ordinary layout classification.
pub(in crate::internal) fn balance_regular_edges(
    graph: &mut ArenaGraph,
    special: &[usize],
    regular: &[usize],
) {
    for vertical in [true, false] {
        let mut locked = node_locked_segments(graph, vertical);
        // Go uses pointer identity for these fixed route points. TALA creates
        // each published point with Route.createSegmentEndpoints().Copy(), so
        // the matching identity in the arena is its edge and route-point slot.
        let fixed_port_points = fixed_port_point_ids(graph, regular);
        // Go seeds specialEdgeSegments with the selected routes' endpoint
        // segments before appending the caller's special edges. Keep those
        // fixed corridors in the lock set even though their coordinates are
        // excluded from the movable segment list.
        let mut special_segments = Vec::new();
        let mut segments = Vec::new();
        for segment in collect_balance_segments(graph, regular, vertical) {
            let geometry = canonical_segment(graph, segment, vertical);
            let endpoint_is_fixed = fixed_port_points.contains(&(segment.edge, segment.first))
                || fixed_port_points.contains(&(segment.edge, segment.second));
            if endpoint_is_fixed {
                special_segments.push(geometry);
                continue;
            }
            let connected_to_unit_node = [geometry.0, geometry.1].into_iter().any(|point| {
                graph.nodes.iter().any(|node| {
                    let Some(rect) = node_rect(graph, node.input_id) else {
                        return false;
                    };
                    rect.size.width == 1.0
                        && rect.size.height == 1.0
                        && (point.x == rect.origin.x || point.x == rect.right())
                        && (point.y == rect.origin.y || point.y == rect.bottom())
                })
            });
            if !connected_to_unit_node {
                segments.push(segment);
            }
        }
        special_segments.extend(
            collect_balance_segments(graph, special, vertical)
                .into_iter()
                .map(|segment| canonical_segment(graph, segment, vertical)),
        );
        let mut ignored = vec![false; segments.len()];

        // Segments sharing the coordinate of an overlapping special segment
        // become locked themselves, matching the recovered propagation pass.
        for (index, segment) in segments.iter().copied().enumerate() {
            let current = canonical_segment(graph, segment, vertical);
            if special_segments.iter().any(|locked_segment| {
                segments_overlap_along_length(current, *locked_segment, vertical, 1.0)
                    && (segment_coordinate(current, vertical)
                        - segment_coordinate(*locked_segment, vertical))
                    .abs()
                        <= 1.0
            }) {
                ignored[index] = true;
                special_segments.push(current);
            }
        }
        locked.extend(special_segments.clone());

        while ignored.iter().any(|ignored| !ignored) {
            let ranges: Vec<_> = segments
                .iter()
                .enumerate()
                .filter(|(index, _)| !ignored[*index])
                .map(|(index, segment)| {
                    (index, movement_bounds(graph, *segment, vertical, &locked))
                })
                .collect();
            let Some((_, min_range)) = ranges.iter().min_by(|(_, first), (_, second)| {
                (first.1 - first.0).total_cmp(&(second.1 - second.0))
            }) else {
                break;
            };
            let min_range = *min_range;
            let candidates: Vec<_> = ranges
                .iter()
                .filter(|(_, range)| *range == min_range)
                .map(|(index, _)| *index)
                .collect();
            for seed_index in candidates.iter().copied() {
                if ignored[seed_index] {
                    continue;
                }
                let seed = segments[seed_index];
                let seed_geometry = canonical_segment(graph, seed, vertical);
                let mut batch = vec![seed_index];
                for other_index in &candidates {
                    if *other_index == seed_index || ignored[*other_index] {
                        continue;
                    }
                    let other = segments[*other_index];
                    let buffer = if seed.edge == other.edge { 0.0 } else { 40.0 };
                    if segments_overlap_along_length(
                        seed_geometry,
                        canonical_segment(graph, other, vertical),
                        vertical,
                        buffer,
                    ) {
                        batch.push(*other_index);
                    }
                }
                for other_index in 0..segments.len() {
                    if ignored[other_index] || batch.contains(&other_index) {
                        continue;
                    }
                    if shared_balancing_cluster(graph, seed, segments[other_index], vertical) {
                        batch.push(other_index);
                        continue;
                    }
                    let other_geometry = canonical_segment(graph, segments[other_index], vertical);
                    if segments_overlap_along_length(seed_geometry, other_geometry, vertical, 1.0)
                        && (segment_coordinate(seed_geometry, vertical)
                            - segment_coordinate(other_geometry, vertical))
                        .abs()
                            <= 1.0
                    {
                        batch.push(other_index);
                    }
                }
                batch.sort_by(|first, second| {
                    let first_geometry = canonical_segment(graph, segments[*first], vertical);
                    let second_geometry = canonical_segment(graph, segments[*second], vertical);
                    let primary = if vertical {
                        segment_coordinate(first_geometry, true)
                            .total_cmp(&segment_coordinate(second_geometry, true))
                    } else {
                        segment_coordinate(first_geometry, false)
                            .total_cmp(&segment_coordinate(second_geometry, false))
                    };
                    primary.then_with(|| {
                        if vertical {
                            first_geometry.0.y.total_cmp(&second_geometry.0.y)
                        } else {
                            first_geometry.0.x.total_cmp(&second_geometry.0.x)
                        }
                    })
                });
                let distinct: BTreeSet<_> = batch
                    .iter()
                    .map(|index| {
                        rounded_balance_lane_key(segment_coordinate(
                            canonical_segment(graph, segments[*index], vertical),
                            vertical,
                        ))
                    })
                    .collect();
                let values = evenly_distribute(min_range.0, min_range.1, distinct.len());
                let batch_set: BTreeSet<_> = batch.iter().copied().collect();
                let original: Vec<_> = batch
                    .iter()
                    .map(|index| {
                        segment_coordinate_for_balance(
                            canonical_segment(graph, segments[*index], vertical),
                            vertical,
                        )
                    })
                    .collect();
                let mut proposed = original.clone();
                let mut value_index = 0;
                let mut segment_index = 0;
                while value_index < values.len() && segment_index < batch.len() {
                    let coordinate = segment_coordinate_for_balance(
                        canonical_segment(graph, segments[batch[segment_index]], vertical),
                        vertical,
                    );
                    let mut shared_count = 1;
                    while segment_index + shared_count < batch.len() {
                        let other_coordinate = segment_coordinate_for_balance(
                            canonical_segment(
                                graph,
                                segments[batch[segment_index + shared_count]],
                                vertical,
                            ),
                            vertical,
                        );
                        if (coordinate - other_coordinate).abs() <= 1.0 {
                            shared_count += 1;
                        } else {
                            break;
                        }
                    }
                    for value in proposed.iter_mut().skip(segment_index).take(shared_count) {
                        *value = values[value_index];
                    }
                    segment_index += shared_count;
                    value_index += 1;
                }
                let order = check_balance_order(
                    graph,
                    &batch,
                    &batch_set,
                    &segments,
                    &special_segments,
                    &proposed,
                    vertical,
                );
                // TALA's reversal fallback admits a batch when the proposed
                // move uncrosses the neighboring channel. The common
                // one-segment case is equivalent to moving toward the lower
                // coordinate; upward/rightward reversals require the full Go
                // crossing scorer and remain rejected conservatively.
                let reversal_uncrosses = order == BalanceOrder::Reversed
                    && reversal_removes_crossings(graph, &segments, &batch, &proposed, vertical);
                let accept = order == BalanceOrder::Preserved || reversal_uncrosses;
                #[cfg(feature = "diagnostic-traces")]
                if let Some(target) = crate::internal::trace_env_value("D2RS_TRACE_BALANCE_EDGE_ID")
                    && batch.iter().any(|index| {
                        graph.edge_records[segments[*index].edge]
                            .tala_id
                            .to_string()
                            == target
                    })
                {
                    eprint!(
                        "BALANCE_BATCH_RUST edge={} axis={} range={},{} order={:?} accept={} values={:?}",
                        target,
                        if vertical { "vertical" } else { "horizontal" },
                        min_range.0,
                        min_range.1,
                        order,
                        accept,
                        values
                    );
                    for (index, segment_index) in batch.iter().enumerate() {
                        let segment = segments[*segment_index];
                        let (start, end) = canonical_segment(graph, segment, vertical);
                        eprint!(
                            " edge={} segment={},{} original={} proposed={}",
                            graph.edge_records[segment.edge].tala_id,
                            segment.first,
                            segment.second,
                            segment_coordinate_for_balance((start, end), vertical),
                            proposed[index]
                        );
                    }
                    eprintln!();
                }
                if accept {
                    for (index, segment_index) in batch.iter().enumerate() {
                        set_segment_coordinate(
                            graph,
                            segments[*segment_index],
                            vertical,
                            proposed[index],
                        );
                    }
                }
                for index in batch {
                    ignored[index] = true;
                    locked.push(canonical_segment(graph, segments[index], vertical));
                }
            }
        }
    }
    for edge_index in regular.iter().copied() {
        graph.edge_records[edge_index].route.deduplicate();
    }
}

fn has_fixed_balancing_ports(graph: &ArenaGraph, edge_index: usize) -> bool {
    let edge = &graph.edge_records[edge_index];
    edge.has_table_column()
        || graph.nodes[edge.from.0 as usize].shape == ShapeKind::Diamond
        || graph.nodes[edge.to.0 as usize].shape == ShapeKind::Diamond
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, ShapeKind, Size};

    #[test]
    fn balancing_locks_rule_generated_and_one_unit_misaligned_routes() {
        let mut input = Graph::default();
        let source = input.add_node(Node::new(
            "source",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let target = input.add_node(Node::new(
            "target",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let edge = input.add_edge(Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        let index = edge.0 as usize;
        graph.edge_records[index]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: 0.0, y: 0.0 },
                Point { x: 50.0, y: 30.0 },
            ]));
        assert!(!is_special_edge_for_balancing(&graph, index));
        graph.edge_records[index].route.point_mut(1).x = 1.0;
        assert!(is_special_edge_for_balancing(&graph, index));
        graph.edge_records[index].route.point_mut(1).x = 50.0;
        graph.nodes[target.0 as usize].shape = ShapeKind::Diamond;
        assert!(is_special_edge_for_balancing(&graph, index));
        graph.nodes[target.0 as usize].shape = ShapeKind::Rectangle;
        graph.edge_records[index].source_table_column = Some(0);
        assert!(is_special_edge_for_balancing(&graph, index));
        graph.edge_records[index].source_table_column = None;
        graph.edge_records[index].to = source;
        assert!(is_special_edge_for_balancing(&graph, index));
    }

    #[test]
    fn balancing_obstacles_follow_current_graph_nodes_after_vessel_retirement() {
        let mut input = Graph::default();
        let source = input.add_node(Node::new(
            "source",
            Size {
                width: 100.0,
                height: 40.0,
            },
        ));
        let target = input.add_node(Node::new(
            "target",
            Size {
                width: 100.0,
                height: 40.0,
            },
        ));
        let retired = input.add_node(Node::new(
            "retired",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let edge = input.add_edge(Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[source.0 as usize].position = Some(Point { x: 0.0, y: -40.0 });
        graph.nodes[target.0 as usize].position = Some(Point { x: 0.0, y: 20.0 });
        graph.nodes[retired.0 as usize].position = Some(Point { x: 30.0, y: 5.0 });
        graph.set_owner_cluster_vessel_flag(retired, true).unwrap();
        // A vessel flag without its actual record is rejected by source
        // topology_preflight.go:1031, even if only retained Containers reach it.
        let kind = crate::internal::owner_groups::OwnerGroupKind::Cluster;
        let group = graph.new_owner_group(kind);
        let mut fields = graph.owner_group_fields(group).unwrap();
        fields.vessel = Some(retired);
        fields.graph = Some(crate::internal::OwnerGraphId(0));
        graph.set_owner_group_fields(group, fields).unwrap();
        let map = graph
            .owner_graph_group_map(crate::internal::OwnerGraphId(0), kind)
            .unwrap();
        graph
            .write_owner_group_map_entry(&map, Some(retired), Some(group))
            .unwrap();
        graph.node_order.retain(|node| *node != retired);
        graph.refresh_node_order_membership();
        graph.synchronize_original_owner_graph_view();
        graph.edge_records[edge.0 as usize]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: 40.0, y: 0.0 },
                Point { x: 40.0, y: 20.0 },
            ]));
        balance_edge_segments_with_budget(
            &mut graph,
            &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS),
        )
        .unwrap();
        assert_eq!(
            graph.edge_records[edge.0 as usize].route.read(),
            vec![Point { x: 50.0, y: 0.0 }, Point { x: 50.0, y: 20.0 }]
        );
        assert_eq!(
            graph.nodes[retired.0 as usize].position,
            Some(Point { x: 30.0, y: 5.0 })
        );

        // A reinserted allocated vessel is a real obstacle again. This is
        // slice membership, not a blanket exclusion of synthetic nodes.
        graph.node_order.push(retired);
        graph.refresh_node_order_membership();
        graph.synchronize_original_owner_graph_view();
        graph.edge_records[edge.0 as usize]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: 40.0, y: 0.0 },
                Point { x: 40.0, y: 20.0 },
            ]));
        balance_edge_segments_with_budget(
            &mut graph,
            &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS),
        )
        .unwrap();
        assert_eq!(
            graph.edge_records[edge.0 as usize].route.read(),
            vec![Point { x: 70.0, y: 0.0 }, Point { x: 70.0, y: 20.0 }]
        );
    }

    #[test]
    fn rounded_balance_lanes_match_go_float_map_zero_equality() {
        assert_eq!(
            BTreeSet::from([
                rounded_balance_lane_key(-0.4),
                rounded_balance_lane_key(0.4),
            ])
            .len(),
            1
        );
        assert_ne!(
            rounded_balance_lane_key(-0.5),
            rounded_balance_lane_key(0.5)
        );
        assert_ne!(
            rounded_balance_lane_key(-1.0),
            rounded_balance_lane_key(1.0)
        );
    }

    #[test]
    fn even_distribution_uses_go_indexed_float_operations() {
        let values = evenly_distribute(0.0, 56_763_892_615_818_120.0, 6);

        assert_eq!(values.len(), 6);
        // Go evaluates floor + float64(index)*increment. Accumulating the
        // increment six times produces adjacent IEEE-754 value ...93d.
        assert_eq!(values[5].to_bits(), 0x4365_9b68_08cb_e93c);
    }

    #[test]
    fn deduplication_preserves_a_two_point_route_after_full_collapse() {
        let point = Point { x: 12.0, y: -4.0 };
        let mut route = vec![point, point];

        deduplicate_route(&mut route);

        assert_eq!(route, vec![point, Point { x: 13.0, y: -4.0 }]);
    }

    #[test]
    fn fixed_port_validation_and_quality_ignore_detached_obstacles() {
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
        let obstacle = input.add_node(Node::new(
            "obstacle",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let edge = input.add_edge(Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        for (node, position) in [
            (source, Point { x: 0.0, y: 0.0 }),
            (target, Point { x: 200.0, y: 0.0 }),
            (obstacle, Point { x: 100.0, y: 90.0 }),
        ] {
            graph.set_position(node, position);
        }
        let route = |y| {
            vec![
                Point { x: 20.0, y: 10.0 },
                Point { x: 40.0, y: 10.0 },
                Point { x: 40.0, y },
                Point { x: 180.0, y },
                Point { x: 180.0, y: 10.0 },
                Point { x: 200.0, y: 10.0 },
            ]
        };
        let before = route(60.0);
        let candidate = route(100.0);
        graph.edge_records[edge.0 as usize]
            .route
            .replace(RoutePoints::literal(&(candidate.clone())));
        assert!(!changed_route_is_clear(
            &graph,
            edge.0 as usize,
            &candidate,
            &before,
            true
        ));
        assert!(inspect_balance_quality(&graph).route_obstructions > 0);

        graph.node_order.retain(|&node| node != obstacle);
        graph.invalidate_node_order_membership();
        assert!(changed_route_is_clear(
            &graph,
            edge.0 as usize,
            &candidate,
            &before,
            true
        ));
        let visible_quality = inspect_balance_quality(&graph);
        assert_eq!(visible_quality.route_obstructions, 0);
        // Detached geometry must not contribute to scale or label inspection.
        graph.nodes[obstacle.0 as usize].rect.size = Size {
            width: 1000.0,
            height: 1000.0,
        };
        graph.nodes[obstacle.0 as usize].label_size = Some(Size {
            width: 100.0,
            height: 100.0,
        });
        graph.nodes[obstacle.0 as usize].label_position = LabelPosition::InsideMiddleCenter;
        assert_eq!(inspect_balance_quality(&graph), visible_quality);

        graph.node_order.push(obstacle);
        graph.invalidate_node_order_membership();
        assert!(!changed_route_is_clear(
            &graph,
            edge.0 as usize,
            &candidate,
            &before,
            true
        ));
        assert!(inspect_balance_quality(&graph).route_obstructions > 0);
    }

    #[test]
    fn fixed_port_interior_pass_skips_curve_routes() {
        let mut input = Graph::default();
        let mut source = Node::new(
            "source",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        source.shape = ShapeKind::Diamond;
        source.locked_position = Some(Point { x: 0.0, y: 0.0 });
        let source = input.add_node(source);
        let mut target = Node::new(
            "target",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        target.shape = ShapeKind::Diamond;
        target.locked_position = Some(Point { x: 300.0, y: 0.0 });
        let target = input.add_node(target);
        let edge = input.add_edge(Edge { source, target });
        let route = vec![
            Point { x: 100.0, y: 50.0 },
            Point { x: 130.0, y: 50.0 },
            Point { x: 130.0, y: 200.0 },
            Point { x: 220.0, y: 200.0 },
            Point { x: 220.0, y: 300.0 },
            Point { x: 300.0, y: 300.0 },
            Point { x: 300.0, y: 50.0 },
        ];
        input.set_edge_route(edge, route.clone());
        input.set_edge_is_curve(edge, true);

        let mut curved = ArenaGraph::from_input(&input);
        assert!(curved.edge_records[edge.0 as usize].is_curve);
        balance_edge_segments(&mut curved);
        assert_eq!(curved.edge_records[edge.0 as usize].route.read(), route);

        input.set_edge_is_curve(edge, false);
        let mut orthogonal = ArenaGraph::from_input(&input);
        balance_edge_segments(&mut orthogonal);
        assert_ne!(orthogonal.edge_records[edge.0 as usize].route.read(), route);
    }

    #[test]
    fn fixed_port_identity_uses_route_slot_not_equal_coordinates() {
        let mut input = Graph::default();
        let mut fixed = Node::new(
            "fixed",
            Size {
                width: 20.0,
                height: 20.0,
            },
        );
        fixed.shape = ShapeKind::Diamond;
        let fixed = input.add_node(fixed);
        let source = input.add_node(Node::new(
            "source",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let target_a = input.add_node(Node::new(
            "target-a",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let target_b = input.add_node(Node::new(
            "target-b",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let fixed_edge = input.add_edge(Edge {
            source: fixed,
            target: target_a,
        });
        let other_edge = input.add_edge(Edge {
            source,
            target: target_b,
        });
        let shared_coordinate = Point { x: 50.0, y: 50.0 };
        input.set_edge_route(
            fixed_edge,
            vec![
                shared_coordinate,
                Point { x: 60.0, y: 50.0 },
                Point { x: 70.0, y: 50.0 },
                Point { x: 80.0, y: 50.0 },
            ],
        );
        input.set_edge_route(
            other_edge,
            vec![
                shared_coordinate,
                Point { x: 60.0, y: 60.0 },
                Point { x: 70.0, y: 60.0 },
                Point { x: 80.0, y: 60.0 },
            ],
        );
        let graph = ArenaGraph::from_input(&input);

        let fixed_ids =
            fixed_port_point_ids(&graph, &[fixed_edge.0 as usize, other_edge.0 as usize]);

        assert!(fixed_ids.contains(&(fixed_edge.0 as usize, 0)));
        assert!(!fixed_ids.contains(&(other_edge.0 as usize, 0)));
    }

    #[test]
    fn endpoint_contact_is_not_positive_segment_overlap() {
        let first = (Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 });
        let touching = (Point { x: 10.0, y: 0.0 }, Point { x: 20.0, y: 0.0 });
        let crossing = (Point { x: 9.0, y: 0.0 }, Point { x: 20.0, y: 0.0 });

        assert!(!segments_overlap_along_length(first, touching, false, 0.0));
        assert!(segments_overlap_along_length(first, touching, false, 40.0));
        assert!(segments_overlap_along_length(first, crossing, false, 0.0));
    }

    #[test]
    fn movement_bounds_include_an_obstacle_exactly_at_the_buffer_boundary() {
        let moving = (Point { x: 0.0, y: 50.0 }, Point { x: 20.0, y: 50.0 });
        let exact_boundary = (Point { x: 60.0, y: 80.0 }, Point { x: 80.0, y: 80.0 });

        assert!(intersects_movement_band(
            moving,
            exact_boundary,
            false,
            40.0
        ));
        assert!(!segments_overlap_along_length(
            moving,
            exact_boundary,
            false,
            40.0
        ));
    }

    #[test]
    fn point_segments_keep_the_default_movement_window_around_obstacles() {
        let point = Point {
            x: 2744.0,
            y: 2557.0,
        };
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
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_route(edge, vec![point, point]);
        let arena = ArenaGraph::from_input(&input);
        arena.edge_records[edge.0 as usize]
            .route
            .replace(RoutePoints::literal(&[point, point]));

        let bounds = movement_bounds(
            &arena,
            BalanceSegment {
                edge: edge.0 as usize,
                first: 0,
                second: 1,
            },
            false,
            &[(
                Point {
                    x: 2730.0,
                    y: 2400.0,
                },
                Point {
                    x: 2750.0,
                    y: 2400.0,
                },
            )],
        );

        assert_eq!(bounds, (2457.0, 2657.0));
    }
}
