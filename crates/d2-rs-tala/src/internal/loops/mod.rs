// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dedicated route generation for self-loop edges.
//!
//! Loop index and shape ports select progressively offset orthogonal paths
//! around a node that is both source and target.

use super::*;
use crate::internal::ArenaEdge;
use crate::internal::routing::{Port, PortSide, ports_with_table_column_count};

/// The four loop slots are deliberately ordered like TALA's `makeLoopPorts`:
/// left/top, top/right, bottom/left, and right/bottom.  A slot is reused only
/// when its source and target arrowheads match; otherwise the next open slot
/// is selected.  This small bit of state is observable in multi-loop graphs.
#[derive(Clone, Debug)]
struct LoopSlot {
    source: Port,
    target: Port,
    source_side: PortSide,
    target_side: PortSide,
    vertical_offset: f64,
    horizontal_offset: f64,
    used: bool,
    source_enabled: bool,
    target_enabled: bool,
    source_arrowhead: Option<String>,
    target_arrowhead: Option<String>,
}

fn closest_ports(ports: &[Port], first: PortSide, second: PortSide) -> (Port, Port) {
    let mut best = None;
    for left in ports.iter().copied().filter(|port| port.side == first) {
        for right in ports.iter().copied().filter(|port| port.side == second) {
            let dx = left.point.x - right.point.x;
            let dy = left.point.y - right.point.y;
            let distance = (dx * dx + dy * dy).sqrt();
            if best
                .as_ref()
                .is_none_or(|(_, _, current)| distance < *current)
            {
                best = Some((left, right, distance));
            }
        }
    }
    best.map(|(left, right, _)| (left, right))
        .expect("every concrete D2 shape has ports on all four sides")
}

fn make_loop_slots(
    rect: Rect,
    shape: ShapeKind,
    table_column_count: Option<usize>,
) -> Vec<LoopSlot> {
    // TALA's Node.Ports delegates to the concrete table shape and therefore
    // retains its semantic row count.  Passing that count through here is
    // required even when a loop has no table-column edge; otherwise a table
    // with one row is routed using the zero-row center ports.
    let all = ports_with_table_column_count(rect, shape, table_column_count);
    let (lt_left, lt_top) = closest_ports(&all, PortSide::Left, PortSide::Top);
    let (tr_top, tr_right) = closest_ports(&all, PortSide::Top, PortSide::Right);
    let (bl_bottom, bl_left) = closest_ports(&all, PortSide::Bottom, PortSide::Left);
    let (br_right, br_bottom) = closest_ports(&all, PortSide::Right, PortSide::Bottom);
    [
        (lt_left, lt_top, PortSide::Left, PortSide::Top),
        (tr_top, tr_right, PortSide::Top, PortSide::Right),
        (bl_bottom, bl_left, PortSide::Bottom, PortSide::Left),
        (br_right, br_bottom, PortSide::Right, PortSide::Bottom),
    ]
    .into_iter()
    .map(|(source, target, source_side, target_side)| LoopSlot {
        source,
        target,
        source_side,
        target_side,
        vertical_offset: 0.0,
        horizontal_offset: 0.0,
        used: false,
        source_enabled: false,
        target_enabled: false,
        source_arrowhead: None,
        target_arrowhead: None,
    })
    .collect()
}

fn has_source_arrow(edge: &ArenaEdge) -> bool {
    edge.source_arrow
}

fn has_target_arrow(edge: &ArenaEdge) -> bool {
    edge.target_arrow
}

fn arrow_score(edge: &ArenaEdge) -> usize {
    if has_source_arrow(edge) != has_target_arrow(edge) {
        0
    } else if has_source_arrow(edge) {
        1
    } else {
        2
    }
}

/// Stable edge order used by the Go loop router: arrow direction first, then
/// label area, with the original incident-edge order as the final tie-break.
pub(crate) fn ordered_self_loop_edges(graph: &ArenaGraph, node: NodeId) -> Vec<usize> {
    let mut edges = graph.nodes[node.0 as usize]
        .edges
        .read()
        .iter()
        .map(|edge| edge.0 as usize)
        .filter(|&edge| {
            graph.edge_records[edge].from == node && graph.edge_records[edge].to == node
        })
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| {
        arrow_score(&graph.edge_records[*left])
            .cmp(&arrow_score(&graph.edge_records[*right]))
            .then_with(|| {
                let left_area = graph.edge_records[*left]
                    .label
                    .as_ref()
                    .map_or(0.0, |label| label.size.width * label.size.height);
                let right_area = graph.edge_records[*right]
                    .label
                    .as_ref()
                    .map_or(0.0, |label| label.size.width * label.size.height);
                left_area.total_cmp(&right_area)
            })
    });
    edges
}

fn edge_kind(edge: &ArenaEdge, source: bool) -> Option<String> {
    let enabled = if source {
        has_source_arrow(edge)
    } else {
        has_target_arrow(edge)
    };
    if !enabled {
        None
    } else if source {
        edge.source_arrowhead.clone()
    } else {
        edge.target_arrowhead.clone()
    }
}

fn choose_slot<'a>(slots: &'a mut [LoopSlot], edge: &ArenaEdge) -> &'a mut LoopSlot {
    let source_enabled = has_source_arrow(edge);
    let target_enabled = has_target_arrow(edge);
    let source_arrowhead = edge_kind(edge, true);
    let target_arrowhead = edge_kind(edge, false);
    let matching = slots.iter().position(|slot| {
        slot.used
            && slot.source_enabled == source_enabled
            && slot.target_enabled == target_enabled
            && slot.source_arrowhead == source_arrowhead
            && slot.target_arrowhead == target_arrowhead
    });
    if let Some(index) = matching {
        return &mut slots[index];
    }
    if let Some(index) = slots.iter().position(|slot| !slot.used) {
        return &mut slots[index];
    }
    let index = slots
        .iter()
        .position(|slot| {
            slot.used
                && slot.source_enabled == source_enabled
                && slot.target_enabled == target_enabled
        })
        .unwrap_or(0);
    &mut slots[index]
}

fn bend_point(port: Point, side: PortSide, vertical_offset: f64, horizontal_offset: f64) -> Point {
    match side {
        PortSide::Top => Point {
            x: port.x,
            y: port.y - vertical_offset - 30.0,
        },
        PortSide::Bottom => Point {
            x: port.x,
            y: port.y + vertical_offset + 30.0,
        },
        PortSide::Left => Point {
            x: port.x - horizontal_offset - 30.0,
            y: port.y,
        },
        PortSide::Right => Point {
            x: port.x + horizontal_offset + 30.0,
            y: port.y,
        },
    }
}

fn route_slot(slot: &LoopSlot) -> Vec<Point> {
    let p1 = bend_point(
        slot.source.point,
        slot.source_side,
        slot.vertical_offset,
        slot.horizontal_offset,
    );
    let p2 = bend_point(
        slot.target.point,
        slot.target_side,
        slot.vertical_offset,
        slot.horizontal_offset,
    );
    let intersection = if matches!(slot.source_side, PortSide::Top | PortSide::Bottom) {
        Point { x: p2.x, y: p1.y }
    } else {
        Point { x: p1.x, y: p2.y }
    };
    vec![slot.source.point, p1, intersection, p2, slot.target.point]
}

fn compute_edge_offset(edge: &ArenaEdge, points: &[Point]) -> (f64, f64) {
    let (mut vertical, mut horizontal) = if (points[0].x - points[1].x).abs() < f64::EPSILON {
        (
            (points[0].y - points[1].y).abs(),
            (points[points.len() - 1].x - points[points.len() - 2].x).abs(),
        )
    } else {
        (
            (points[points.len() - 1].y - points[points.len() - 2].y).abs(),
            (points[0].x - points[1].x).abs(),
        )
    };
    if let Some(label) = &edge.label {
        vertical += label.size.height;
        horizontal += label.size.width;
    }
    (vertical, horizontal)
}

/// Route all self edges exactly as TALA's `routeLoops`, returning the edge
/// index, route, and occupied endpoint ports.  Keeping this shared between
/// preprocessing and final routing ensures loop extents and published routes
/// cannot drift apart.
pub(crate) fn self_loop_routes(
    graph: &ArenaGraph,
    node: NodeId,
    rect: Rect,
) -> Vec<(usize, Vec<Point>, Vec<Port>)> {
    let mut slots = make_loop_slots(
        rect,
        graph.nodes[node.0 as usize].shape,
        graph.nodes[node.0 as usize].table_column_count,
    );
    let mut result = Vec::new();
    for edge_index in ordered_self_loop_edges(graph, node) {
        let edge = &graph.edge_records[edge_index];
        let slot = choose_slot(&mut slots, edge);
        let route = route_slot(slot);
        let (vertical, horizontal) = compute_edge_offset(edge, &route);
        slot.vertical_offset = vertical;
        slot.horizontal_offset = horizontal;
        slot.used = true;
        slot.source_enabled = has_source_arrow(edge);
        slot.target_enabled = has_target_arrow(edge);
        slot.source_arrowhead = edge_kind(edge, true);
        slot.target_arrowhead = edge_kind(edge, false);
        if crate::internal::trace_env_enabled("TALA_TRACE_LOOP_ROUTES") {
            eprintln!(
                "LOOP_ROUTE_RUST id={} edge={} points={:?} label={:?} arrows={:?},{:?}",
                graph.nodes[node.0 as usize].tala_id,
                edge_index,
                route,
                edge.label.as_ref().map(|label| &label.size),
                edge.source_arrowhead,
                edge.target_arrowhead,
            );
        }
        result.push((edge_index, route, vec![slot.source, slot.target]));
    }
    result
}

/// Backwards-compatible single-edge helper used by routing tests and callers
/// that already have the loop ordinal.  The ordinal follows TALA's slot order.
pub(super) fn self_loop_route(
    rect: Rect,
    shape: ShapeKind,
    loop_index: usize,
) -> (Vec<Point>, Vec<Port>) {
    let slots = make_loop_slots(rect, shape, None);
    let slot = &slots[loop_index % slots.len()];
    (route_slot(slot), vec![slot.source, slot.target])
}
