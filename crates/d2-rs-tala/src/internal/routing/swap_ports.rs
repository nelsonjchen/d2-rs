// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Exchange of compatible route endpoints around crowded node borders.
//!
//! Local segment order and side geometry identify inversions that can be
//! corrected without rerunning complete path search.

use super::*;

#[cfg(feature = "diagnostic-traces")]
macro_rules! trace_tunnel_scan {
    ($enabled:expr, $event:expr) => {
        if $enabled {
            eprintln!("{}", $event);
        }
    };
}

#[cfg(not(feature = "diagnostic-traces"))]
macro_rules! trace_tunnel_scan {
    ($enabled:expr, $event:expr) => {{}};
}

#[cfg(feature = "diagnostic-traces")]
fn trace_tunnel_point(point: Point) -> serde_json::Value {
    serde_json::json!({
        "x_bits": format!("0x{:016x}", point.x.to_bits()),
        "y_bits": format!("0x{:016x}", point.y.to_bits()),
    })
}

#[cfg(feature = "diagnostic-traces")]
fn trace_tunnel_range(start: f64, end: f64) -> serde_json::Value {
    serde_json::json!({
        "start_bits": format!("0x{:016x}", start.to_bits()),
        "end_bits": format!("0x{:016x}", end.to_bits()),
    })
}

#[cfg(feature = "diagnostic-traces")]
fn trace_tunnel_ranges(ranges: &[(f64, f64)]) -> Vec<serde_json::Value> {
    ranges
        .iter()
        .map(|(start, end)| trace_tunnel_range(*start, *end))
        .collect()
}

fn source_float_order(first: f64, second: f64) -> std::cmp::Ordering {
    if first < second {
        std::cmp::Ordering::Less
    } else if second < first {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Equal
    }
}

fn source_stable_axis_order(
    first_primary: f64,
    second_primary: f64,
    first_secondary: f64,
    second_secondary: f64,
) -> std::cmp::Ordering {
    if first_primary == second_primary {
        source_float_order(first_secondary, second_secondary)
    } else {
        source_float_order(first_primary, second_primary)
    }
}

fn swap_trace_node_id(graph: &ArenaGraph, node: NodeId) -> String {
    let node = &graph.nodes[node.0 as usize];
    if node.external_id.is_empty() {
        node.tala_id.to_string()
    } else {
        node.external_id.clone()
    }
}

fn swap_trace_edge_id(graph: &ArenaGraph, edge_index: usize) -> String {
    let edge = &graph.edge_records[edge_index];
    let mut arrow = "-".to_owned();
    if edge.source_arrow {
        arrow.insert(0, '<');
    }
    if edge.target_arrow {
        arrow.push('>');
    }
    if !edge.source_arrow && !edge.target_arrow {
        arrow.push('-');
    }
    let mut from = swap_trace_node_id(graph, edge.from);
    if let Some(column) = edge.source_table_column {
        from.push_str(&format!("[{column}]"));
    }
    let mut to = swap_trace_node_id(graph, edge.to);
    if let Some(column) = edge.target_table_column {
        to.push_str(&format!("[{column}]"));
    }
    let label = edge
        .label
        .as_ref()
        .map(|label| format!(": {}", label.text))
        .unwrap_or_default();
    format!("{from} {arrow} {to}{label}")
}

fn swap_trace_route(points: &[Point]) -> Vec<serde_json::Value> {
    points
        .iter()
        .map(|point| {
            serde_json::json!({
                "x_bits": format!("0x{:016x}", point.x.to_bits()),
                "y_bits": format!("0x{:016x}", point.y.to_bits()),
            })
        })
        .collect()
}

fn has_large_arrowhead_label(graph: &ArenaGraph, edge_index: usize) -> bool {
    let edge = &graph.edge_records[edge_index];
    edge.source_arrowhead_label
        .as_ref()
        .is_some_and(|label| label.text.len() > 3)
        || edge
            .target_arrowhead_label
            .as_ref()
            .is_some_and(|label| label.text.len() > 3)
}

fn endpoint_indices(graph: &ArenaGraph, edge_index: usize, node: NodeId) -> (usize, usize, usize) {
    let edge = &graph.edge_records[edge_index];
    if edge.from == node {
        (0, 1, 2)
    } else {
        let len = edge.route.len();
        (len - 1, len - 2, len - 3)
    }
}

fn endpoint_side(graph: &ArenaGraph, edge_index: usize, node: NodeId) -> PortSide {
    let (first, second, _) = endpoint_indices(graph, edge_index, node);
    let p1 = graph.edge_records[edge_index].route.point(first);
    let p2 = graph.edge_records[edge_index].route.point(second);
    if p1.x < p2.x {
        PortSide::Right
    } else if p1.x > p2.x {
        PortSide::Left
    } else if p1.y < p2.y {
        PortSide::Bottom
    } else {
        PortSide::Top
    }
}

fn arrowhead_at(graph: &ArenaGraph, edge_index: usize, node: NodeId) -> &str {
    let edge = &graph.edge_records[edge_index];
    let (has_arrow, explicit_shape) = if edge.from == node {
        (edge.source_arrow, edge.source_arrowhead.as_deref())
    } else {
        (edge.target_arrow, edge.target_arrowhead.as_deref())
    };
    explicit_shape.unwrap_or(if has_arrow { "triangle" } else { "none" })
}

fn can_swap_ports(
    graph: &ArenaGraph,
    first_edge: usize,
    second_edge: usize,
    node: NodeId,
    port_counts: &[(Point, usize)],
) -> bool {
    let (first_index, _, _) = endpoint_indices(graph, first_edge, node);
    let (second_index, _, _) = endpoint_indices(graph, second_edge, node);
    let first_port = graph.edge_records[first_edge].route.point(first_index);
    let second_port = graph.edge_records[second_edge].route.point(second_index);
    // Go map reads return the value type's zero when a routed endpoint was not
    // present in the precomputed count map.
    if port_count(port_counts, first_port) == 1 && port_count(port_counts, second_port) == 1 {
        return true;
    }
    arrowhead_at(graph, first_edge, node) == arrowhead_at(graph, second_edge, node)
}

fn port_count(port_counts: &[(Point, usize)], point: Point) -> usize {
    port_counts
        .iter()
        .find(|(port, _)| *port == point)
        .map_or(0, |(_, count)| *count)
}

fn endpoint_segments_intersect(
    graph: &ArenaGraph,
    first_edge: usize,
    second_edge: usize,
    node: NodeId,
    side: PortSide,
) -> bool {
    let (p1_index, p2_index, p3_index) = endpoint_indices(graph, first_edge, node);
    let (q1_index, q2_index, q3_index) = endpoint_indices(graph, second_edge, node);
    let p1 = graph.edge_records[first_edge].route.point(p1_index);
    let p2 = graph.edge_records[first_edge].route.point(p2_index);
    let p3 = graph.edge_records[first_edge].route.point(p3_index);
    let q1 = graph.edge_records[second_edge].route.point(q1_index);
    let q2 = graph.edge_records[second_edge].route.point(q2_index);
    let q3 = graph.edge_records[second_edge].route.point(q3_index);
    match side {
        PortSide::Left => (q2.x < p2.x && p3.y > q1.y) || (q2.x > p2.x && q3.y < p1.y),
        PortSide::Bottom => (p2.y > q2.y && q3.x < p1.x) || (p2.y < q2.y && p3.x > q1.x),
        PortSide::Top => (p2.y < q2.y && q3.x < p1.x) || (p2.y > q2.y && p3.x > q1.x),
        PortSide::Right => (p2.x > q2.x && q3.y < p1.y) || (p2.x < q2.x && p3.y > q1.y),
    }
}

fn endpoint_segments_clear(
    graph: &ArenaGraph,
    node: NodeId,
    first_edge: usize,
    second_edge: usize,
    budget: Option<&super::route_search_guard::RouteStageWork>,
) -> Result<bool, ()> {
    let (p1_index, p2_index, _) = endpoint_indices(graph, first_edge, node);
    let (q1_index, q2_index, _) = endpoint_indices(graph, second_edge, node);
    let p1 = graph.edge_records[first_edge].route.point(p1_index);
    let p2 = graph.edge_records[first_edge].route.point(p2_index);
    let q1 = graph.edge_records[second_edge].route.point(q1_index);
    let q2 = graph.edge_records[second_edge].route.point(q2_index);
    for obstacle_id in graph.graph_node_order() {
        reserve_swap_work(budget, 1)?;
        let obstacle = &graph.nodes[obstacle_id.0 as usize];
        // doesNotPassThroughNodesGuarded visits its one ignored node for
        // every live graph node, including the node that causes the skip.
        reserve_swap_work(budget, 1)?;
        if obstacle_id == node
            || graph.is_descendant_of_scope(node, Some(obstacle_id))
            || obstacle.position.is_none()
        {
            continue;
        }
        let origin = obstacle.position.unwrap();
        let padded = Rect {
            origin: Point {
                x: origin.x - 10.0,
                y: origin.y - 10.0,
            },
            size: crate::Size {
                width: obstacle.rect.size.width + 20.0,
                height: obstacle.rect.size.height + 20.0,
            },
        };
        if super::route_geometry::recovered_segment_intersects_box(p1, p2, padded)
            || super::route_geometry::recovered_segment_intersects_box(q1, q2, padded)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn swap_endpoint_coordinate(
    graph: &mut ArenaGraph,
    first_edge: usize,
    second_edge: usize,
    node: NodeId,
    side: PortSide,
) {
    let (p1, p2, _) = endpoint_indices(graph, first_edge, node);
    let (q1, q2, _) = endpoint_indices(graph, second_edge, node);
    let (first, second) = if first_edge < second_edge {
        let (left, right) = graph.edge_records.split_at_mut(second_edge);
        (&mut left[first_edge], &mut right[0])
    } else {
        let (left, right) = graph.edge_records.split_at_mut(first_edge);
        (&mut right[0], &mut left[second_edge])
    };
    if matches!(side, PortSide::Top | PortSide::Bottom) {
        std::mem::swap(
            &mut first.route.point_mut(p1).x,
            &mut second.route.point_mut(q1).x,
        );
        std::mem::swap(
            &mut first.route.point_mut(p2).x,
            &mut second.route.point_mut(q2).x,
        );
    } else {
        std::mem::swap(
            &mut first.route.point_mut(p1).y,
            &mut second.route.point_mut(q1).y,
        );
        std::mem::swap(
            &mut first.route.point_mut(p2).y,
            &mut second.route.point_mut(q2).y,
        );
    }
}

pub(in crate::internal) fn node_rect(graph: &ArenaGraph, node: NodeId) -> Option<Rect> {
    Some(Rect {
        origin: graph.nodes[node.0 as usize].position?,
        size: graph.nodes[node.0 as usize].rect.size,
    })
}

fn node_blocks_between(obstacle: Rect, first: Rect, second: Rect, horizontal: bool) -> bool {
    if horizontal {
        if !(obstacle.origin.x >= first.right() && obstacle.right() <= second.origin.x) {
            return false;
        }
        obstacle.origin.y <= first.origin.y.max(second.origin.y)
            && obstacle.bottom() >= first.bottom().min(second.bottom())
    } else {
        if !(obstacle.origin.y >= first.bottom() && obstacle.bottom() <= second.origin.y) {
            return false;
        }
        obstacle.origin.x <= first.origin.x.max(second.origin.x)
            && obstacle.right() >= first.right().min(second.right())
    }
}

fn tunnel_ranges(
    graph: &ArenaGraph,
    first_node: NodeId,
    second_node: NodeId,
    filter_short: bool,
    budget: Option<&super::route_search_guard::RouteStageWork>,
) -> Result<(Vec<(f64, f64)>, bool), ()> {
    #[cfg(feature = "diagnostic-traces")]
    let trace_scan = crate::internal::trace_env_enabled("D2RS_TRACE_TUNNEL_SCAN_JSONL");

    // tunnelRangesBetweenChecked charges before its first visibility test.
    reserve_swap_work(budget, 1)?;
    let (Some(first), Some(second)) = (node_rect(graph, first_node), node_rect(graph, second_node))
    else {
        return Ok((Vec::new(), false));
    };
    let horizontal = first.origin.y <= second.bottom() && second.origin.y <= first.bottom();
    let mut vertical = false;
    if !horizontal {
        // The source checks vertical visibility only after horizontal fails.
        reserve_swap_work(budget, 1)?;
        vertical = first.origin.x <= second.right() && second.origin.x <= first.right();
    }
    trace_tunnel_scan!(
        trace_scan,
        serde_json::json!({
            "event": "tunnel-visibility",
            "from": swap_trace_node_id(graph, first_node),
            "to": swap_trace_node_id(graph, second_node),
            "horizontal": horizontal,
            "vertical": vertical,
        })
    );
    if !horizontal && !vertical {
        return Ok((Vec::new(), false));
    }

    // The source charges once before adding its initial range.
    reserve_swap_work(budget, 1)?;
    let (start, end) = if horizontal {
        (
            first.origin.y.max(second.origin.y),
            first.bottom().min(second.bottom()),
        )
    } else {
        let sequence_right = |node: NodeId, rect: Rect| {
            graph.nodes[node.0 as usize]
                .sequence
                .and_then(|index| graph.sequences.get(index))
                .filter(|sequence| sequence.members.last() != Some(&node))
                .map_or_else(|| rect.right(), |_| rect.right() - 35.0)
        };
        (
            first.origin.x.max(second.origin.x),
            sequence_right(first_node, first).min(sequence_right(second_node, second)),
        )
    };
    let mut ranges = vec![(start, end)];
    trace_tunnel_scan!(
        trace_scan,
        serde_json::json!({
            "event": "tunnel-initial-range",
            "from": swap_trace_node_id(graph, first_node),
            "to": swap_trace_node_id(graph, second_node),
            "range": trace_tunnel_range(start, end),
        })
    );

    // Go visits the current Graph.Nodes slice, rather than every stable arena
    // allocation retained by previous aggregate projections.
    for other_id in graph.graph_node_order_cow().iter().copied() {
        reserve_swap_work(budget, 1)?;
        if other_id == first_node || other_id == second_node {
            trace_tunnel_scan!(
                trace_scan,
                serde_json::json!({
                    "event": "tunnel-obstacle-skip",
                    "node": swap_trace_node_id(graph, other_id),
                    "reason": "endpoint",
                })
            );
            continue;
        }

        // The source computes all four relations in order, even when an
        // earlier relation is already true.
        let other_below_first = is_descendant_with_route_work(graph, other_id, first_node, budget)?;
        let first_below_other = is_descendant_with_route_work(graph, first_node, other_id, budget)?;
        let other_below_second =
            is_descendant_with_route_work(graph, other_id, second_node, budget)?;
        let second_below_other =
            is_descendant_with_route_work(graph, second_node, other_id, budget)?;
        trace_tunnel_scan!(
            trace_scan,
            serde_json::json!({
                "event": "tunnel-obstacle-scope",
                "node": swap_trace_node_id(graph, other_id),
                "other_below_from": other_below_first,
                "from_below_other": first_below_other,
                "other_below_to": other_below_second,
                "to_below_other": second_below_other,
            })
        );
        if other_below_first || first_below_other || other_below_second || second_below_other {
            continue;
        }
        let Some(obstacle) = node_rect(graph, other_id) else {
            trace_tunnel_scan!(
                trace_scan,
                serde_json::json!({
                    "event": "tunnel-obstacle-skip",
                    "node": swap_trace_node_id(graph, other_id),
                    "reason": "unpositioned",
                })
            );
            continue;
        };
        let blocks_first = node_blocks_between(obstacle, first, second, horizontal);
        let blocks_second =
            !blocks_first && node_blocks_between(obstacle, second, first, horizontal);
        let blocked = blocks_first || blocks_second;
        trace_tunnel_scan!(
            trace_scan,
            serde_json::json!({
                "event": "tunnel-obstacle-blocked",
                "node": swap_trace_node_id(graph, other_id),
                "blocked": blocked,
            })
        );
        if blocked {
            return Ok((Vec::new(), false));
        }
        let between = if horizontal {
            (first.origin.x < obstacle.origin.x && obstacle.origin.x < second.origin.x)
                || (second.origin.x < obstacle.origin.x && obstacle.origin.x < first.origin.x)
        } else {
            (first.origin.y < obstacle.origin.y && obstacle.origin.y < second.origin.y)
                || (second.origin.y < obstacle.origin.y && obstacle.origin.y < first.origin.y)
        };
        trace_tunnel_scan!(
            trace_scan,
            serde_json::json!({
                "event": "tunnel-obstacle-position",
                "node": swap_trace_node_id(graph, other_id),
                "between": between,
            })
        );
        if !between {
            continue;
        }
        let cut = if horizontal {
            (obstacle.origin.y, obstacle.bottom())
        } else {
            (obstacle.origin.x, obstacle.right())
        };
        let mut next = Vec::new();
        // Traces report the index.
        #[cfg_attr(
            not(feature = "diagnostic-traces"),
            allow(clippy::unused_enumerate_index)
        )]
        for (_range_index, (start, end)) in ranges.into_iter().enumerate() {
            // tunnelRangesBetweenChecked charges once for each range visited.
            reserve_swap_work(budget, 1)?;
            let (_action, replacement) = if cut.0 <= start && cut.1 >= end {
                ("deleted", Vec::new())
            } else if start < cut.0 && cut.1 < end {
                ("split", vec![(start, cut.0), (cut.1, end)])
            } else if cut.0 <= start && start < cut.1 && cut.1 < end {
                ("clip-start", vec![(cut.1, end)])
            } else if start < cut.0 && cut.0 < end && cut.1 >= end {
                ("clip-end", vec![(start, cut.0)])
            } else {
                ("unchanged", vec![(start, end)])
            };
            trace_tunnel_scan!(
                trace_scan,
                serde_json::json!({
                    "event": "tunnel-range-cut",
                    "node": swap_trace_node_id(graph, other_id),
                    "range_index": _range_index,
                    "action": _action,
                    "before": trace_tunnel_range(start, end),
                    "cut": trace_tunnel_range(cut.0, cut.1),
                    "after": trace_tunnel_ranges(&replacement),
                })
            );
            next.extend(replacement);
        }
        // Go polls cancellation after each qualifying obstacle's range
        // subtraction, before charging and filtering replacement ranges.
        reserve_swap_work(budget, 0)?;
        let mut retained = Vec::new();
        for (start, end) in next {
            // The source charges once for every candidate range before its
            // short-tunnel filter, including ranges it then discards.
            reserve_swap_work(budget, 1)?;
            let keep = !filter_short || end - start >= 40.0;
            trace_tunnel_scan!(
                trace_scan,
                serde_json::json!({
                    "event": "tunnel-range-filter",
                    "range": trace_tunnel_range(start, end),
                    "keep": keep,
                })
            );
            if keep {
                retained.push((start, end));
            }
        }
        ranges = retained;
        trace_tunnel_scan!(
            trace_scan,
            serde_json::json!({
                "event": "tunnel-ranges-after-obstacle",
                "node": swap_trace_node_id(graph, other_id),
                "ranges": trace_tunnel_ranges(&ranges),
            })
        );
    }
    trace_tunnel_scan!(
        trace_scan,
        serde_json::json!({
            "event": "tunnel-ranges-result",
            "horizontal": horizontal,
            "ranges": trace_tunnel_ranges(&ranges),
        })
    );
    Ok((ranges, horizontal))
}

fn is_descendant_with_route_work(
    graph: &ArenaGraph,
    descendant: NodeId,
    ancestor: NodeId,
    budget: Option<&super::route_search_guard::RouteStageWork>,
) -> Result<bool, ()> {
    let Some(budget) = budget else {
        return Ok(graph.is_descendant_of_scope(descendant, Some(ancestor)));
    };

    // Match isDescendantOfWithRouteGuard's step before each ownership-chain
    // identity check, including the final root whose parent is absent.
    let mut current = descendant;
    loop {
        reserve_swap_work(Some(budget), 1)?;
        if current == ancestor {
            return Ok(true);
        }
        let node = &graph.nodes[current.0 as usize];
        let parent = if let Some(container) = node.container {
            Some(container)
        } else if let Some(cluster) = node.cluster {
            let vessel_tala_id = graph.clusters[cluster].vessel_tala_id;
            graph
                .nodes
                .iter()
                .find(|candidate| candidate.tala_id == vessel_tala_id)
                .map(|candidate| candidate.input_id)
        } else if let Some(sequence) = node.sequence {
            let vessel_tala_id = graph.sequences[sequence].vessel_tala_id;
            graph
                .nodes
                .iter()
                .find(|candidate| candidate.tala_id == vessel_tala_id)
                .map(|candidate| candidate.input_id)
        } else {
            None
        };
        let Some(parent) = parent else {
            return Ok(false);
        };
        current = parent;
    }
}

fn is_u_shaped(points: &[Point]) -> bool {
    if points.len() != 4 {
        return false;
    }
    if points[0].x == points[1].x {
        (points[0].y - points[1].y).signum() == (points[3].y - points[2].y).signum()
            && points[1].y == points[2].y
    } else {
        (points[0].x - points[1].x).signum() == (points[3].x - points[2].x).signum()
            && points[2].x == points[1].x
    }
}

fn is_s_shaped(points: &[Point]) -> bool {
    points.len() == 4 && !is_u_shaped(points)
}

fn is_l_shaped(points: &[Point]) -> bool {
    if points.len() != 3 {
        return false;
    }
    let first_vertical = points[0].x == points[1].x;
    let second_horizontal = points[1].y == points[2].y;
    first_vertical == second_horizontal
}

fn attached_ports(graph: &ArenaGraph, node: NodeId) -> Vec<Point> {
    let edges = &graph.nodes[node.0 as usize].edges;
    let mut ports = Vec::with_capacity(edges.len());
    for edge_id in edges.read() {
        let edge = &graph.edge_records[edge_id.0 as usize];
        if edge.route.is_empty() {
            continue;
        }
        let _route_values = edge.route.read();
        let point = if edge.from == node {
            edge.route.point(0)
        } else {
            edge.route.point(edge.route.len() - 1)
        };
        // Go uses geo.Point as a map key. Numeric float equality makes
        // positive and negative zero the same occupied port.
        if !ports.contains(&point) {
            ports.push(point);
        }
    }
    ports
}

fn make_straight_line(
    graph: &mut ArenaGraph,
    edge_index: usize,
    budget: Option<&super::route_search_guard::RouteStageWork>,
) -> Result<bool, ()> {
    #[cfg(feature = "diagnostic-traces")]
    let trace_scan = crate::internal::trace_env_enabled("D2RS_TRACE_TUNNEL_SCAN_JSONL");

    let edge = &graph.edge_records[edge_index];
    if edge.route.len() < 2 {
        return Ok(false);
    }
    let from = edge.from;
    let to = edge.to;
    let from_port = edge.route.point(0);
    let to_port = edge.route.point(edge.route.len() - 1);
    let mut occupied = attached_ports(graph, from);
    for point in attached_ports(graph, to) {
        if !occupied.contains(&point) {
            occupied.push(point);
        }
    }
    trace_tunnel_scan!(
        trace_scan,
        serde_json::json!({
            "event": "straight-line-begin",
            "edge": swap_trace_edge_id(graph, edge_index),
            "route": swap_trace_route(&graph.edge_records[edge_index].route.read()),
        })
    );
    let (ranges, horizontal) = tunnel_ranges(graph, from, to, false, budget)?;
    // Traces report the index.
    #[cfg_attr(
        not(feature = "diagnostic-traces"),
        allow(clippy::unused_enumerate_index)
    )]
    for (_range_index, (start, end)) in ranges.into_iter().enumerate() {
        // makeStraightLineGuarded charges before visiting each range.
        reserve_swap_work(budget, 1)?;
        trace_tunnel_scan!(
            trace_scan,
            serde_json::json!({
                "event": "straight-line-range",
                "range_index": _range_index,
                "range": trace_tunnel_range(start, end),
            })
        );
        let mut value = start + 5.0;
        #[cfg(feature = "diagnostic-traces")]
        let mut candidate_index = 0;
        while value <= end - 5.0 {
            // The candidate work unit is charged before port occupancy tests.
            reserve_swap_work(budget, 1)?;
            let candidate_from = if horizontal {
                Point {
                    x: from_port.x,
                    y: value,
                }
            } else {
                Point {
                    x: value,
                    y: from_port.y,
                }
            };
            let candidate_to = if horizontal {
                Point {
                    x: to_port.x,
                    y: value,
                }
            } else {
                Point {
                    x: value,
                    y: to_port.y,
                }
            };
            let from_occupied = occupied.contains(&candidate_from);
            let to_occupied = occupied.contains(&candidate_to);
            let accepted = !from_occupied && !to_occupied;
            trace_tunnel_scan!(
                trace_scan,
                serde_json::json!({
                    "event": "straight-line-candidate",
                    "range_index": _range_index,
                    "candidate_index": candidate_index,
                    "from": trace_tunnel_point(candidate_from),
                    "to": trace_tunnel_point(candidate_to),
                    "from_occupied": from_occupied,
                    "to_occupied": to_occupied,
                    "accepted": accepted,
                })
            );
            if accepted {
                graph.edge_records[edge_index]
                    .route
                    .replace(RoutePoints::literal(&[candidate_from, candidate_to]));
                trace_tunnel_scan!(
                    trace_scan,
                    serde_json::json!({
                        "event": "straight-line-result",
                        "changed": true,
                        "route": swap_trace_route(&graph.edge_records[edge_index].route.read()),
                    })
                );
                return Ok(true);
            }
            value += 5.0;
            #[cfg(feature = "diagnostic-traces")]
            {
                candidate_index += 1;
            }
        }
    }
    trace_tunnel_scan!(
        trace_scan,
        serde_json::json!({
            "event": "straight-line-result",
            "changed": false,
            "route": swap_trace_route(&graph.edge_records[edge_index].route.read()),
        })
    );
    Ok(false)
}

fn distinct_node_port_count(graph: &ArenaGraph, node: NodeId) -> usize {
    let mut ports = Vec::new();
    for edge_id in &graph.nodes[node.0 as usize].edges.read() {
        let edge = &graph.edge_records[edge_id.0 as usize];
        let route_values = edge.route.read();
        let point = if edge.from == node {
            route_values.first()
        } else {
            route_values.last()
        };
        if let Some(point) = point
            && !ports.contains(point)
        {
            ports.push(*point);
        }
    }
    ports.len()
}

pub(in crate::internal) fn refine_edge(
    graph: &mut ArenaGraph,
    edge_index: usize,
    budget: Option<&super::route_search_guard::RouteStageWork>,
) -> Result<bool, ()> {
    reserve_swap_work(budget, 1)?;
    let edge = &graph.edge_records[edge_index];
    let trace_target = crate::internal::trace_env_value("TALA_TRACE_SWAP_EDGE");
    let trace_refine = crate::internal::trace_env_enabled("TALA_TRACE_SWAP_PORTS")
        && trace_target
            .as_deref()
            .is_none_or(|target| target.parse::<i64>().ok() == Some(edge.tala_id));
    if edge.from == edge.to
        || edge.route.len() <= 2
        || edge.is_between_table_columns()
        || graph.edge_touches_tree_node(crate::EdgeId(edge_index as u32))
    {
        return Ok(false);
    }
    let from = edge.from;
    let to = edge.to;
    // refineEdgeGuarded gathers occupied ports independently from the two
    // endpoint Nodes, then visits each distinct port on the target.
    reserve_swap_stage_optional_steps(budget, graph.nodes[from.0 as usize].edges.len())?;
    reserve_swap_stage_optional_steps(budget, graph.nodes[to.0 as usize].edges.len())?;
    reserve_swap_stage_optional_steps(budget, distinct_node_port_count(graph, to))?;
    if is_s_shaped(&edge.route.read()) || is_l_shaped(&edge.route.read()) {
        return make_straight_line(graph, edge_index, budget);
    }
    if is_u_shaped(&edge.route.read()) {
        return Ok(false);
    }
    if edge.route.len() >= 4 && is_s_shaped(&edge.route.read()[..4]) {
        let points = edge.route.read().clone();
        let bend = if points[0].x == points[1].x {
            Point {
                x: points[1].x,
                y: points[3].y,
            }
        } else {
            Point {
                x: points[3].x,
                y: points[1].y,
            }
        };
        if trace_refine {
            eprintln!(
                "SWAP_REFINE_PREFIX_RUST edge={} route={:?} bend={:?}",
                edge.tala_id,
                edge.route.read(),
                bend
            );
        }
        let mut blocked_by = None;
        let mut clear = true;
        for obstacle_id in graph.graph_node_order() {
            reserve_swap_work(budget, 1)?;
            let obstacle = &graph.nodes[obstacle_id.0 as usize];
            let mut ignored = false;
            for endpoint in [from, to] {
                reserve_swap_work(budget, 1)?;
                if obstacle_id == endpoint
                    || graph.is_descendant_of_scope(endpoint, Some(obstacle_id))
                {
                    ignored = true;
                    break;
                }
            }
            if ignored || obstacle.position.is_none() {
                continue;
            }
            let rect = node_rect(graph, obstacle_id).unwrap();
            let padded = Rect {
                origin: Point {
                    x: rect.origin.x - 10.0,
                    y: rect.origin.y - 10.0,
                },
                size: crate::Size {
                    width: rect.size.width + 20.0,
                    height: rect.size.height + 20.0,
                },
            };
            let first_touches =
                super::route_geometry::recovered_segment_intersects_box(points[0], bend, padded);
            let second_touches =
                super::route_geometry::recovered_segment_intersects_box(bend, points[3], padded);
            if trace_refine && (first_touches || second_touches) {
                blocked_by = Some((obstacle.tala_id, padded, first_touches, second_touches));
            }
            if first_touches || second_touches {
                clear = false;
                break;
            }
        }
        if trace_refine {
            eprintln!(
                "SWAP_REFINE_CLEAR_RUST edge={} clear={} blocked_by={:?}",
                graph.edge_records[edge_index].tala_id, clear, blocked_by
            );
        }
        if clear {
            let removed_route_length = graph.edge_records[edge_index].route.len();
            // removeEdgePointsGuarded stops checking removal indices as soon
            // as one matches, so the removed points cost fewer comparisons.
            // Mirror its nested loops to retain the source work limit.
            let mut removal_work = 0;
            for point_index in 0..removed_route_length {
                removal_work += 1;
                for remove_index in [1, 2, 3] {
                    removal_work += 1;
                    if point_index == remove_index {
                        break;
                    }
                }
            }
            reserve_swap_stage_optional_steps(budget, removal_work)?;
            // insertEdgePointGuarded then visits the two remaining points.
            reserve_swap_stage_optional_steps(budget, removed_route_length - 3)?;
            let edge = &mut graph.edge_records[edge_index];
            edge.route.remove_fresh(1..4);
            edge.route.insert_fresh(1, bend);
            return Ok(true);
        }
    }
    Ok(false)
}

/// Direct translation of D2 `geo.IntersectionPoint`.
///
/// SwapEdgePorts deliberately receives the integer-rounded intersection from
/// this general segment formula. Constructing the mathematically exact
/// orthogonal point changes half-pixel routes before `refine_edge`.
fn intersection_point(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    d2_rs_core::shape_geometry::line_intersection_point(a, b, c, d)
}

fn swap_endpoint_points(
    graph: &mut ArenaGraph,
    first_edge: usize,
    second_edge: usize,
    node: NodeId,
) {
    let (p1, p2, _) = endpoint_indices(graph, first_edge, node);
    let (q1, q2, _) = endpoint_indices(graph, second_edge, node);
    let (first, second) = if first_edge < second_edge {
        let (left, right) = graph.edge_records.split_at_mut(second_edge);
        (&mut left[first_edge], &mut right[0])
    } else {
        let (left, right) = graph.edge_records.split_at_mut(first_edge);
        (&mut right[0], &mut left[second_edge])
    };
    first.route.swap_slots(p1, &second.route, q1);
    first.route.swap_slots(p2, &second.route, q2);
}

fn swap_adjacent_side_ports(
    graph: &mut ArenaGraph,
    node: NodeId,
    sides: &BTreeMap<PortSide, Vec<usize>>,
    port_counts: &[(Point, usize)],
    trace_jsonl: bool,
    budget: Option<&super::route_search_guard::RouteStageWork>,
) -> Result<bool, ()> {
    let top_bottom: Vec<_> = sides
        .get(&PortSide::Top)
        .into_iter()
        .chain(sides.get(&PortSide::Bottom))
        .flatten()
        .copied()
        .collect();
    let left_right: Vec<_> = sides
        .get(&PortSide::Left)
        .into_iter()
        .chain(sides.get(&PortSide::Right))
        .flatten()
        .copied()
        .collect();
    let mut swapped = false;
    let trace_swap_target = crate::internal::trace_env_value("TALA_TRACE_SWAP_NODE");
    let node_tala_id = graph.nodes[node.0 as usize].tala_id;
    let node_external_id = graph.nodes[node.0 as usize].external_id.clone();
    let trace_node = crate::internal::trace_env_enabled("TALA_TRACE_SWAP_PORTS")
        && trace_swap_target.as_deref().is_none_or(|target| {
            target == node_external_id || target.parse::<i64>().ok() == Some(node_tala_id)
        });
    'vertical: for first_edge in top_bottom {
        for second_edge in &left_right {
            let second_edge = *second_edge;
            if graph.edge_records[first_edge].route.len() <= 2 {
                if trace_jsonl {
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "event": "swap-port-adjacent-skip",
                            "node": swap_trace_node_id(graph, node),
                            "a": swap_trace_edge_id(graph, first_edge),
                            "b": swap_trace_edge_id(graph, second_edge),
                            "reason": "short-a-route",
                        })
                    );
                }
                continue 'vertical;
            }
            if graph.edge_records[second_edge].route.len() <= 2 {
                if trace_jsonl {
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "event": "swap-port-adjacent-skip",
                            "node": swap_trace_node_id(graph, node),
                            "a": swap_trace_edge_id(graph, first_edge),
                            "b": swap_trace_edge_id(graph, second_edge),
                            "reason": "short-b-route",
                        })
                    );
                }
                continue;
            }
            let can_swap = can_swap_ports(graph, first_edge, second_edge, node, port_counts);
            let a_before = trace_jsonl.then(|| graph.edge_records[first_edge].route.read().clone());
            let b_before =
                trace_jsonl.then(|| graph.edge_records[second_edge].route.read().clone());
            if trace_jsonl {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "swap-port-adjacent-candidate",
                        "node": swap_trace_node_id(graph, node),
                        "a": swap_trace_edge_id(graph, first_edge),
                        "b": swap_trace_edge_id(graph, second_edge),
                        "can_swap": can_swap,
                        "a_before": swap_trace_route(a_before.as_ref().unwrap()),
                        "b_before": swap_trace_route(b_before.as_ref().unwrap()),
                    })
                );
            }
            if trace_node {
                eprintln!(
                    "SWAP_ADJACENT_CANDIDATE_RUST node={} a={} b={} can_swap={} a_points={:?} b_points={:?}",
                    node_tala_id,
                    graph.edge_records[first_edge].tala_id,
                    graph.edge_records[second_edge].tala_id,
                    can_swap,
                    graph.edge_records[first_edge].route.read(),
                    graph.edge_records[second_edge].route.read(),
                );
            }
            if !can_swap {
                if trace_jsonl {
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "event": "swap-port-adjacent-skip",
                            "node": swap_trace_node_id(graph, node),
                            "a": swap_trace_edge_id(graph, first_edge),
                            "b": swap_trace_edge_id(graph, second_edge),
                            "reason": "ports",
                        })
                    );
                }
                continue;
            }
            let (_, a2, a3) = endpoint_indices(graph, first_edge, node);
            let (_, b2, b3) = endpoint_indices(graph, second_edge, node);
            let intersection = intersection_point(
                graph.edge_records[first_edge].route.point(a2),
                graph.edge_records[first_edge].route.point(a3),
                graph.edge_records[second_edge].route.point(b2),
                graph.edge_records[second_edge].route.point(b3),
            );
            let Some(intersection) = intersection else {
                if trace_jsonl {
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "event": "swap-port-adjacent-skip",
                            "node": swap_trace_node_id(graph, node),
                            "a": swap_trace_edge_id(graph, first_edge),
                            "b": swap_trace_edge_id(graph, second_edge),
                            "reason": "no-intersection",
                        })
                    );
                }
                if trace_node {
                    eprintln!(
                        "SWAP_ADJACENT_SKIP_RUST node={} a={} b={} reason=no-intersection",
                        node_tala_id,
                        graph.edge_records[first_edge].tala_id,
                        graph.edge_records[second_edge].tala_id,
                    );
                }
                continue;
            };
            if trace_node {
                eprintln!(
                    "SWAP_ADJACENT_INTERSECTION_RUST node={} a={} b={} at={:?}",
                    node_tala_id,
                    graph.edge_records[first_edge].tala_id,
                    graph.edge_records[second_edge].tala_id,
                    intersection,
                );
            }
            if trace_jsonl {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "swap-port-adjacent-intersection",
                        "node": swap_trace_node_id(graph, node),
                        "a": swap_trace_edge_id(graph, first_edge),
                        "b": swap_trace_edge_id(graph, second_edge),
                        "x_bits": format!("0x{:016x}", intersection.x.to_bits()),
                        "y_bits": format!("0x{:016x}", intersection.y.to_bits()),
                    })
                );
            }
            reserve_swap_stage_optional_steps(budget, graph.edge_records[first_edge].route.len())?;
            reserve_swap_stage_optional_steps(budget, graph.edge_records[second_edge].route.len())?;
            let first_backup = graph.edge_records[first_edge].route.read().clone();
            let second_backup = graph.edge_records[second_edge].route.read().clone();
            swap_endpoint_points(graph, first_edge, second_edge, node);
            for edge_index in [first_edge, second_edge] {
                reserve_swap_stage_optional_steps(
                    budget,
                    graph.edge_records[edge_index].route.len(),
                )?;
                let from_node = graph.edge_records[edge_index].from == node;
                let insert_at = if from_node {
                    2
                } else {
                    graph.edge_records[edge_index].route.len() - 2
                };
                graph.edge_records[edge_index]
                    .route
                    .insert_fresh(insert_at, intersection);
                let (_, second, _) = endpoint_indices(graph, edge_index, node);
                let intersection_index = if from_node {
                    2
                } else {
                    graph.edge_records[edge_index].route.len() - 3
                };
                if graph.edge_records[edge_index]
                    .route
                    .point_mut(intersection_index)
                    .x
                    == graph.edge_records[edge_index].route.point(second).x
                {
                    graph.edge_records[edge_index]
                        .route
                        .point_mut(intersection_index)
                        .x += 2.5;
                    graph.edge_records[edge_index].route.point_mut(second).x += 2.5;
                } else {
                    graph.edge_records[edge_index]
                        .route
                        .point_mut(intersection_index)
                        .y -= 2.5;
                    graph.edge_records[edge_index].route.point_mut(second).y -= 2.5;
                }
            }
            let first_improved = refine_edge(graph, first_edge, budget)?;
            let second_improved = refine_edge(graph, second_edge, budget)?;
            let accepted = first_improved || second_improved;
            if !accepted {
                graph.edge_records[first_edge]
                    .route
                    .replace(RoutePoints::literal(&(first_backup.clone())));
                graph.edge_records[second_edge]
                    .route
                    .replace(RoutePoints::literal(&(second_backup.clone())));
            }
            if trace_jsonl {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "swap-port-adjacent-result",
                        "node": swap_trace_node_id(graph, node),
                        "a": swap_trace_edge_id(graph, first_edge),
                        "b": swap_trace_edge_id(graph, second_edge),
                        "a_improved": first_improved,
                        "b_improved": second_improved,
                        "accepted": accepted,
                        "a_before": swap_trace_route(a_before.as_ref().unwrap()),
                        "b_before": swap_trace_route(b_before.as_ref().unwrap()),
                        "a_after": swap_trace_route(&graph.edge_records[first_edge].route.read()),
                        "b_after": swap_trace_route(&graph.edge_records[second_edge].route.read()),
                    })
                );
            }
            if trace_node {
                eprintln!(
                    "SWAP_ADJACENT_RESULT_RUST node={} a={} b={} improved={},{} a_points={:?} b_points={:?}",
                    node_tala_id,
                    graph.edge_records[first_edge].tala_id,
                    graph.edge_records[second_edge].tala_id,
                    first_improved,
                    second_improved,
                    graph.edge_records[first_edge].route.read(),
                    graph.edge_records[second_edge].route.read(),
                );
            }
            if accepted {
                if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_PORTS") {
                    eprintln!(
                        "SWAP_ADJACENT_RUST node={} first={}>{} second={}>{} first_points={:?} second_points={:?}",
                        graph.nodes[node.0 as usize].tala_id,
                        graph.nodes[graph.edge_records[first_edge].from.0 as usize].tala_id,
                        graph.nodes[graph.edge_records[first_edge].to.0 as usize].tala_id,
                        graph.nodes[graph.edge_records[second_edge].from.0 as usize].tala_id,
                        graph.nodes[graph.edge_records[second_edge].to.0 as usize].tala_id,
                        graph.edge_records[first_edge].route.read(),
                        graph.edge_records[second_edge].route.read(),
                    );
                }
                swapped = true;
            }
        }
    }
    Ok(swapped)
}

/// Translation of recovered `Node.SwapEdgePorts`, including same-side
/// uncrossing and adjacent-side tunnel refinement with rollback.
pub(in crate::internal) fn swap_edge_ports(graph: &mut ArenaGraph) -> bool {
    super::swap_owner::swap_edge_ports(graph)
}

fn swap_edge_ports_inner(
    graph: &mut ArenaGraph,
    budget: Option<&super::route_search_guard::RouteStageWork>,
) -> Result<bool, ()> {
    let mut swapped = false;
    let trace_jsonl = crate::internal::trace_env_enabled("D2RS_TRACE_SWAP_PORTS_JSONL");
    // `SwapAllEdgePorts` visits the current Graph.Nodes slice, not every
    // stable arena allocation. The arena retains hidden aggregate members so
    // later stages can restore them, but they are absent from the source
    // slice while their vessel is active.
    let node_ids = graph.graph_node_order();
    for node in node_ids {
        let node_id = swap_trace_node_id(graph, node);
        let mut node_swapped = false;
        if trace_jsonl {
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "swap-port-node",
                    "phase": "begin",
                    "node": node_id,
                })
            );
        }
        let eligible: Vec<_> = graph.nodes[node.0 as usize]
            .edges
            .read()
            .iter()
            .map(|edge| edge.0 as usize)
            .filter(|edge_index| {
                let edge = &graph.edge_records[*edge_index];
                // Sequence vessels retain an aggregate external-edge slice
                // on their first member while Rust keeps the serialized
                // endpoints attached to the original members. Go's
                // SwapEdgePorts sees a real vessel edge whose endpoint is the
                // vessel; it never treats an edge belonging to another
                // sequence member as incident to this node. Restrict this
                // stage to the actual endpoint relation before applying the
                // recovered port predicates.
                (edge.from == node || edge.to == node)
                    && edge.from != edge.to
                    && edge.route.len() >= 3
                    && !edge.has_table_column()
                    && !has_large_arrowhead_label(graph, *edge_index)
            })
            .collect();
        let mut port_counts = Vec::new();
        let mut sides: BTreeMap<PortSide, Vec<usize>> = BTreeMap::new();
        let mut side_order = Vec::new();
        for edge_index in eligible {
            let (port_index, _, _) = endpoint_indices(graph, edge_index, node);
            let point = graph.edge_records[edge_index].route.point(port_index);
            if let Some((_, count)) = port_counts.iter_mut().find(|(port, _)| *port == point) {
                *count += 1;
            } else {
                port_counts.push((point, 1));
            }
            let side = endpoint_side(graph, edge_index, node);
            if !sides.contains_key(&side) {
                // Go groups sides in a map and later ranges that map. Preserve
                // the source edge-discovery order as the stable Rust traversal;
                // Go's runtime map order can still vary between processes and
                // is retained verbatim by the diagnostic trace comparator.
                side_order.push(side);
            }
            sides.entry(side).or_default().push(edge_index);
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_PORTS") {
            eprintln!(
                "SWAP_GROUPS_RUST node={} sides={:?}",
                graph.nodes[node.0 as usize].tala_id,
                sides
                    .iter()
                    .map(|(side, edges)| (
                        side,
                        edges
                            .iter()
                            .map(|edge| graph.edge_records[*edge].tala_id)
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>(),
            );
        }

        for side in &side_order {
            let edges = sides
                .get_mut(side)
                .expect("every discovered port side has a group");
            if trace_jsonl {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "swap-port-side-order",
                        "phase": "before-sort",
                        "node": node_id,
                        "side": format!("{side:?}"),
                        "edges": edges.iter().map(|edge| swap_trace_edge_id(graph, *edge)).collect::<Vec<_>>(),
                    })
                );
            }
            edges.sort_by(|first, second| {
                let (p1, _, p3) = endpoint_indices(graph, *first, node);
                let (q1, _, q3) = endpoint_indices(graph, *second, node);
                let first_points = &graph.edge_records[*first].route.read();
                let second_points = &graph.edge_records[*second].route.read();
                if matches!(side, PortSide::Top | PortSide::Bottom) {
                    source_stable_axis_order(
                        first_points[p1].x,
                        second_points[q1].x,
                        first_points[p3].x,
                        second_points[q3].x,
                    )
                } else {
                    source_stable_axis_order(
                        first_points[p1].y,
                        second_points[q1].y,
                        first_points[p3].y,
                        second_points[q3].y,
                    )
                }
            });
            if trace_jsonl {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "swap-port-side-order",
                        "phase": "after-sort",
                        "node": node_id,
                        "side": format!("{side:?}"),
                        "edges": edges.iter().map(|edge| swap_trace_edge_id(graph, *edge)).collect::<Vec<_>>(),
                    })
                );
            }
            for index in 0..edges.len().saturating_sub(1) {
                let first = edges[index];
                let second = edges[index + 1];
                let intersects = endpoint_segments_intersect(graph, first, second, node, *side);
                let can_swap =
                    intersects && can_swap_ports(graph, first, second, node, &port_counts);
                let before = trace_jsonl.then(|| {
                    (
                        graph.edge_records[first].route.read().clone(),
                        graph.edge_records[second].route.read().clone(),
                    )
                });
                if trace_jsonl {
                    let (first_before, second_before) = before.as_ref().unwrap();
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "event": "swap-port-same-side-candidate",
                            "node": node_id,
                            "side": format!("{side:?}"),
                            "a": swap_trace_edge_id(graph, first),
                            "b": swap_trace_edge_id(graph, second),
                            "intersects": intersects,
                            "can_swap": can_swap,
                            "a_before": swap_trace_route(first_before),
                            "b_before": swap_trace_route(second_before),
                        })
                    );
                }
                if !intersects || !can_swap {
                    continue;
                }
                swap_endpoint_coordinate(graph, first, second, node, *side);
                let clear = endpoint_segments_clear(graph, node, first, second, budget)?;
                if clear {
                    if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_PORTS") {
                        eprintln!(
                            "SWAP_SAME_RUST node={} first={}>{} second={}>{} side={:?}",
                            graph.nodes[node.0 as usize].tala_id,
                            graph.nodes[graph.edge_records[first].from.0 as usize].tala_id,
                            graph.nodes[graph.edge_records[first].to.0 as usize].tala_id,
                            graph.nodes[graph.edge_records[second].from.0 as usize].tala_id,
                            graph.nodes[graph.edge_records[second].to.0 as usize].tala_id,
                            side,
                        );
                    }
                    edges.swap(index, index + 1);
                    node_swapped = true;
                } else {
                    swap_endpoint_coordinate(graph, first, second, node, *side);
                }
                if trace_jsonl {
                    let (first_before, second_before) = before.as_ref().unwrap();
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "event": "swap-port-same-side-result",
                            "node": node_id,
                            "side": format!("{side:?}"),
                            "a": swap_trace_edge_id(graph, first),
                            "b": swap_trace_edge_id(graph, second),
                            "clear": clear,
                            "accepted": clear,
                            "a_before": swap_trace_route(first_before),
                            "b_before": swap_trace_route(second_before),
                            "a_after": swap_trace_route(&graph.edge_records[first].route.read()),
                            "b_after": swap_trace_route(&graph.edge_records[second].route.read()),
                        })
                    );
                }
            }
        }
        node_swapped |=
            swap_adjacent_side_ports(graph, node, &sides, &port_counts, trace_jsonl, budget)?;
        if trace_jsonl {
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "swap-port-node",
                    "phase": "end",
                    "node": node_id,
                    "swapped": node_swapped,
                })
            );
        }
        swapped |= node_swapped;
    }
    Ok(swapped)
}

fn reserve_swap_stage_steps(
    budget: &super::route_search_guard::RouteStageWork,
    steps: usize,
) -> Result<(), ()> {
    for _ in 0..steps {
        super::route_search_guard::reserve_checked(budget, 1)?;
    }
    Ok(())
}

fn reserve_swap_work(
    budget: Option<&super::route_search_guard::RouteStageWork>,
    units: u64,
) -> Result<(), ()> {
    if let Some(budget) = budget {
        super::route_search_guard::reserve_checked(budget, units)
    } else {
        Ok(())
    }
}

fn reserve_swap_stage_optional_steps(
    budget: Option<&super::route_search_guard::RouteStageWork>,
    steps: usize,
) -> Result<(), ()> {
    for _ in 0..steps {
        reserve_swap_work(budget, 1)?;
    }
    Ok(())
}

fn reserve_swap_validation_edge(
    graph: &ArenaGraph,
    edge_id: crate::EdgeId,
    seen_edges: &mut [bool],
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), ()> {
    let seen = &mut seen_edges[edge_id.0 as usize];
    if *seen {
        return Ok(());
    }
    *seen = true;
    // validateRouteStageGeometry validates both endpoints and every live
    // route point once for each distinct edge, after charging each reference.
    reserve_swap_stage_steps(budget, 2)?;
    reserve_swap_stage_steps(budget, graph.edge_records[edge_id.0 as usize].route.len())
}

fn reserve_swap_stage_validation_work(
    graph: &ArenaGraph,
    graph_nodes: &[NodeId],
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), ()> {
    reserve_swap_stage_steps(budget, graph_nodes.len())?;
    let mut seen_edges = vec![false; graph.edge_records.len()];
    for edge_id in graph.edge_order.read().iter().copied() {
        super::route_search_guard::reserve_checked(budget, 1)?;
        reserve_swap_validation_edge(graph, edge_id, &mut seen_edges, budget)?;
    }
    for node_id in graph_nodes {
        for edge_id in &graph.nodes[node_id.0 as usize].edges.read() {
            super::route_search_guard::reserve_checked(budget, 1)?;
            reserve_swap_validation_edge(graph, *edge_id, &mut seen_edges, budget)?;
        }
    }
    Ok(())
}

fn reserve_swap_capture_node(
    node_id: NodeId,
    seen_nodes: &mut [bool],
    queued_nodes: &mut Vec<NodeId>,
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), ()> {
    super::route_search_guard::reserve_checked(budget, 1)?;
    let seen = &mut seen_nodes[node_id.0 as usize];
    if !*seen {
        *seen = true;
        queued_nodes.push(node_id);
    }
    Ok(())
}

fn reserve_swap_capture_edge(
    graph: &ArenaGraph,
    edge_id: crate::EdgeId,
    seen_edges: &mut [bool],
    seen_nodes: &mut [bool],
    queued_nodes: &mut Vec<NodeId>,
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), ()> {
    // captureRouteMutations charges every edge reference before de-duplicating
    // it, then scans the entire Go slice backing array for a new edge.
    super::route_search_guard::reserve_checked(budget, 1)?;
    let seen = &mut seen_edges[edge_id.0 as usize];
    if *seen {
        return Ok(());
    }
    *seen = true;
    let edge = &graph.edge_records[edge_id.0 as usize];
    reserve_swap_stage_steps(budget, edge.route.capacity())?;
    reserve_swap_capture_node(edge.from, seen_nodes, queued_nodes, budget)?;
    reserve_swap_capture_node(edge.to, seen_nodes, queued_nodes, budget)
}

fn reserve_swap_stage_snapshot_work(
    graph: &ArenaGraph,
    graph_nodes: &[NodeId],
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), ()> {
    let mut seen_edges = vec![false; graph.edge_records.len()];
    let mut seen_nodes = vec![false; graph.nodes.len()];
    let mut queued_nodes = Vec::new();
    for edge_id in graph.edge_order.read().iter().copied() {
        reserve_swap_capture_edge(
            graph,
            edge_id,
            &mut seen_edges,
            &mut seen_nodes,
            &mut queued_nodes,
            budget,
        )?;
    }
    for node_id in graph_nodes {
        reserve_swap_capture_node(*node_id, &mut seen_nodes, &mut queued_nodes, budget)?;
        for edge_id in &graph.nodes[node_id.0 as usize].edges.read() {
            reserve_swap_capture_edge(
                graph,
                *edge_id,
                &mut seen_edges,
                &mut seen_nodes,
                &mut queued_nodes,
                budget,
            )?;
        }
    }
    // Match captureRouteMutations' descendant discovery for ordinary
    // containers, cluster vessels, and sequence vessels. Every child
    // reference is charged even when another path already captured it.
    let mut next = 0;
    while next < queued_nodes.len() {
        let node_id = queued_nodes[next];
        next += 1;
        let node = &graph.nodes[node_id.0 as usize];
        if node.is_container
            && let Some(children) = graph.containers.get(&Some(node_id))
        {
            for child in children {
                reserve_swap_capture_node(child, &mut seen_nodes, &mut queued_nodes, budget)?;
            }
        }
        if let Some(cluster) = graph
            .clusters
            .iter()
            .find(|cluster| cluster.vessel_tala_id == node.tala_id)
        {
            for child in &cluster.members {
                reserve_swap_capture_node(*child, &mut seen_nodes, &mut queued_nodes, budget)?;
            }
        }
        if let Some(sequence) = graph
            .sequences
            .iter()
            .find(|sequence| sequence.vessel_tala_id == node.tala_id)
        {
            for child in &sequence.members {
                reserve_swap_capture_node(*child, &mut seen_nodes, &mut queued_nodes, budget)?;
            }
        }
    }
    Ok(())
}

fn checked_sort_cost(length: usize) -> u64 {
    if length < 2 {
        return 1;
    }
    let levels = (usize::BITS - (length - 1).leading_zeros()) as u64;
    // stableSortRouteValues charges one step for every copied output value,
    // then one aggregate len(values) charge, on every merge pass.
    (length as u64).saturating_mul(levels).saturating_mul(2)
}

/// Checked owner boundary with source work charges and shared-storage rollback.
pub(in crate::internal) fn swap_edge_ports_with_budget(
    graph: &mut ArenaGraph,
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), super::route_stage_guard::RouteStageError> {
    super::swap_owner::swap_edge_ports_with_budget(graph, budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_side_route_order_matches_go_less_and_equal_for_signed_zero() {
        assert_eq!(
            source_stable_axis_order(-0.0, 0.0, 2.0, 1.0),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            source_stable_axis_order(0.0, -0.0, 1.0, 2.0),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            source_stable_axis_order(f64::NAN, 0.0, 1.0, 2.0),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn port_counts_match_go_point_map_equality_for_signed_zero() {
        let port_counts = [(Point { x: -0.0, y: 1.0 }, 2)];

        assert_eq!(port_count(&port_counts, Point { x: 0.0, y: 1.0 }), 2);
        assert_eq!(port_count(&port_counts, Point { x: -0.0, y: 1.0 }), 2);
    }

    #[test]
    fn intersection_rounds_the_delta_like_d2_geo() {
        let intersection = intersection_point(
            Point {
                x: 4554.0,
                y: 1122.5,
            },
            Point {
                x: 4392.5,
                y: 1122.5,
            },
            Point {
                x: 4392.5,
                y: 776.0,
            },
            Point {
                x: 4392.5,
                y: 1182.0,
            },
        )
        .unwrap();

        // D2 computes u0 + round(s * (u1-u0)), rather than rounding the
        // resulting coordinate or constructing the exact orthogonal point.
        assert_eq!(
            intersection,
            Point {
                x: 4392.0,
                y: 1122.5,
            }
        );
    }
}
