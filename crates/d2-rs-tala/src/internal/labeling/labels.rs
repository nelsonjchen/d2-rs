// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Edge-label and arrowhead-label scoring and publication.
//!
//! Candidate positions are evaluated against final route geometry, boxes,
//! sibling labels, preferred order, and fixed D2 metadata.

use super::{ArenaEdge, ArenaGraph};
use crate::internal::RoutePoints;
use crate::{ExternalLabel, LabelPosition, NodeId, Point, Rect, ShapeKind, Size};
use std::cmp::Ordering;

mod guarded;
mod node;
pub(super) use node::default_node_label_position;
#[cfg(test)]
pub(super) use node::place_node_labels;
pub(super) use node::point_on_box;
#[cfg(test)]
pub(super) use node::positioned_node_label_obstacles;
pub(super) use node::{
    ancestor_rects, partial_overlap_count, positioned_icon_label_rect, positioned_node_label_rect,
    sibling_and_child_rects,
};

const EDGE_LABEL_PREFERENCE_ORDER: &[LabelPosition] = &[
    LabelPosition::OutsideTopCenter,
    LabelPosition::OutsideBottomCenter,
    LabelPosition::OutsideTopLeft,
    LabelPosition::OutsideTopRight,
    LabelPosition::OutsideBottomLeft,
    LabelPosition::OutsideBottomRight,
    LabelPosition::InsideMiddleCenter,
    LabelPosition::InsideMiddleLeft,
    LabelPosition::InsideMiddleRight,
];

macro_rules! trace_reorder_duplicates {
    ($event:expr) => {
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("D2RS_TRACE_REORDER_DUPLICATES_JSONL") {
            eprintln!("{}", $event);
        }
    };
}

#[cfg(feature = "diagnostic-traces")]
fn trace_reorder_duplicate_route(points: &[Point]) -> Vec<serde_json::Value> {
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

fn is_unlocked(position: LabelPosition) -> bool {
    matches!(
        position,
        LabelPosition::UnlockedTop | LabelPosition::UnlockedMiddle | LabelPosition::UnlockedBottom
    )
}

fn is_on_edge(position: LabelPosition) -> bool {
    matches!(
        position,
        LabelPosition::InsideMiddleLeft
            | LabelPosition::InsideMiddleCenter
            | LabelPosition::InsideMiddleRight
            | LabelPosition::UnlockedMiddle
    )
}

/// Pinned Go 1.27.0 arm64 emits fused multiply-add operations in the label
/// geometry helpers. Other targets retain the source's separate operations;
/// their compiler/CPU configurations require independent bit qualification.
fn label_mul_add(left: f64, right: f64, addend: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        left.mul_add(right, addend)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        left * right + addend
    }
}

/// `geo.EuclideanDistance` takes exact axis branches before squaring. This
/// preserves finite huge lengths and subnormal segments used by labels.
fn label_segment_length(start: Point, end: Point) -> f64 {
    if start.x == end.x {
        (start.y - end.y).abs()
    } else if start.y == end.y {
        (start.x - end.x).abs()
    } else {
        let dx = start.x - end.x;
        let dy = start.y - end.y;
        // The second square is fused into the rounded first square in the
        // pinned ARM64 EuclideanDistance body before FSQRT.
        label_mul_add(dy, dy, dx * dx).sqrt()
    }
}

/// `geo.Point.Interpolate` uses weighted endpoints, with `end * ratio`
/// rounded before the pinned ARM64 fused addition of `start * (1-ratio)`.
fn interpolate_label_point(start: Point, end: Point, ratio: f64) -> Point {
    let inverse = 1.0 - ratio;
    Point {
        x: label_mul_add(start.x, inverse, end.x * ratio),
        y: label_mul_add(start.y, inverse, end.y * ratio),
    }
}

fn route_length(points: &[Point]) -> f64 {
    points
        .windows(2)
        .map(|segment| label_segment_length(segment[0], segment[1]))
        .sum()
}

fn point_at_distance(points: &[Point], distance: f64) -> Option<(Point, usize)> {
    if points.len() < 2 {
        return None;
    }
    let mut remaining = distance;
    let mut last_segment = None;
    for (index, segment) in points.windows(2).enumerate() {
        let length = label_segment_length(segment[0], segment[1]);
        // Go's Route.GetPointAtDistance retains zero-length segments. A
        // selected coincident segment therefore produces a present NaN point
        // through 0/0, including a zero-distance first segment.
        last_segment = Some((index, length));
        if remaining <= length {
            let ratio = remaining / length;
            return Some((
                interpolate_label_point(segment[0], segment[1], ratio),
                index,
            ));
        }
        remaining -= length;
    }
    let (index, length) = last_segment?;
    let segment = &points[index..=index + 1];
    let ratio = 1.0 + remaining / length;
    Some((
        interpolate_label_point(segment[0], segment[1], ratio),
        index,
    ))
}

fn chop_precision(value: f64) -> f64 {
    let rounded = ((value * 10_000.0) as f32 as f64 / 10_000.0).round();
    if rounded == 0.0 { 0.0 } else { rounded }
}

pub(super) fn route_label_top_left(
    points: &[Point],
    position: LabelPosition,
    percentage: f64,
    width: f64,
    height: f64,
    stroke_width: f64,
) -> Option<Point> {
    let length = route_length(points);
    let distance = match position {
        LabelPosition::InsideMiddleLeft
        | LabelPosition::OutsideTopLeft
        | LabelPosition::OutsideBottomLeft => 0.25 * length,
        LabelPosition::InsideMiddleCenter
        | LabelPosition::OutsideTopCenter
        | LabelPosition::OutsideBottomCenter => 0.5 * length,
        LabelPosition::InsideMiddleRight
        | LabelPosition::OutsideTopRight
        | LabelPosition::OutsideBottomRight => 0.75 * length,
        LabelPosition::UnlockedTop
        | LabelPosition::UnlockedMiddle
        | LabelPosition::UnlockedBottom => percentage * length,
        _ => return None,
    };
    let (mut center, segment_index) = point_at_distance(points, distance)?;
    let top = matches!(
        position,
        LabelPosition::OutsideTopLeft
            | LabelPosition::OutsideTopCenter
            | LabelPosition::OutsideTopRight
            | LabelPosition::UnlockedTop
    );
    let bottom = matches!(
        position,
        LabelPosition::OutsideBottomLeft
            | LabelPosition::OutsideBottomCenter
            | LabelPosition::OutsideBottomRight
            | LabelPosition::UnlockedBottom
    );
    if top || bottom {
        let start = points[segment_index];
        let end = points[segment_index + 1];
        let segment_length = label_segment_length(start, end);
        #[cfg(not(target_arch = "x86_64"))]
        let (normal_x, normal_y) = {
            let direction = if top { -1.0 } else { 1.0 };
            (
                (start.y - end.y) / segment_length * direction,
                (end.x - start.x) / segment_length * direction,
            )
        };
        #[cfg(target_arch = "x86_64")]
        let (normal_x, normal_y) = {
            // Go amd64 flips the sign bit for the top normal, including NaNs.
            let x = (start.y - end.y) / segment_length;
            let y = (end.x - start.x) / segment_length;
            if top { (-x, -y) } else { (x, y) }
        };
        // label.Position.GetPointOnRoute forms both offsets and their final
        // projections with fused additions in the pinned ARM64 body.
        let padding = label_mul_add(stroke_width, 0.5, 5.0);
        let offset_x = label_mul_add(width, 0.5, padding);
        let offset_y = label_mul_add(height, 0.5, padding);
        center.x = label_mul_add(normal_x, offset_x, center.x);
        center.y = label_mul_add(normal_y, offset_y, center.y);
    }
    Some(Point {
        x: chop_precision(label_mul_add(-width, 0.5, center.x)),
        y: chop_precision(label_mul_add(-height, 0.5, center.y)),
    })
}

/// Translation of D2 label `Position.GetPointOnRoute`, called by recovered
/// TALA `Edge.GetLabelTopLeft` with a fixed three-pixel stroke width.
pub(super) fn edge_label_top_left(
    edge: &ArenaEdge,
    position: LabelPosition,
    percentage: f64,
    width: f64,
    height: f64,
) -> Option<Point> {
    route_label_top_left(&edge.route.read(), position, percentage, width, height, 3.0)
}

pub(super) fn label_rect(
    edge: &ArenaEdge,
    position: LabelPosition,
    percentage: f64,
) -> Option<Rect> {
    let label = edge.label.as_ref()?;
    Some(Rect {
        origin: edge_label_top_left(
            edge,
            position,
            percentage,
            label.size.width,
            label.size.height,
        )?,
        size: label.size,
    })
}

fn arrowhead_height(enabled: bool, explicit: &Option<String>) -> f64 {
    if !enabled {
        return 0.0;
    }
    // d2target.Arrowhead.Dimensions at BaseConnection's two-pixel stroke.
    // An enabled arrow without an explicit shape is D2's triangle default.
    match explicit.as_deref().unwrap_or("triangle") {
        "none" => 0.0,
        "arrow" | "triangle" => 12.0,
        "unfilled-triangle" => 15.0,
        "line" => 16.0,
        "filled-diamond" => 14.0,
        "diamond" => 18.0,
        "cross" => 17.0,
        "filled-circle" | "circle" => 18.0,
        "filled-box" | "box" => 16.0,
        "cf-one" | "cf-many" | "cf-one-required" | "cf-many-required" => 18.0,
        _ => 0.0,
    }
}

/// Recovered `Edge.GetPositionedArrowheadLabel`, including D2's deliberate
/// integer truncation of label dimensions while calculating the position.
pub(super) fn arrowhead_label_rect_for_route(
    edge: &ArenaEdge,
    route: &[Point],
    is_target: bool,
) -> Option<Rect> {
    let label = if is_target {
        edge.target_arrowhead_label.as_ref()?
    } else {
        edge.source_arrowhead_label.as_ref()?
    };
    if route.len() < 2 {
        return None;
    }

    let width = label.size.width as i64 as f64;
    let height = label.size.height as i64 as f64;
    let segment_index = if is_target { route.len() - 2 } else { 0 };
    let start = route[segment_index];
    let end = route[segment_index + 1];
    let segment_length = label_segment_length(end, start);
    // GetUnitNormalVector(end, start): the unlocked-top normal points away
    // from the endpoint's arrowhead segment. A coincident segment has a NaN
    // normal in Go; retain its label box so guarded collision scans still run.
    let normal_x = (end.y - start.y) / segment_length;
    let normal_y = (start.x - end.x) / segment_length;
    let shift_x = label_mul_add(height, 0.5, 5.0);
    let shift_y = label_mul_add(width, 0.5, 5.0);
    let shift = label_mul_add(normal_y.abs(), shift_y, normal_x.abs() * shift_x);
    let length = route_length(route);
    let percentage = if is_target {
        if length > 0.0 {
            1.0 - shift / length
        } else {
            1.0
        }
    } else if length > 0.0 {
        shift / length
    } else {
        0.0
    };
    let mut origin = route_label_top_left(
        route,
        LabelPosition::UnlockedTop,
        percentage,
        width,
        height,
        2.0,
    )?;

    // d2target checks the destination arrow first for a target label, then
    // deliberately falls back to the source arrowhead.
    // Raw explicit strings participate in the source fallback test. An empty
    // target string has zero dimensions but still differs from "none".
    let target_has_arrow = edge
        .target_arrowhead
        .as_deref()
        .map_or(edge.target_arrow, |name| name != "none");
    let arrow_height = if is_target && target_has_arrow {
        arrowhead_height(edge.target_arrow, &edge.target_arrowhead)
    } else {
        arrowhead_height(edge.source_arrow, &edge.source_arrowhead)
    };
    let offset = (arrow_height / 2.0 + 2.0) - 1.0 - 5.0;
    if offset > 0.0 {
        origin.x = label_mul_add(normal_x, offset, origin.x);
        origin.y = label_mul_add(normal_y, offset, origin.y);
    }
    Some(Rect {
        origin,
        // PositionedArrowheadLabel keeps the original floating dimensions.
        size: label.size,
    })
}

#[derive(Clone, Debug)]
pub(super) struct PositionedArrowheadLabel {
    pub(super) rect: Rect,
    pub(super) edge_index: usize,
    pub(super) is_target: bool,
    pub(super) text: String,
}

pub(super) fn positioned_arrowhead_label_for_route(
    edge_index: usize,
    edge: &ArenaEdge,
    route: &[Point],
    is_target: bool,
) -> Option<PositionedArrowheadLabel> {
    let label = if is_target {
        edge.target_arrowhead_label.as_ref()?
    } else {
        edge.source_arrowhead_label.as_ref()?
    };
    Some(PositionedArrowheadLabel {
        rect: arrowhead_label_rect_for_route(edge, route, is_target)?,
        edge_index,
        is_target,
        text: label.text.clone(),
    })
}

/// Direct translation of `PositionedArrowheadLabel.Cost`, with the caller's
/// route/edge scan represented by its one-per-route overlap count.
pub(super) fn positioned_arrowhead_label_cost(
    graph: &ArenaGraph,
    candidate: &PositionedArrowheadLabel,
    positioned_labels: &[PositionedArrowheadLabel],
    overlapping_edge_count: usize,
) -> f64 {
    for other in positioned_labels {
        if candidate.edge_index == other.edge_index && candidate.is_target == other.is_target {
            continue;
        }
        if overlaps(candidate.rect, other.rect, 0.0) && candidate.text != other.text {
            return f64::INFINITY;
        }
    }

    let edge = &graph.edge_records[candidate.edge_index];
    let node_overlap_count = graph
        .node_order
        .iter()
        .map(|id| (id.0 as usize, &graph.nodes[id.0 as usize]))
        .filter(|(node_index, node)| {
            let is_non_endpoint_ancestor = node.input_id != edge.from
                && node.input_id != edge.to
                && (is_ancestor_of(graph, *node_index, edge.from.0 as usize)
                    || is_ancestor_of(graph, *node_index, edge.to.0 as usize));
            if is_non_endpoint_ancestor {
                return false;
            }
            node.position.is_some_and(|position| {
                overlaps(
                    candidate.rect,
                    Rect {
                        origin: position,
                        size: node.rect.size,
                    },
                    5.0,
                )
            })
        })
        .count();

    graph.turn_cost_value() * 4.0 * node_overlap_count as f64
        + graph.turn_cost_value() * overlapping_edge_count as f64
}

pub(super) fn positioned_arrowhead_label_overlaps_route(
    candidate: &PositionedArrowheadLabel,
    route: &[Point],
) -> bool {
    route
        .windows(2)
        .any(|segment| rect_overlaps_segment(candidate.rect, segment[0], segment[1], 0.0))
}

fn arrowhead_label_rect(edge: &ArenaEdge, is_target: bool) -> Option<Rect> {
    arrowhead_label_rect_for_route(edge, &edge.route.read(), is_target)
}

fn initial_label_obstacles(graph: &ArenaGraph, include_fixed: bool) -> Vec<Rect> {
    let mut obstacles = Vec::new();
    for edge_id in &graph.edge_order.read() {
        let edge = &graph.edge_records[edge_id.0 as usize];
        if let Some(rect) = arrowhead_label_rect(edge, false) {
            obstacles.push(rect);
        }
        if let Some(rect) = arrowhead_label_rect(edge, true) {
            obstacles.push(rect);
        }
    }
    for edge_id in &graph.edge_order.read() {
        let edge = &graph.edge_records[edge_id.0 as usize];
        if (edge.from == edge.to || include_fixed && edge.label_position_fixed)
            && let Some(label) = &edge.label
            && let Some(rect) = label_rect(edge, label.position, label.percentage)
        {
            obstacles.push(rect);
        }
    }
    obstacles
}

fn external_node_label_rect(node_rect: Rect, label: ExternalLabel) -> Rect {
    const PADDING: f64 = 5.0;
    Rect {
        origin: label.top_left(node_rect, PADDING),
        size: label.size,
    }
}

pub(super) fn overlaps(left: Rect, right: Rect, delta: f64) -> bool {
    left.origin.x < right.origin.x + right.size.width + delta
        && right.origin.x < left.origin.x + left.size.width + delta
        && left.origin.y < right.origin.y + right.size.height + delta
        && right.origin.y < left.origin.y + left.size.height + delta
}

fn covers(outer: Rect, inner: Rect) -> bool {
    inner.origin.x >= outer.origin.x
        && inner.origin.y >= outer.origin.y
        && inner.origin.x + inner.size.width <= outer.origin.x + outer.size.width
        && inner.origin.y + inner.size.height <= outer.origin.y + outer.size.height
}

fn overlap_area(left: Rect, right: Rect) -> f64 {
    let width = (left.origin.x + left.size.width).min(right.origin.x + right.size.width)
        - left.origin.x.max(right.origin.x);
    let height = (left.origin.y + left.size.height).min(right.origin.y + right.size.height)
        - left.origin.y.max(right.origin.y);
    width.max(0.0) * height.max(0.0)
}

fn contains(rect: Rect, point: Point, delta: f64) -> bool {
    rect.origin.x - delta <= point.x
        && point.x <= rect.origin.x + rect.size.width + delta
        && rect.origin.y - delta <= point.y
        && point.y <= rect.origin.y + rect.size.height + delta
}

pub(super) fn orientation(p: Point, q: Point, r: Point) -> f64 {
    let pq_x = q.x - p.x;
    let pq_y = q.y - p.y;
    let pr_x = r.x - p.x;
    let pr_y = r.y - p.y;

    // The pinned Go 1.27.0 macOS arm64 compiler emits an FMSUB for the second
    // product in layoutgraph.orientation. Preserve that fused rounding on
    // arm64. Go's amd64 GOAMD64=v1 build uses separate MULSD/SUBSD operations;
    // other release-target configurations still need qualification.
    #[cfg(target_arch = "aarch64")]
    {
        (-pq_x).mul_add(pr_y, pq_y * pr_x)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        pq_y * pr_x - pq_x * pr_y
    }
}

fn straddles_line(left: f64, right: f64) -> bool {
    left == 0.0 || right == 0.0 || (left < 0.0) != (right < 0.0)
}

fn closed_intervals_overlap(a1: f64, a2: f64, b1: f64, b2: f64) -> bool {
    use crate::internal::routing::channel_solver::{maximum, minimum};
    let a_min = minimum(a1, a2);
    let a_max = maximum(a1, a2);
    let b_min = minimum(b1, b2);
    let b_max = maximum(b1, b2);
    a_min <= b_max && b_min <= a_max
}

fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    // layoutgraph.intersects evaluates all four sides before distinguishing
    // collinear intervals from straddling lines. Its comparison semantics also
    // apply when finite coordinates overflow or underflow during orientation.
    let abc = orientation(a, b, c);
    let abd = orientation(a, b, d);
    let cda = orientation(c, d, a);
    let cdb = orientation(c, d, b);
    if abc == 0.0 && abd == 0.0 && cda == 0.0 && cdb == 0.0 {
        return closed_intervals_overlap(a.x, b.x, c.x, d.x)
            && closed_intervals_overlap(a.y, b.y, c.y, d.y);
    }
    straddles_line(abc, abd) && straddles_line(cda, cdb)
}

fn rect_overlaps_segment(rect: Rect, start: Point, end: Point, delta: f64) -> bool {
    if contains(rect, start, delta) || contains(rect, end, delta) {
        return true;
    }
    let left = rect.origin.x - delta;
    let right = rect.origin.x + rect.size.width + delta;
    let top = rect.origin.y - delta;
    let bottom = rect.origin.y + rect.size.height + delta;
    let top_left = Point { x: left, y: top };
    let top_right = Point { x: right, y: top };
    let bottom_left = Point { x: left, y: bottom };
    let bottom_right = Point {
        x: right,
        y: bottom,
    };
    segments_intersect(top_left, top_right, start, end)
        || segments_intersect(top_right, bottom_right, start, end)
        || segments_intersect(bottom_right, bottom_left, start, end)
        || segments_intersect(bottom_left, top_left, start, end)
}

fn edge_overlap_count(
    rect: Rect,
    edges: impl Iterator<Item = usize>,
    graph: &ArenaGraph,
    delta: f64,
) -> usize {
    edges
        .map(|edge_index| {
            graph.edge_records[edge_index]
                .route
                .read()
                .windows(2)
                .filter(|segment| rect_overlaps_segment(rect, segment[0], segment[1], delta))
                .count()
        })
        .sum()
}

fn overlap_count(rect: Rect, others: &[Rect], delta: f64) -> usize {
    others
        .iter()
        .filter(|other| overlaps(rect, **other, delta))
        .count()
}

fn edge_label_overlap_score(
    label_area: f64,
    node_overlap_area: f64,
    exact_label_overlaps: usize,
    edge_overlaps: usize,
    almost_label_overlaps: usize,
    node_overlaps: usize,
    shared_segment_overlaps: usize,
) -> f64 {
    // This argument order is the recovered ScoreEdgeLabelOverlaps call
    // contract, including its counterintuitive 10x exact-label weight.
    2.0 * (node_overlap_area / label_area)
        + 10.0 * exact_label_overlaps as f64
        + almost_label_overlaps as f64
        + 2.0 * node_overlaps as f64
        + 2.0 * edge_overlaps as f64
        + shared_segment_overlaps as f64
}

fn is_ancestor_of(graph: &ArenaGraph, ancestor_index: usize, node_index: usize) -> bool {
    let ancestor = graph.nodes[ancestor_index].input_id;
    let mut current = graph.nodes[node_index].container;
    while let Some(container) = current {
        if container == ancestor {
            return true;
        }
        current = graph.nodes[container.0 as usize].container;
    }
    false
}

fn node_overlap_score(rect: Rect, edge_index: usize, graph: &ArenaGraph) -> (f64, usize) {
    let edge = &graph.edge_records[edge_index];
    let nodes = graph.graph_node_order_cow();
    // placement.go788-805 partitions live Graph.Nodes; guard.go215-234
    // measures each list separately before adding the two accumulated areas.
    let measure = |ancestors: bool| {
        let mut area = 0.0;
        let mut count = 0;
        for &id in nodes.iter() {
            let node = &graph.nodes[id.0 as usize];
            let ancestor = id != edge.from
                && id != edge.to
                && (is_ancestor_of(graph, id.0 as usize, edge.from.0 as usize)
                    || is_ancestor_of(graph, id.0 as usize, edge.to.0 as usize));
            if ancestor != ancestors {
                continue;
            }
            let Some(position) = node.position else {
                continue;
            };
            let node_rect = Rect {
                origin: position,
                size: node.rect.size,
            };
            if !overlaps(rect, node_rect, 5.0) || (ancestors && covers(node_rect, rect)) {
                continue;
            }
            if overlaps(rect, node_rect, 0.0) {
                area += overlap_area(rect, node_rect);
                count += 1;
            }
            if !node.is_container {
                area += rect.size.width * rect.size.height;
                count += 1;
            }
        }
        (area, count)
    };
    let (ancestor_area, ancestor_count) = measure(true);
    let (other_area, other_count) = measure(false);
    (ancestor_area + other_area, ancestor_count + other_count)
}

#[derive(Clone, Copy, Debug)]
struct AxisSegment {
    fixed: f64,
    start: f64,
    end: f64,
}

fn merged_shared_ranges(mut segments: Vec<AxisSegment>) -> Vec<AxisSegment> {
    segments.sort_by(|left, right| {
        left.fixed
            .total_cmp(&right.fixed)
            .then_with(|| left.start.total_cmp(&right.start))
            .then_with(|| left.end.total_cmp(&right.end))
    });
    let mut shared = Vec::new();
    let mut group_start = 0;
    while group_start < segments.len() {
        let fixed = segments[group_start].fixed;
        let mut group_end = group_start + 1;
        while group_end < segments.len() && segments[group_end].fixed == fixed {
            group_end += 1;
        }
        if group_end - group_start > 1 {
            let mut previous = segments[group_start];
            let mut current_shared = None::<AxisSegment>;
            for current in &segments[group_start + 1..group_end] {
                if previous.end > current.start {
                    // placement.go1108-1113: a long trunk can have separated
                    // shared spans; keep the exclusive corridor between them.
                    if current_shared.is_some_and(|range| current.start > range.end) {
                        shared.push(current_shared.take().unwrap());
                    }
                    let overlap_end = previous.end.min(current.end);
                    match &mut current_shared {
                        Some(range) => range.end = range.end.max(overlap_end),
                        None => {
                            current_shared = Some(AxisSegment {
                                fixed,
                                start: current.start,
                                end: overlap_end,
                            });
                        }
                    }
                } else if let Some(range) = current_shared.take() {
                    shared.push(range);
                }
                if current.end > previous.end {
                    previous = *current;
                }
            }
            if let Some(range) = current_shared {
                shared.push(range);
            }
        }
        group_start = group_end;
    }
    shared
}

fn shared_segment_boxes(graph: &ArenaGraph) -> Vec<Rect> {
    let mut vertical = Vec::new();
    let mut horizontal = Vec::new();
    // findSharedSegmentsChecked receives the live Graph.Edges slice.
    for edge_id in &graph.edge_order.read() {
        let edge = &graph.edge_records[edge_id.0 as usize];
        for points in edge.route.read().windows(2) {
            if points[0].x == points[1].x && points[0].y == points[1].y {
                continue;
            }
            if points[0].x == points[1].x {
                vertical.push(AxisSegment {
                    fixed: points[0].x,
                    start: points[0].y.min(points[1].y),
                    end: points[0].y.max(points[1].y),
                });
            } else if points[0].y == points[1].y {
                horizontal.push(AxisSegment {
                    fixed: points[0].y,
                    start: points[0].x.min(points[1].x),
                    end: points[0].x.max(points[1].x),
                });
            }
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    if crate::internal::trace_env_enabled("TALA_TRACE_SHARED_SEGMENTS") {
        eprintln!(
            "SHARED_SEGMENTS_RUST_RAW vertical={:?} horizontal={:?}",
            vertical, horizontal
        );
    }
    // TALA's findSharedSegments groups collinear segments, then sweeps each
    // group into merged shared ranges. Pairwise intersections are not
    // equivalent: three coincident routes would otherwise count the same
    // physical obstacle multiple times during label scoring.
    let mut boxes = Vec::new();
    boxes.extend(
        merged_shared_ranges(vertical)
            .into_iter()
            .map(|segment| Rect {
                origin: Point {
                    x: segment.fixed - 12.5,
                    y: segment.start,
                },
                size: Size {
                    width: 25.0,
                    height: segment.end - segment.start,
                },
            }),
    );
    boxes.extend(
        merged_shared_ranges(horizontal)
            .into_iter()
            .map(|segment| Rect {
                origin: Point {
                    x: segment.start,
                    y: segment.fixed - 12.5,
                },
                size: Size {
                    width: segment.end - segment.start,
                    height: 25.0,
                },
            }),
    );
    #[cfg(feature = "diagnostic-traces")]
    if crate::internal::trace_env_enabled("TALA_TRACE_SHARED_SEGMENTS") {
        eprintln!(
            "SHARED_SEGMENTS_RUST count={} boxes={:?}",
            boxes.len(),
            boxes
        );
    }
    boxes
}

fn graph_node_rect_entries(graph: &ArenaGraph) -> Vec<(Rect, bool)> {
    graph
        .nodes
        .iter()
        .filter_map(|node| {
            Some((
                Rect {
                    origin: node.position?,
                    size: node.rect.size,
                },
                node.is_container,
            ))
        })
        .collect()
}

fn label_node_overlap_area(rect: Rect, nodes: &[(Rect, bool)], delta: f64, partial: bool) -> f64 {
    nodes
        .iter()
        .filter_map(|(other, is_container)| {
            if !overlaps(rect, *other, delta) || (partial && covers(*other, rect)) {
                return None;
            }
            let mut area = 0.0;
            if delta == 0.0 || overlaps(rect, *other, 0.0) {
                area += overlap_area(rect, *other);
            }
            if !*is_container {
                area += rect.size.width * rect.size.height;
            }
            Some(area)
        })
        .sum()
}

/// Recovered TALA `Graph.ScoreExistingLabelPlacements` scalar.
#[cfg(test)]
pub(super) fn score_existing_label_placements(graph: &ArenaGraph) -> f64 {
    score_existing_label_placements_checked(graph).expect("existing-label source invariant")
}

pub(super) fn score_existing_label_placements_checked(
    graph: &ArenaGraph,
) -> Result<f64, crate::LayoutError> {
    let mut placed_fake_nodes = initial_label_obstacles(graph, false);
    // Source traverses Graph.Nodes, not retained endpoint allocations.
    let live_ids = graph.graph_node_order_cow();
    let live_nodes = live_ids
        .iter()
        .map(|id| (id.0 as usize, &graph.nodes[id.0 as usize]))
        .collect::<Vec<_>>();
    let graph_node_rects = live_nodes
        .iter()
        .filter_map(|(_, node)| {
            Some((
                Rect {
                    origin: graph.position(node.input_id)?,
                    size: node.rect.size,
                },
                node.is_container,
            ))
        })
        .collect::<Vec<_>>();
    let graph_node_rects_only = graph_node_rects
        .iter()
        .map(|(rect, _)| *rect)
        .collect::<Vec<_>>();
    let mut total_score = 0.0;
    let mut missing_placed_label = false;

    for (_, node) in &live_nodes {
        if node.has_icon
            && node.shape != ShapeKind::Image
            && let Some(rect) = positioned_icon_label_rect(graph, node.input_id)
        {
            placed_fake_nodes.push(rect);
            let previous_fake_nodes = &placed_fake_nodes[..placed_fake_nodes.len() - 1];
            let score = overlap_count(rect, &graph_node_rects_only, 0.0)
                + edge_overlap_count(rect, graph.member_edge_indices(), graph, 0.0)
                + overlap_count(rect, previous_fake_nodes, 0.0) * 2;
            total_score += score as f64;
        }

        let Some(rect) = positioned_node_label_rect(graph, node.input_id) else {
            continue;
        };
        let siblings_and_children = sibling_and_child_rects(graph, node.input_id);
        let ancestors = ancestor_rects(graph, node.input_id);
        let score = overlap_count(rect, &siblings_and_children, 5.0)
            + partial_overlap_count(rect, &ancestors, 5.0)
            + edge_overlap_count(rect, graph.member_edge_indices(), graph, 4.0)
            + overlap_count(rect, &placed_fake_nodes, 5.0) * 2;
        total_score += score as f64;
        placed_fake_nodes.push(rect);
    }

    let shared_segments = shared_segment_boxes(graph);
    for (edge_index, edge) in graph.member_edge_entries() {
        if edge.from == edge.to {
            continue;
        }
        let Some(label) = edge.label.as_ref() else {
            continue;
        };
        let Some(label_rect) = label_rect(edge, label.position, label.percentage) else {
            // A nil TopLeft faults only when an overlap scan visits geometry.
            // Empty Graph.Nodes does not resurrect retained edge endpoints.
            let other_segments = graph.member_edge_entries().any(|(other_index, other)| {
                other_index != edge_index && other.route.read().len() > 1
            });
            if !live_nodes.is_empty()
                || other_segments
                || !placed_fake_nodes.is_empty()
                || missing_placed_label
                || !shared_segments.is_empty()
            {
                return Err(crate::LayoutError::SeedInvariant);
            }
            // With no overlap operands the source reaches 0 / label area.
            total_score += 2.0 * (0.0 / (label.size.width * label.size.height));
            missing_placed_label = true;
            continue;
        };
        if missing_placed_label {
            return Err(crate::LayoutError::SeedInvariant);
        }
        let exact_label_overlaps = overlap_count(label_rect, &placed_fake_nodes, 0.0);
        let almost_label_overlaps = overlap_count(label_rect, &placed_fake_nodes, 5.0)
            + (exact_label_overlaps as f64 * 0.5).ceil() as usize;
        let shared_segment_overlaps = overlap_count(label_rect, &shared_segments, 5.0);
        let ancestors = live_nodes
            .iter()
            .copied()
            .filter(|(_, node)| node.input_id != edge.from && node.input_id != edge.to)
            .filter_map(|(node_index, node)| {
                (is_ancestor_of(graph, node_index, edge.from.0 as usize)
                    || is_ancestor_of(graph, node_index, edge.to.0 as usize))
                .then_some(node)
            })
            .filter_map(|node| {
                Some((
                    Rect {
                        origin: graph.position(node.input_id)?,
                        size: node.rect.size,
                    },
                    node.is_container,
                ))
            })
            .collect::<Vec<_>>();
        // TALA's source-shaped loop appends every node that is not a strict
        // ancestor to `nonAncestors`, including the edge endpoints. Keeping
        // endpoints here is observable: a label can overlap the source or
        // target node, and TALA charges that area in the existing-label score.
        let non_ancestors = live_nodes
            .iter()
            .copied()
            .filter(|(node_index, node)| {
                !(node.input_id != edge.from
                    && node.input_id != edge.to
                    && (is_ancestor_of(graph, *node_index, edge.from.0 as usize)
                        || is_ancestor_of(graph, *node_index, edge.to.0 as usize)))
            })
            .filter_map(|(_, node)| {
                Some((
                    Rect {
                        origin: graph.position(node.input_id)?,
                        size: node.rect.size,
                    },
                    node.is_container,
                ))
            })
            .collect::<Vec<_>>();
        let node_overlap_area = label_node_overlap_area(label_rect, &ancestors, 5.0, true)
            + label_node_overlap_area(label_rect, &non_ancestors, 5.0, false);
        let edge_overlap_count = edge_overlap_count(
            label_rect,
            graph
                .member_edge_indices()
                .filter(|candidate| *candidate != edge_index),
            graph,
            5.0,
        );
        let label_area = label_rect.size.width * label_rect.size.height;
        total_score += 2.0 * (node_overlap_area / label_area)
            + 10.0 * exact_label_overlaps as f64
            + almost_label_overlaps as f64
            + 2.0 * edge_overlap_count as f64
            + shared_segment_overlaps as f64;
        placed_fake_nodes.push(label_rect);
    }

    Ok(1.0 / (total_score + 1.0))
}

#[cfg(feature = "diagnostic-traces")]
#[allow(clippy::too_many_arguments)]
fn trace_edge_label_candidate(
    graph: &ArenaGraph,
    edge_index: usize,
    position: LabelPosition,
    percentage: f64,
    rect: Rect,
    placed: &[Rect],
    shared: &[Rect],
    node_area: f64,
    node_count: usize,
    edge_count: usize,
    exact: usize,
    almost: usize,
    shared_count: usize,
    score: f64,
) {
    let id = graph.edge_records[edge_index].tala_id.to_string();
    if !std::env::var("D2RS_TRACE_LABEL_EDGE_OBSTACLES")
        .is_ok_and(|value| value == "*" || value == id)
    {
        return;
    }
    let box_value = |rect: Rect| serde_json::json!({"position":rect.origin,"width":rect.size.width,"height":rect.size.height});
    let nodes = graph
        .graph_node_order_cow()
        .iter()
        .map(|id| &graph.nodes[id.0 as usize])
        .map(|node| {
            serde_json::json!({"id":node.tala_id.to_string(),"position":node.position,
        "width":node.rect.size.width,"height":node.rect.size.height,"container":node.is_container})
        })
        .collect::<Vec<_>>();
    eprintln!(
        "{}",
        serde_json::json!({"event":"label-edge-candidate","id":id,"position":position.d2_name(),"percentage":percentage,
        "box":box_value(rect),"nodes":nodes,"placed":placed.iter().copied().map(box_value).collect::<Vec<_>>(),
        "shared":shared.iter().copied().map(box_value).collect::<Vec<_>>(),"nodeArea":node_area,"nodeCount":node_count,
        "edgeCount":edge_count,"exact":exact,"almost":almost,"sharedCount":shared_count,"score":score})
    );
}

fn score_candidate(
    graph: &ArenaGraph,
    edge_index: usize,
    position: LabelPosition,
    percentage: f64,
    placed_labels: &[Rect],
    shared_segments: &[Rect],
) -> Option<(f64, Rect)> {
    let edge = &graph.edge_records[edge_index];
    let rect = label_rect(edge, position, percentage)?;
    let (node_overlap_area, node_overlap_count) = node_overlap_score(rect, edge_index, graph);
    let mut edge_indices = graph
        .member_edge_indices()
        .filter(|candidate| !is_on_edge(position) || *candidate != edge_index)
        .collect::<Vec<_>>();
    // Recovered findBestEdgeLabelPosition deliberately appends same-side
    // edges from the endpoint cluster a second time. This weights the shared
    // cluster path during overlap scoring; deduplicating the inventory makes
    // right/left candidates incorrectly beat TALA's centered placement.
    if let Some(cluster) = graph.nodes[edge.from.0 as usize].cluster {
        edge_indices.extend(graph.member_edge_indices().filter(|candidate| {
            *candidate != edge_index
                && graph.nodes[graph.edge_records[*candidate].from.0 as usize].cluster
                    == Some(cluster)
        }));
    } else if let Some(cluster) = graph.nodes[edge.to.0 as usize].cluster {
        edge_indices.extend(graph.member_edge_indices().filter(|candidate| {
            *candidate != edge_index
                && graph.nodes[graph.edge_records[*candidate].to.0 as usize].cluster
                    == Some(cluster)
        }));
    }
    let edge_overlap_count = edge_overlap_count(rect, edge_indices.iter().copied(), graph, 5.0)
        + (edge_overlap_count(rect, edge_indices.into_iter(), graph, 0.0) as f64 * 0.5).ceil()
            as usize;
    let exact_label_overlaps = overlap_count(rect, placed_labels, 0.0);
    let almost_label_overlaps = overlap_count(rect, placed_labels, 5.0)
        + (exact_label_overlaps as f64 * 0.5).ceil() as usize;
    let shared_segment_overlaps = overlap_count(rect, shared_segments, 5.0);
    let label_area = rect.size.width * rect.size.height;
    let score = edge_label_overlap_score(
        label_area,
        node_overlap_area,
        exact_label_overlaps,
        edge_overlap_count,
        almost_label_overlaps,
        node_overlap_count,
        shared_segment_overlaps,
    );
    #[cfg(feature = "diagnostic-traces")]
    trace_edge_label_candidate(
        graph,
        edge_index,
        position,
        percentage,
        rect,
        placed_labels,
        shared_segments,
        node_overlap_area,
        node_overlap_count,
        edge_overlap_count,
        exact_label_overlaps,
        almost_label_overlaps,
        shared_segment_overlaps,
        score,
    );
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var("D2_TRACE_LABEL_CANDIDATE_EDGE").is_ok_and(|value| {
        value == "*" || value.split(',').any(|id| id == edge.tala_id.to_string())
    }) {
        eprintln!(
            "LABEL_CANDIDATE edge={} position={} percentage={:016x} rect={:016x},{:016x},{:016x},{:016x} node_area={:016x} node_count={} edge_count={} exact={} almost={} shared={} area={:016x} score={:016x}",
            edge.tala_id,
            position.d2_name(),
            percentage.to_bits(),
            rect.origin.x.to_bits(),
            rect.origin.y.to_bits(),
            rect.size.width.to_bits(),
            rect.size.height.to_bits(),
            node_overlap_area.to_bits(),
            node_overlap_count,
            edge_overlap_count,
            exact_label_overlaps,
            almost_label_overlaps,
            shared_segment_overlaps,
            label_area.to_bits(),
            score.to_bits()
        );
    }
    if crate::internal::trace_env_value("TALA_TRACE_LABEL_EDGE").is_some_and(|value| {
        value == edge_index.to_string()
            || edge.label.as_ref().is_some_and(|label| label.text == value)
    }) {
        eprintln!(
            "LABEL_SCORE_RUST edge={} position={:?} node_area={} node_count={} edge_count={} exact={} almost={} shared={} area={} score={}",
            edge_index,
            position,
            node_overlap_area,
            node_overlap_count,
            edge_overlap_count,
            exact_label_overlaps,
            almost_label_overlaps,
            shared_segment_overlaps,
            label_area,
            score,
        );
    }
    Some((score, rect))
}

fn cluster_bounds(graph: &ArenaGraph, cluster_index: usize) -> Option<Rect> {
    let cluster = graph.clusters.get(cluster_index)?;
    let mut left = f64::INFINITY;
    let mut top = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut bottom = f64::NEG_INFINITY;
    for member in &cluster.members {
        let node = &graph.nodes[member.0 as usize];
        let position = node.position?;
        left = left.min(position.x);
        top = top.min(position.y);
        right = right.max(position.x + node.rect.size.width);
        bottom = bottom.max(position.y + node.rect.size.height);
    }
    left.is_finite().then_some(Rect {
        origin: Point { x: left, y: top },
        size: Size {
            width: right - left,
            height: bottom - top,
        },
    })
}

/// D2 v0.9.0 isClusterPathSharedChecked uses closed interval overlap.
/// Both directional interval bounds must hold; disjoint collinear segments
/// do not share a cluster path.
fn is_cluster_path_shared(graph: &ArenaGraph, edge_index: usize) -> bool {
    let edge = &graph.edge_records[edge_index];
    let from_cluster = graph.nodes[edge.from.0 as usize].cluster;
    let to_cluster = graph.nodes[edge.to.0 as usize].cluster;
    let (cluster_index, adjacent, cluster_at_from) = if let Some(cluster) = from_cluster {
        (cluster, edge.to, true)
    } else if let Some(cluster) = to_cluster {
        (cluster, edge.from, false)
    } else {
        return false;
    };
    if edge.route.len() < 2 {
        return false;
    }
    let (p1, p2) = if cluster_at_from {
        (
            edge.route.point(edge.route.len() - 1),
            edge.route.point(edge.route.len() - 2),
        )
    } else {
        (edge.route.point(0), edge.route.point(1))
    };

    for member in &graph.clusters[cluster_index].members {
        for (other_index, other) in graph.member_edge_entries() {
            if other_index == edge_index || other.route.len() < 2 {
                continue;
            }
            let joins_member_to_adjacent = other.from == *member && other.to == adjacent
                || other.to == *member && other.from == adjacent;
            if !joins_member_to_adjacent {
                continue;
            }
            let (op1, op2) = if cluster_at_from {
                (
                    other.route.point(other.route.len() - 1),
                    other.route.point(other.route.len() - 2),
                )
            } else {
                (other.route.point(0), other.route.point(1))
            };
            if p1.x == p2.x && op1.x == op2.x && p1.x == op1.x {
                if p1.y.max(p2.y) >= op1.y.min(op2.y) && op1.y.max(op2.y) >= p1.y.min(p2.y) {
                    return true;
                }
            } else if p1.y == p2.y
                && op1.y == op2.y
                && p1.y == op1.y
                && (p1.x.max(p2.x) >= op1.x.min(op2.x) && op1.x.max(op2.x) >= p1.x.min(p2.x))
            {
                return true;
            }
        }
    }
    false
}

fn candidate_positions(
    graph: &ArenaGraph,
    edge_index: usize,
    initial: LabelPosition,
) -> Vec<LabelPosition> {
    let mut positions = Vec::with_capacity(12);
    if is_unlocked(initial) {
        if initial == LabelPosition::UnlockedMiddle {
            positions.extend([
                LabelPosition::UnlockedMiddle,
                LabelPosition::UnlockedTop,
                LabelPosition::UnlockedBottom,
            ]);
        } else {
            positions.extend([initial, initial.mirrored(), LabelPosition::UnlockedMiddle]);
        }
        positions.extend_from_slice(EDGE_LABEL_PREFERENCE_ORDER);
    } else if is_cluster_path_shared(graph, edge_index) {
        let edge = &graph.edge_records[edge_index];
        let (cluster_index, member, external, cluster_at_from) =
            if let Some(cluster) = graph.nodes[edge.from.0 as usize].cluster {
                (cluster, edge.from, edge.to, true)
            } else {
                (
                    graph.nodes[edge.to.0 as usize].cluster.unwrap(),
                    edge.to,
                    edge.from,
                    false,
                )
            };
        let cluster = &graph.clusters[cluster_index];
        if let (Some(bounds), Some(member_index)) = (
            cluster_bounds(graph, cluster_index),
            cluster
                .members
                .iter()
                .position(|candidate| *candidate == member),
        ) {
            let external_node = &graph.nodes[external.0 as usize];
            let orientation = graph.sized_box_orientation(
                (external_node.position.unwrap(), external_node.rect.size),
                (bounds.origin, bounds.size),
            );
            let midpoint = (cluster.members.len() - 1) as f64 * 0.5;
            let before = (member_index as f64) < midpoint;
            let after = (member_index as f64) > midpoint;
            let preferred = match (cluster_at_from, orientation) {
                (true, super::Orientation::Top | super::Orientation::Right) if before => {
                    Some(LabelPosition::UnlockedTop)
                }
                (true, super::Orientation::Top | super::Orientation::Right) if after => {
                    Some(LabelPosition::UnlockedBottom)
                }
                (true, super::Orientation::Bottom | super::Orientation::Left) if before => {
                    Some(LabelPosition::UnlockedBottom)
                }
                (true, super::Orientation::Bottom | super::Orientation::Left) if after => {
                    Some(LabelPosition::UnlockedTop)
                }
                (false, super::Orientation::Top | super::Orientation::Right) if before => {
                    Some(LabelPosition::UnlockedBottom)
                }
                (false, super::Orientation::Top | super::Orientation::Right) if after => {
                    Some(LabelPosition::UnlockedTop)
                }
                (false, super::Orientation::Bottom | super::Orientation::Left) if before => {
                    Some(LabelPosition::UnlockedTop)
                }
                (false, super::Orientation::Bottom | super::Orientation::Left) if after => {
                    Some(LabelPosition::UnlockedBottom)
                }
                _ => None,
            };
            positions.extend(preferred);
        }
        positions.push(LabelPosition::UnlockedMiddle);
        positions.extend_from_slice(EDGE_LABEL_PREFERENCE_ORDER);
    } else {
        positions.extend_from_slice(EDGE_LABEL_PREFERENCE_ORDER);
        positions.extend([
            LabelPosition::UnlockedTop,
            LabelPosition::UnlockedBottom,
            LabelPosition::UnlockedMiddle,
        ]);
    }
    positions
}

/// Recovered `getLabelPercentageSearchRange`.
///
/// A label on a cluster edge may slide only along the portion of the route
/// outside that cluster. Searching the entire route can incorrectly find a
/// zero-overlap unlocked placement on the shared/internal portion.
fn label_percentage_search_range(graph: &ArenaGraph, edge_index: usize) -> (f64, f64) {
    let edge = &graph.edge_records[edge_index];
    let Some(label) = edge.label.as_ref() else {
        return (0.0, 1.0);
    };
    let length = route_length(&edge.route.read());
    if length == 0.0 || edge.route.len() < 2 {
        return (0.0, 1.0);
    }
    let half_height = label.size.height * 0.5;
    let half_width = label.size.width * 0.5;

    if graph.nodes[edge.from.0 as usize].cluster.is_some() {
        let p1 = edge.route.point(0);
        let p2 = edge.route.point(1);
        if p1.x == p2.x && (p1.y - p2.y).abs() > half_height {
            return (0.0, ((p1.y - p2.y + half_height).abs() / length).min(1.0));
        }
        if (p1.x - p2.x).abs() > half_width {
            return (0.0, ((p1.x - p2.x + half_width).abs() / length).min(1.0));
        }
    } else if graph.nodes[edge.to.0 as usize].cluster.is_some() {
        let p1 = edge.route.point(edge.route.len() - 1);
        let p2 = edge.route.point(edge.route.len() - 2);
        if p1.x == p2.x && (p1.y - p2.y).abs() > half_height {
            return (
                ((length - (p1.y - p2.y).abs() + half_height) / length).max(0.0),
                1.0,
            );
        }
        if (p1.x - p2.x).abs() > half_width {
            return (
                ((length - (p1.x - p2.x).abs() + half_width) / length).max(0.0),
                1.0,
            );
        }
    }
    (0.0, 1.0)
}

/// Original labeling.Place: one transaction for nodes, icons and edge labels.
pub(super) fn place_labels_checked(graph: &mut ArenaGraph) -> Result<(), super::limits::Error> {
    guarded::place_seed(graph, Some(&ReorderRuntimeContext), guarded::MAX_WORK)
}
#[cfg(feature = "diagnostic-traces")]
pub(super) fn place_labels_probe(
    graph: &mut ArenaGraph,
    context: Option<&dyn super::limits::Context>,
    limit: i64,
) -> Result<(), super::limits::Error> {
    guarded::place_all(graph, context, limit)
}

/// Flat/hierarchy-aware edge-label portion of recovered `Graph.PlaceLabels`.
pub(super) fn place_edge_labels(graph: &mut ArenaGraph) {
    let shared_segments = shared_segment_boxes(graph);
    let mut placed_labels = initial_label_obstacles(graph, true);
    // Graph.PlaceLabels carries icons and every automatically positioned node
    // label forward. Fixed labels are used only while placing their own icon.
    placed_labels.extend(
        graph
            .graph_node_order_cow()
            .iter()
            .flat_map(|&node| node::positioned_node_label_obstacles(graph, node)),
    );

    // Recovered Graph.PlaceLabels ranges over the graph's logical edge slice,
    // whose stable order may differ from arena/serialized edge IDs after tree
    // reconnection. The following stable sort therefore has to start from
    // `edge_order`: already placed labels become obstacles for later edges.
    let mut order: Vec<_> = graph
        .edge_order
        .read()
        .iter()
        .map(|edge_id| edge_id.0 as usize)
        .filter(|index| {
            let edge = &graph.edge_records[*index];
            edge.label.is_some() && edge.from != edge.to && !edge.label_position_fixed
        })
        .collect();
    order.sort_by(|left, right| {
        let left_edge = &graph.edge_records[*left];
        let right_edge = &graph.edge_records[*right];
        match (left_edge.route.len() == 2, right_edge.route.len() == 2) {
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
            _ => route_length(&left_edge.route.read())
                .total_cmp(&route_length(&right_edge.route.read())),
        }
    });

    place_edge_labels_in_order(graph, order, placed_labels, &shared_segments);
}

/// Recovered `Graph.PlaceNewEdgeLabels` for standalone routing. Existing
/// nonrequested non-loop labels, fixed labels, arrowhead labels, icons, and
/// automatic node labels are obstacles; only requested unfixed non-loop labels move.
pub(super) fn place_new_edge_labels(
    graph: &mut ArenaGraph,
    edges: &[usize],
    context: &dyn super::limits::Context,
) -> Result<(), super::limits::Error> {
    guarded::place(
        graph,
        &edges.iter().copied().map(Some).collect::<Vec<_>>(),
        Some(context),
        guarded::MAX_WORK,
    )
}
#[cfg(feature = "diagnostic-traces")]
pub(super) fn place_new_edge_labels_probe(
    graph: &mut ArenaGraph,
    edges: &[Option<usize>],
    context: Option<&dyn super::limits::Context>,
    limit: i64,
) -> Result<(), super::limits::Error> {
    guarded::place(graph, edges, context, limit)
}

fn place_edge_labels_in_order(
    graph: &mut ArenaGraph,
    order: Vec<usize>,
    mut placed_labels: Vec<Rect>,
    shared_segments: &[Rect],
) {
    for edge_index in order {
        let label = graph.edge_records[edge_index].label.as_ref().unwrap();
        let initial_position = label.position;
        let initial_percentage = label.percentage;
        let trace_label =
            crate::internal::trace_env_value("TALA_TRACE_LABEL_EDGE").is_some_and(|value| {
                value == edge_index.to_string()
                    || graph.edge_records[edge_index]
                        .label
                        .as_ref()
                        .is_some_and(|label| label.text == value)
            });
        let mut best = None::<(f64, LabelPosition, f64, Rect)>;
        for position in candidate_positions(graph, edge_index, initial_position) {
            if best.as_ref().is_some_and(|best| best.0 == 0.0) {
                break;
            }
            if is_unlocked(position) && initial_percentage == 0.0 {
                let (mut percentage, end) = label_percentage_search_range(graph, edge_index);
                while percentage < end {
                    if let Some((score, rect)) = score_candidate(
                        graph,
                        edge_index,
                        position,
                        percentage,
                        &placed_labels,
                        shared_segments,
                    ) {
                        if trace_label {
                            eprintln!(
                                "LABEL_CANDIDATE_RUST edge={} position={:?} percentage={} score={} update={}",
                                edge_index,
                                position,
                                percentage,
                                score,
                                best.as_ref().is_none_or(|best| score < best.0)
                            );
                        }
                        if best.as_ref().is_none_or(|best| score < best.0) {
                            best = Some((score, position, percentage, rect));
                        }
                    }
                    if best.as_ref().is_some_and(|best| best.0 == 0.0) {
                        break;
                    }
                    percentage += 0.025;
                }
            } else if let Some((score, rect)) = score_candidate(
                graph,
                edge_index,
                position,
                initial_percentage,
                &placed_labels,
                shared_segments,
            ) {
                if trace_label {
                    eprintln!(
                        "LABEL_CANDIDATE_RUST edge={} position={:?} percentage={} score={} update={}",
                        edge_index,
                        position,
                        initial_percentage,
                        score,
                        best.as_ref().is_none_or(|best| score < best.0)
                    );
                }
                if best.as_ref().is_none_or(|best| score < best.0) {
                    best = Some((score, position, initial_percentage, rect));
                }
            }
        }
        if let Some((_, position, percentage, rect)) = best {
            let label = graph.edge_records[edge_index].label.as_mut().unwrap();
            label.position = position;
            label.percentage = percentage;
            placed_labels.push(rect);
        }
    }
}

fn reorder_nil<T>(value: Option<T>) -> T {
    value.unwrap_or_else(|| {
        std::panic::panic_any("runtime error: invalid memory address or nil pointer dereference")
    })
}
fn reorder_slot(
    route: &RoutePoints,
    index: usize,
) -> Option<super::owner_points::SharedOwnerPoint> {
    route.header().get(index).unwrap_or_else(|| {
        std::panic::panic_any(format!(
            "runtime error: index out of range [{index}] with length {}",
            route.len()
        ))
    })
}
fn is_straight(edge: &ArenaEdge) -> bool {
    let mut p = reorder_slot(&edge.route, 0);
    let mut q = reorder_slot(&edge.route, 1);
    for i in 2..edge.route.len() {
        let r = reorder_slot(&edge.route, i);
        if orientation(
            reorder_nil(p.clone()).get(),
            reorder_nil(q.clone()).get(),
            reorder_nil(r.clone()).get(),
        ) != 0.0
        {
            return false;
        }
        p = q;
        q = r;
    }
    true
}

pub(super) fn is_duplicate(left: &ArenaEdge, right: &ArenaEdge) -> bool {
    left.from == right.from && left.to == right.to || left.from == right.to && left.to == right.from
}

fn tala_arrowhead(edge: &ArenaEdge, source: bool) -> &str {
    if source {
        edge.source_arrowhead.as_deref().unwrap_or("")
    } else {
        edge.target_arrowhead.as_deref().unwrap_or("")
    }
}

pub(super) fn duplicate_arrowheads_match(edge: &ArenaEdge, other: &ArenaEdge) -> bool {
    let source = tala_arrowhead(edge, true);
    let target = tala_arrowhead(edge, false);
    let other_source = tala_arrowhead(other, true);
    let other_target = tala_arrowhead(other, false);

    if other.from == edge.to {
        other_source == target && other_target == source
    } else {
        other_source == source && other_target == target
    }
}

fn has_overlapping_end(graph: &ArenaGraph, edge_index: usize) -> bool {
    let edge = &graph.edge_records[edge_index];
    let start = reorder_slot(&edge.route, 0);
    let end = reorder_slot(&edge.route, edge.route.len() - 1);
    let equal = |a: Option<super::owner_points::SharedOwnerPoint>,
                 b: &Option<super::owner_points::SharedOwnerPoint>| {
        a.zip(b.as_ref()).is_some_and(|(a, b)| a.get() == b.get())
    };
    for (node, endpoint) in [(edge.from, &start), (edge.to, &end)] {
        let node = reorder_nil(graph.nodes.get(node.0 as usize));
        for other in node.edges.header().iter_slots() {
            let other = reorder_nil(other).0 as usize;
            if other == edge_index {
                continue;
            }
            let other = &graph.edge_records[other].route;
            if equal(reorder_slot(other, 0), endpoint)
                || equal(reorder_slot(other, other.len() - 1), endpoint)
            {
                return true;
            }
        }
    }
    false
}

/// Pinned public coordinator.go1633: context polls and route-slot rollback.
/// This boundary has no work budget; the standalone guarded helper is separate.
struct ReorderRuntimeContext;
impl super::limits::Context for ReorderRuntimeContext {
    fn done(&self) -> Option<bool> {
        super::cancellation_done()
    }
    fn error(&self) -> Option<super::limits::ContextError> {
        super::cancellation_error()
    }
}
fn reorder_check(context: &dyn super::limits::Context) -> Result<(), super::limits::Error> {
    match context.error() {
        Some(cause) => Err(super::limits::Error::Context {
            location: "ReorderDuplicates".into(),
            cause,
        }),
        None => Ok(()),
    }
}
pub(super) fn reorder_duplicates(graph: &mut ArenaGraph) {
    let _ = reorder_duplicates_with_context(graph, Some(&ReorderRuntimeContext));
}
pub(super) fn reorder_duplicates_with_context(
    graph: &mut ArenaGraph,
    context: Option<&dyn super::limits::Context>,
) -> Result<(), super::limits::Error> {
    let edges = graph.edge_order.header();
    reorder_duplicates_in_header(graph, &edges, context)
}
fn reorder_duplicates_in_header(
    graph: &mut ArenaGraph,
    edges: &super::owner_pointer_slices::SharedEdgeSlice,
    context: Option<&dyn super::limits::Context>,
) -> Result<(), super::limits::Error> {
    let context = reorder_nil(context);
    reorder_check(context)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut snapshots = Vec::new();
    for edge in edges.iter_slots() {
        let edge = reorder_nil(edge).0 as usize;
        if seen.insert(edge) {
            let points = graph.edge_records[edge].route.clone();
            let values = points.header().iter_slots().collect::<Vec<_>>();
            snapshots.push((edge, points, values));
        }
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reorder_duplicates_inner(graph, edges, context)
    }));
    if !matches!(&outcome, Ok(Ok(()))) {
        // Go copies active pointer slots, without restoring point coordinates
        // or any spare capacity outside those original active slots.
        for (edge, points, values) in snapshots {
            for (i, value) in values.into_iter().enumerate() {
                points
                    .header()
                    .write_capacity_slot(i, value)
                    .expect("saved route slot");
            }
            graph.edge_records[edge].route.replace(points);
        }
    }
    match outcome {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
pub(super) fn reorder_duplicates_checked(
    graph: &mut ArenaGraph,
) -> Result<(), super::limits::Error> {
    reorder_duplicates_with_context(graph, Some(&ReorderRuntimeContext))
}

pub(super) fn reorder_duplicates_in_edges(graph: &mut ArenaGraph, edges: &[usize]) {
    let edges = super::owner_pointer_slices::SharedEdgeSlice::from_values(
        &edges
            .iter()
            .map(|i| crate::EdgeId(*i as u32))
            .collect::<Vec<_>>(),
    )
    .expect("requested edges");
    let _ = reorder_duplicates_in_header(graph, &edges, Some(&ReorderRuntimeContext));
}
fn reorder_duplicates_inner(
    graph: &mut ArenaGraph,
    edges: &super::owner_pointer_slices::SharedEdgeSlice,
    context: &dyn super::limits::Context,
) -> Result<(), super::limits::Error> {
    for (_edge_order, edge_index) in edges.iter_slots().enumerate() {
        reorder_check(context)?;
        let edge_index = reorder_nil(edge_index).0 as usize;
        if graph.edge_records[edge_index].has_table_column()
            || graph.edge_records[edge_index].label.is_none()
            || !is_straight(&graph.edge_records[edge_index])
        {
            continue;
        }
        let edge_has_overlap = has_overlapping_end(graph, edge_index);
        let mut duplicates = Vec::new();
        for (_other_order, other_index) in edges.iter_slots().enumerate() {
            reorder_check(context)?;
            let other_index = reorder_nil(other_index).0 as usize;
            if other_index == edge_index
                || !is_duplicate(
                    &graph.edge_records[edge_index],
                    &graph.edge_records[other_index],
                )
                || !is_straight(&graph.edge_records[other_index])
            {
                continue;
            }
            let other_has_overlap = edge_has_overlap || has_overlapping_end(graph, other_index);
            let arrowheads_match = if other_has_overlap {
                let edge = &graph.edge_records[edge_index];
                let other = &graph.edge_records[other_index];
                duplicate_arrowheads_match(edge, other)
            } else {
                true
            };
            trace_reorder_duplicates!(serde_json::json!({
                "stage": "ReorderDuplicates",
                "event": "candidate",
                "edge_order": _edge_order,
                "other_order": _other_order,
                "overlap": other_has_overlap,
                "arrow_match": arrowheads_match,
                "included": arrowheads_match,
            }));
            if arrowheads_match {
                duplicates.push(_other_order);
            }
        }
        if duplicates.len() <= 1 {
            continue;
        }

        let edge = &graph.edge_records[edge_index];
        let start = edge.route.point(0);
        let end = edge.route.point(edge.route.len() - 1);
        let mut source_order = Vec::with_capacity(duplicates.len() + 1);
        source_order.push(edge_index);
        source_order.extend(
            duplicates
                .iter()
                .map(|i| reorder_nil(edges.get(*i).unwrap()).0 as usize),
        );
        let mut target_order = source_order.clone();
        if end.y - start.y > end.x - start.x {
            crate::internal::go_sort::sort_by(&mut source_order, |left, right| {
                graph.edge_records[*left].route.point(0).x
                    < graph.edge_records[*right].route.point(0).x
            });
            crate::internal::go_sort::sort_by(&mut target_order, |left, right| {
                graph.edge_records[*left]
                    .route
                    .point(graph.edge_records[*left].route.len() - 1)
                    .x
                    < graph.edge_records[*right]
                        .route
                        .point(graph.edge_records[*right].route.len() - 1)
                        .x
            });
        } else {
            crate::internal::go_sort::sort_by(&mut source_order, |left, right| {
                graph.edge_records[*left].route.point(0).y
                    < graph.edge_records[*right].route.point(0).y
            });
            crate::internal::go_sort::sort_by(&mut target_order, |left, right| {
                graph.edge_records[*left]
                    .route
                    .point(graph.edge_records[*left].route.len() - 1)
                    .y
                    < graph.edge_records[*right]
                        .route
                        .point(graph.edge_records[*right].route.len() - 1)
                        .y
            });
        }
        trace_reorder_duplicates!(serde_json::json!({
            "stage": "ReorderDuplicates",
            "event": "orders",
            "edge_order": _edge_order,
            "source_order": source_order.iter().map(|index| edges.iter_slots().position(|candidate| candidate==Some(crate::EdgeId(*index as u32))).unwrap()).collect::<Vec<_>>(),
            "target_order": target_order.iter().map(|index| edges.iter_slots().position(|candidate| candidate==Some(crate::EdgeId(*index as u32))).unwrap()).collect::<Vec<_>>(),
        }));
        reorder_check(context)?;
        if source_order != target_order {
            continue;
        }
        let current = source_order
            .iter()
            .position(|candidate| *candidate == edge_index)
            .unwrap();
        if current == 0 || current + 1 == source_order.len() {
            continue;
        }
        let first = source_order[0];
        let last = *source_order.last().unwrap();
        if graph.edge_records[first].label.is_some() && graph.edge_records[last].label.is_some() {
            continue;
        }
        let swap_with = if graph.edge_records[last].label.is_none() {
            last
        } else if graph.edge_records[first].label.is_none() {
            first
        } else {
            continue;
        };
        reorder_check(context)?;
        let reverse = graph.edge_records[swap_with].from == graph.edge_records[edge_index].to;
        let labelled_points = graph.edge_records[edge_index].route.clone();
        let outside_points = graph.edge_records[swap_with].route.clone();
        #[cfg(feature = "diagnostic-traces")]
        let _labelled_route_before = trace_reorder_duplicate_route(&labelled_points.read());
        #[cfg(feature = "diagnostic-traces")]
        let _outside_route_before = trace_reorder_duplicate_route(&outside_points.read());
        graph.edge_records[edge_index].route.replace(outside_points);
        graph.edge_records[swap_with].route.replace(labelled_points);
        if reverse {
            graph.edge_records[edge_index].route.reverse();
            graph.edge_records[swap_with].route.reverse();
        }
        trace_reorder_duplicates!(serde_json::json!({
            "stage": "ReorderDuplicates",
            "event": "swap",
            "edge_order": _edge_order,
            "swap_order": edges.iter_slots().position(|candidate| candidate==Some(crate::EdgeId(swap_with as u32))).unwrap(),
            "reverse": reverse,
            "edge_before": _labelled_route_before,
            "other_before": _outside_route_before,
            "edge_after": trace_reorder_duplicate_route(&graph.edge_records[edge_index].route.read()),
            "other_after": trace_reorder_duplicate_route(&graph.edge_records[swap_with].route.read()),
        }));
    }
    reorder_check(context)
}

// Development observer corresponding to Go labeling.place entry.
#[cfg(feature = "diagnostic-traces")]
pub(super) fn trace_label_placement_input(graph: &ArenaGraph) {
    use std::fmt::Write as _;
    if std::env::var("D2_TRACE_LABEL_PLACEMENT").as_deref() != Ok("1") {
        return;
    }
    let mut snapshot = String::new();
    writeln!(
        &mut snapshot,
        "LABEL_PLACEMENT begin nodes={} edges={}",
        graph.nodes.len(),
        graph.edge_order.len()
    )
    .unwrap();
    for id in &graph.edge_order.read() {
        let edge = &graph.edge_records[id.0 as usize];
        let Some(label) = &edge.label else {
            continue;
        };
        let points = edge
            .route
            .read()
            .iter()
            .map(|p| format!("{:016x},{:016x}", p.x.to_bits(), p.y.to_bits()))
            .collect::<Vec<_>>()
            .join(";");
        writeln!(&mut snapshot, "LABEL_PLACEMENT edge={} from={} to={} position={} percentage={:016x} fixed={} points={}", edge.tala_id, graph.nodes[edge.from.0 as usize].tala_id, graph.nodes[edge.to.0 as usize].tala_id, label.position.d2_name(), label.percentage.to_bits(), edge.label_position_fixed, points).unwrap();
    }
    writeln!(&mut snapshot, "LABEL_PLACEMENT end").unwrap();
    eprint!("{snapshot}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ArrowheadLabel, ContentAlignment, Edge, EdgeId, EdgeLabel, Graph, Insets, Node, NodeId,
        ShapeKind, Size,
    };

    fn reorder_fixture() -> ArenaGraph {
        let mut input = Graph::default();
        let a = input.add_node(Node::new(
            "a",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let b = input.add_node(Node::new(
            "b",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        for i in 0..3 {
            let edge = input.add_edge(Edge {
                source: a,
                target: b,
            });
            input.set_edge_route(
                edge,
                vec![
                    Point {
                        x: 20.0 + 10.0 * i as f64,
                        y: 100.0,
                    },
                    Point {
                        x: 20.0 + 10.0 * i as f64,
                        y: 300.0,
                    },
                ],
            );
            if i == 1 {
                input.set_edge_label(
                    edge,
                    Some(EdgeLabel {
                        text: "middle".into(),
                        size: Size {
                            width: 20.0,
                            height: 10.0,
                        },
                        position: LabelPosition::UnlockedTop,
                        percentage: 0.0,
                    }),
                );
            }
        }
        ArenaGraph::from_input(&input)
    }
    struct ReorderTestContext {
        polls: std::cell::Cell<u64>,
        stop: u64,
        panic: bool,
        point: Option<super::super::owner_points::SharedOwnerPoint>,
    }
    impl super::super::limits::Context for ReorderTestContext {
        // The public boundary must call Err even with an open Done channel.
        fn done(&self) -> Option<bool> {
            Some(false)
        }
        fn error(&self) -> Option<super::super::limits::ContextError> {
            let n = self.polls.get() + 1;
            self.polls.set(n);
            if n == self.stop {
                if let Some(point) = &self.point {
                    point.set(Point {
                        x: 9876.5,
                        y: -1234.5,
                    });
                }
                if self.panic {
                    panic!("reorder context panic");
                }
                return Some(super::super::limits::ContextError::Cancelled);
            }
            None
        }
    }
    #[test]
    fn reorder_public_swaps_headers_and_reverses_retained_slots() {
        let mut g = reorder_fixture();
        let last = &mut g.edge_records[2];
        std::mem::swap(&mut last.from, &mut last.to);
        last.route.reverse();
        let middle = g.edge_records[1].route.clone();
        let last = g.edge_records[2].route.clone();
        let old_middle = middle.slot(0).unwrap();
        let old_last = last.slot(1).unwrap();
        reorder_duplicates_checked(&mut g).unwrap();
        assert!(
            g.edge_records[1]
                .route
                .header()
                .same_backing_allocation(&last.header())
        );
        assert!(
            g.edge_records[2]
                .route
                .header()
                .same_backing_allocation(&middle.header())
        );
        assert!(last.slot(0).unwrap().same_allocation(&old_last));
        assert!(middle.slot(1).unwrap().same_allocation(&old_middle));
    }
    #[test]
    fn reorder_public_restores_slots_on_each_cancel_and_panic() {
        for panic in [false, true] {
            for stop in 1..=10 {
                let mut g = reorder_fixture();
                let last = &mut g.edge_records[2];
                std::mem::swap(&mut last.from, &mut last.to);
                last.route.reverse();
                let saved = g
                    .edge_records
                    .iter()
                    .map(|e| {
                        (
                            e.route.clone(),
                            e.route.header().iter_slots().collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>();
                let c = ReorderTestContext {
                    polls: 0.into(),
                    stop,
                    panic,
                    point: None,
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    reorder_duplicates_with_context(&mut g, Some(&c))
                }));
                if panic {
                    assert!(result.is_err());
                } else {
                    assert!(result.unwrap().is_err());
                }
                for (e, (header, slots)) in g.edge_records.iter().zip(saved) {
                    assert!(e.route.header().same_backing_allocation(&header.header()));
                    assert_eq!(e.route.len(), header.len());
                    assert_eq!(e.route.capacity(), header.capacity());
                    for (i, slot) in slots.into_iter().enumerate() {
                        assert!(e.route.slot(i).unwrap().same_allocation(&slot.unwrap()));
                    }
                }
            }
        }
    }
    #[test]
    fn reorder_public_rollback_keeps_context_coordinate_mutation() {
        for panic in [false, true] {
            let mut g = reorder_fixture();
            let point = g.edge_records[0].route.slot(0).unwrap();
            let c = ReorderTestContext {
                polls: 0.into(),
                stop: 10,
                panic,
                point: Some(point.clone()),
            };
            let original = g.edge_records[1].route.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                reorder_duplicates_with_context(&mut g, Some(&c))
            }));
            if panic {
                assert!(result.is_err());
            } else {
                assert!(result.unwrap().is_err());
            }
            assert!(
                g.edge_records[1]
                    .route
                    .header()
                    .same_backing_allocation(&original.header())
            );
            assert_eq!(
                point.get(),
                Point {
                    x: 9876.5,
                    y: -1234.5
                }
            );
        }
    }
    #[test]
    fn reorder_straight_two_null_points_does_not_dereference_them() {
        let mut g = reorder_fixture();
        g.edge_records[0]
            .route
            .replace(RoutePoints::pointer_literal(&[None, None]));
        assert!(is_straight(&g.edge_records[0]));
        g.edge_records[0].route.append_slot(None);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| is_straight(
                &g.edge_records[0]
            )))
            .is_err()
        );
    }

    #[test]
    fn edge_label_area_uses_live_nodes_and_preserves_duplicate_membership() {
        let mut input = Graph::default();
        let from = input.add_node(Node::new(
            "from",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let to = input.add_node(Node::new(
            "to",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let retired = input.add_node(Node::new(
            "retired",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let edge = input.add_edge(Edge {
            source: from,
            target: to,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(from, Point { x: -100.0, y: 0.0 });
        graph.set_position(to, Point { x: 100.0, y: 0.0 });
        graph.set_position(retired, Point::default());
        graph.node_order = vec![to, from];
        let candidate = Rect {
            origin: Point { x: 10.0, y: 10.0 },
            size: Size {
                width: 20.0,
                height: 20.0,
            },
        };
        assert_eq!(
            node_overlap_score(candidate, edge.0 as usize, &graph),
            (0.0, 0)
        );
        graph.node_order = vec![to, retired, from];
        assert_eq!(
            node_overlap_score(candidate, edge.0 as usize, &graph),
            (800.0, 2)
        );
        graph.node_order.push(retired);
        assert_eq!(
            node_overlap_score(candidate, edge.0 as usize, &graph),
            (1600.0, 4)
        );
    }

    fn label_collision_candidate(route: &[Point]) -> PositionedArrowheadLabel {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 20.0, 20.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 20.0, 20.0, ShapeKind::Rectangle));
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_arrowhead_labels(
            edge,
            crate::EdgeArrowheadLabels {
                source: Some(ArrowheadLabel {
                    text: "endpoint".into(),
                    size: Size {
                        width: 20.0,
                        height: 10.0,
                    },
                    color: None,
                }),
                target: None,
            },
        );
        let graph = ArenaGraph::from_input(&input);
        positioned_arrowhead_label_for_route(0, &graph.edge_records[0], route, false).unwrap()
    }

    #[test]
    fn label_collision_rejects_nan_orientations_from_finite_routes() {
        // Every route coordinate is finite and far outside the box in Y.
        // Subtracting the first two X coordinates overflows before the fused
        // orientation expression evaluates the right-boundary side values.
        let candidate =
            label_collision_candidate(&[Point { x: -5.0, y: 16.0 }, Point { x: 95.0, y: 16.0 }]);
        assert_eq!(
            candidate.rect,
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: 20.0,
                    height: 10.0,
                },
            }
        );
        let prior = [
            Point { x: 1e308, y: 1e308 },
            Point {
                x: -1e308,
                y: 5e307,
            },
            Point {
                x: -5e307,
                y: 1e308,
            },
        ];
        assert!(
            prior
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite())
        );
        let top_right = Point { x: 20.0, y: 0.0 };
        let bottom_right = Point { x: 20.0, y: 10.0 };
        assert!(orientation(prior[0], prior[1], top_right).is_nan());
        assert!(orientation(prior[0], prior[1], bottom_right).is_nan());
        assert!(!segments_intersect(
            top_right,
            bottom_right,
            prior[0],
            prior[1]
        ));
        assert!(!positioned_arrowhead_label_overlaps_route(
            &candidate, &prior
        ));
    }

    #[test]
    fn label_collision_rejects_present_nan_boxes() {
        let candidate =
            label_collision_candidate(&[Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 0.0 }]);
        assert!(candidate.rect.origin.x.is_nan());
        assert!(candidate.rect.origin.y.is_nan());
        assert!(!positioned_arrowhead_label_overlaps_route(
            &candidate,
            &[
                Point { x: 0.0, y: 0.0 },
                Point { x: 1.0, y: 0.0 },
                Point { x: 2.0, y: 0.0 },
            ],
        ));
    }

    #[test]
    fn label_collision_accepts_crossing_after_orientation_underflow() {
        let rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 1e-200,
                height: 1e-200,
            },
        };
        let start = Point {
            x: -1e-200,
            y: 5e-201,
        };
        let end = Point {
            x: 2e-200,
            y: 5e-201,
        };
        let top_right = Point { x: 1e-200, y: 0.0 };
        let bottom_right = Point {
            x: 1e-200,
            y: 1e-200,
        };
        for side in [
            orientation(top_right, bottom_right, start),
            orientation(top_right, bottom_right, end),
            orientation(start, end, top_right),
            orientation(start, end, bottom_right),
        ] {
            assert_eq!(side, 0.0);
        }
        assert!(!contains(rect, start, 0.0));
        assert!(!contains(rect, end, 0.0));
        // Go's all-zero branch tests closed coordinate intervals, including
        // this crossing whose endpoints lie outside the opposite segment.
        assert!(segments_intersect(top_right, bottom_right, start, end));
        assert!(rect_overlaps_segment(rect, start, end, 0.0));
    }

    #[test]
    fn label_collision_preserves_finite_disjoint_and_crossing_controls() {
        let candidate =
            label_collision_candidate(&[Point { x: -5.0, y: 16.0 }, Point { x: 95.0, y: 16.0 }]);
        assert!(!positioned_arrowhead_label_overlaps_route(
            &candidate,
            &[
                Point { x: 300.0, y: 300.0 },
                Point {
                    x: 200.0,
                    y: -200.0
                },
                Point {
                    x: 400.0,
                    y: -200.0
                },
            ],
        ));
        assert!(positioned_arrowhead_label_overlaps_route(
            &candidate,
            &[
                Point { x: 10.0, y: 30.0 },
                Point { x: 10.0, y: -20.0 },
                Point { x: 30.0, y: -20.0 },
            ],
        ));
    }

    #[test]
    fn label_geometry_preserves_go_axis_distances() {
        // geo.EuclideanDistance takes axis-aligned abs branches before
        // squaring, preserving both extreme cases below.
        let tiny = [
            Point { x: 0.0, y: 7.25 },
            Point {
                x: f64::from_bits(0x10),
                y: 7.25,
            },
        ];
        assert_eq!(route_length(&tiny).to_bits(), 0x10);
        let (point, segment) = point_at_distance(&tiny, f64::from_bits(0x8)).unwrap();
        assert_eq!(point.x.to_bits(), 0x8);
        assert_eq!(point.y, 7.25);
        assert_eq!(segment, 0);

        let huge = [Point { x: 3.5, y: -1e200 }, Point { x: 3.5, y: 1e200 }];
        assert_eq!(route_length(&huge).to_bits(), 0x6984e718d7d7625a);
        for (position, expected) in [
            (LabelPosition::UnlockedTop, Point { x: 10.0, y: -5.0 }),
            (LabelPosition::UnlockedMiddle, Point { x: -7.0, y: -5.0 }),
            (LabelPosition::UnlockedBottom, Point { x: -24.0, y: -5.0 }),
        ] {
            assert_eq!(
                route_label_top_left(&huge, position, 0.5, 20.75, 10.25, 3.5),
                Some(expected)
            );
        }
    }

    #[test]
    #[cfg(target_arch = "aarch64")]
    fn label_geometry_matches_go_arm64_point_interpolation() {
        // Go 1.27.0 arm64 evaluates a*(1-t)+b*t with a fused addition,
        // rather than the current delta interpolation's one-ULP-low Y.
        let route = [Point { x: 10.25, y: -7.75 }, Point { x: 88.1, y: 51.9 }];
        assert_eq!(route_length(&route).to_bits(), 0x405884d016c2afc3);
        let (point, segment) = point_at_distance(&route, 37.125).unwrap();
        assert_eq!(point.x.to_bits(), 0x4043dc093f628b86);
        assert_eq!(point.y.to_bits(), 0x402da8cb48c2d393);
        assert_eq!(segment, 0);
    }

    #[test]
    #[cfg(target_arch = "aarch64")]
    fn label_geometry_matches_go_arm64_oblique_arrowheads() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 20.0, 20.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 20.0, 20.0, ShapeKind::Rectangle));
        input.add_edge(Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        let edge = &mut graph.edge_records[0];
        edge.source_arrow = true;
        edge.target_arrow = true;
        edge.source_arrowhead = Some("triangle".into());
        edge.target_arrowhead = Some("filled-diamond".into());
        let label = ArrowheadLabel {
            text: "port".into(),
            size: Size {
                width: 20.75,
                height: 10.25,
            },
            color: None,
        };
        edge.source_arrowhead_label = Some(label.clone());
        edge.target_arrowhead_label = Some(label);
        let route = [Point { x: 10.25, y: -7.75 }, Point { x: 88.1, y: 51.9 }];
        for (is_target, x_bits, y_bits) in [
            (false, 0x40393766e057fece, 0xc0292cd44f2ac8bb),
            (true, 0x4052f4c69420ff8d, 0x40389e60c49fe973),
        ] {
            let positioned = positioned_arrowhead_label_for_route(0, edge, &route, is_target)
                .expect("the endpoint label is present");
            assert_eq!(positioned.rect.origin.x.to_bits(), x_bits);
            assert_eq!(positioned.rect.origin.y.to_bits(), y_bits);
            assert_eq!(positioned.rect.size.width, 20.75);
            assert_eq!(positioned.rect.size.height, 10.25);
        }
    }

    #[test]
    fn coincident_arrowhead_label_geometry_preserves_go_nan_boxes() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 20.0, 20.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 20.0, 20.0, ShapeKind::Rectangle));
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_arrowhead_labels(
            edge,
            crate::EdgeArrowheadLabels {
                source: Some(ArrowheadLabel {
                    text: "port".into(),
                    size: Size {
                        width: 20.0,
                        height: 10.0,
                    },
                    color: None,
                }),
                target: None,
            },
        );
        let graph = ArenaGraph::from_input(&input);
        for route in [
            vec![Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 0.0 }],
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 0.0, y: 0.0 },
                Point { x: 100.0, y: 0.0 },
            ],
        ] {
            let positioned =
                positioned_arrowhead_label_for_route(0, &graph.edge_records[0], &route, false)
                    .expect("Go retains a present label on a coincident endpoint segment");
            assert!(positioned.rect.origin.x.is_nan());
            assert!(positioned.rect.origin.y.is_nan());
            // The independent Go 1.27.0 arm64 capture retains these NaN
            // payloads; other targets keep the portable semantic assertion.
            #[cfg(target_arch = "aarch64")]
            {
                assert_eq!(positioned.rect.origin.x.to_bits(), 0x7ff8000000000000);
                assert_eq!(positioned.rect.origin.y.to_bits(), 0x7ff8000000000000);
            }
            assert_eq!(
                positioned.rect.size,
                Size {
                    width: 20.0,
                    height: 10.0
                }
            );
        }
        let route = [
            Point { x: 0.0, y: 0.0 },
            Point { x: 0.0, y: 0.0 },
            Point { x: 100.0, y: 0.0 },
        ];
        let origin =
            route_label_top_left(&route, LabelPosition::UnlockedMiddle, 0.0, 20.0, 10.0, 2.0)
                .expect("Go retains the point selected on the zero-length first segment");
        assert!(origin.x.is_nan());
        assert!(origin.y.is_nan());
        #[cfg(target_arch = "aarch64")]
        {
            assert_eq!(origin.x.to_bits(), 0x7ff8000000000000);
            assert_eq!(origin.y.to_bits(), 0x7ff8000000000000);
        }
    }

    #[test]
    fn cluster_path_candidates_require_closed_interval_overlap() {
        use crate::internal::{ClusterArrangement, ClusterState};
        for horizontal in [false, true] {
            for from_cluster in [false, true] {
                let mut input = Graph::default();
                let adjacent = input.add_node(Node::new(
                    "external",
                    Size {
                        width: 40.0,
                        height: 40.0,
                    },
                ));
                let a = input.add_node(Node::new(
                    "a",
                    Size {
                        width: 40.0,
                        height: 40.0,
                    },
                ));
                let b = input.add_node(Node::new(
                    "b",
                    Size {
                        width: 40.0,
                        height: 40.0,
                    },
                ));
                for member in [a, b] {
                    let (source, target) = if from_cluster {
                        (member, adjacent)
                    } else {
                        (adjacent, member)
                    };
                    input.add_edge(Edge { source, target });
                }
                let mut graph = ArenaGraph::from_input(&input);
                graph.clusters.push(ClusterState {
                    members: vec![a, b],
                    arrangement: ClusterArrangement::Column,
                    desired_arrangement: ClusterArrangement::Column,
                    padding: 5.0,
                    vessel_tala_id: 17,
                    fixed_size: false,
                });
                graph.nodes[a.0 as usize].cluster = Some(0);
                graph.nodes[b.0 as usize].cluster = Some(0);
                let point = |t| {
                    if horizontal {
                        Point { x: t, y: 0.0 }
                    } else {
                        Point { x: 0.0, y: t }
                    }
                };
                graph.edge_records[0]
                    .route
                    .replace(RoutePoints::literal(&[point(0.0), point(10.0)]));
                for (offset, shared) in [
                    (-20.0, false),
                    (-10.0, true),
                    (5.0, true),
                    (10.0, true),
                    (20.0, false),
                ] {
                    graph.edge_records[1]
                        .route
                        .replace(RoutePoints::literal(&[point(offset), point(offset + 10.0)]));
                    assert_eq!(
                        is_cluster_path_shared(&graph, 0),
                        shared,
                        "horizontal={horizontal} from_cluster={from_cluster} offset={offset}"
                    );
                    let first = candidate_positions(&graph, 0, LabelPosition::Unset)[0];
                    if shared {
                        assert!(is_unlocked(first));
                    } else {
                        assert_eq!(first, LabelPosition::OutsideTopCenter);
                    }
                }
            }
        }
    }

    fn node(name: &str, width: f64, height: f64, shape: ShapeKind) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.into(),
            size: Size { width, height },
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
            content_insets: Insets::uniform(60.0),
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
            shape,
        }
    }

    #[test]
    fn overlapping_end_matches_route_endpoint_values_not_node_alignment() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 20.0, 20.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 20.0, 20.0, ShapeKind::Rectangle));
        let other_target = input.add_node(node("other", 20.0, 20.0, ShapeKind::Rectangle));
        let edge = input.add_edge(Edge { source, target });
        let other = input.add_edge(Edge {
            source,
            target: other_target,
        });
        input.set_edge_route(
            edge,
            vec![Point { x: 1.0, y: 0.0 }, Point { x: 100.0, y: 100.0 }],
        );
        input.set_edge_route(
            other,
            vec![Point { x: 1.0, y: 0.0 }, Point { x: 200.0, y: 200.0 }],
        );
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 100.0, y: 100.0 });
        graph.set_position(other_target, Point { x: 200.0, y: 200.0 });
        assert!(has_overlapping_end(&graph, edge.0 as usize));

        graph.edge_records[other.0 as usize]
            .route
            .replace_point(0, Point { x: 1.0, y: 1.0 });
        graph.set_position(target, Point { x: 100.0, y: 0.0 });
        graph.set_position(other_target, Point { x: 200.0, y: 0.0 });
        assert!(!has_overlapping_end(&graph, edge.0 as usize));
    }

    #[test]
    fn duplicate_filter_compares_raw_arrowheads_and_reverses_direction() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 20.0, 20.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 20.0, 20.0, ShapeKind::Rectangle));
        let forward = input.add_edge(Edge { source, target });
        let same_direction = input.add_edge(Edge { source, target });
        let reverse = input.add_edge(Edge {
            source: target,
            target: source,
        });
        let mut graph = ArenaGraph::from_input(&input);

        // The source helper compares actual arrowhead strings.
        graph.edge_records[forward.0 as usize].target_arrowhead = Some("triangle".into());
        graph.edge_records[same_direction.0 as usize].target_arrowhead = Some("triangle".into());
        assert!(duplicate_arrowheads_match(
            &graph.edge_records[forward.0 as usize],
            &graph.edge_records[same_direction.0 as usize]
        ));

        // Reversed edges match after swapping source and target identities.
        graph.edge_records[forward.0 as usize].source_arrowhead = Some("triangle".into());
        graph.edge_records[forward.0 as usize].target_arrowhead = None;
        graph.edge_records[reverse.0 as usize].source_arrowhead = None;
        graph.edge_records[reverse.0 as usize].target_arrowhead = Some("triangle".into());
        assert!(duplicate_arrowheads_match(
            &graph.edge_records[forward.0 as usize],
            &graph.edge_records[reverse.0 as usize]
        ));

        // Explicit shapes are compared by identity, not just arrow presence.
        graph.edge_records[same_direction.0 as usize].target_arrowhead = Some("diamond".into());
        assert!(!duplicate_arrowheads_match(
            &graph.edge_records[forward.0 as usize],
            &graph.edge_records[same_direction.0 as usize]
        ));
    }

    #[test]
    fn duplicate_ordering_treats_signed_zero_as_equal_like_go() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 20.0, 20.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 20.0, 20.0, ShapeKind::Rectangle));
        let edges = (0..3)
            .map(|_| input.add_edge(Edge { source, target }))
            .collect::<Vec<_>>();
        input.set_edge_label(
            edges[1],
            Some(EdgeLabel {
                text: "middle".into(),
                size: Size {
                    width: 20.0,
                    height: 12.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
        let mut graph = ArenaGraph::from_input(&input);
        graph.edge_records[edges[0].0 as usize]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: -0.0, y: 40.0 },
                Point { x: 0.0, y: 200.0 },
            ]));
        graph.edge_records[edges[1].0 as usize]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: 0.0, y: 40.0 },
                Point { x: -0.0, y: 200.0 },
            ]));
        graph.edge_records[edges[2].0 as usize]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: -1.0, y: 40.0 },
                Point { x: -1.0, y: 200.0 },
            ]));

        reorder_duplicates(&mut graph);

        let labelled_route = &graph.edge_records[edges[1].0 as usize].route.read();
        assert_eq!(labelled_route[0].x.to_bits(), (-0.0f64).to_bits());
        assert_eq!(labelled_route[1].x.to_bits(), 0.0f64.to_bits());
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn orientation_matches_go_127_arm64_fused_rounding() {
        let p = Point {
            x: 274877906944.0,
            y: -112141207424.4068,
        };
        let q = Point {
            x: -1460899053800.1104,
            y: 1942854608477.2622,
        };
        let r = Point {
            x: -4932452975288.331,
            y: 6052846240280.6,
        };
        assert_eq!(orientation(p, q, r).to_bits(), 0x41ab_f24b_b170_e2e0);
    }

    #[test]
    fn recovered_box_predicates_preserve_direction_and_strict_edge_contact() {
        let outer = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 100.0,
                height: 100.0,
            },
        };
        let inner = Rect {
            origin: Point { x: 10.0, y: 10.0 },
            size: Size {
                width: 20.0,
                height: 20.0,
            },
        };
        let touching = Rect {
            origin: Point { x: 100.0, y: 0.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };

        assert!(covers(outer, inner));
        assert!(!covers(inner, outer));
        assert!(covers(outer, outer), "covers is inclusive at equality");
        assert!(overlaps(outer, inner, 0.0));
        assert!(!overlaps(outer, touching, 0.0));
    }

    #[test]
    fn recovered_route_label_geometry_matches_d2_dependency() {
        let edge = ArenaEdge {
            invisible: false,
            label_position_fixed: false,
            source_label_positions: Default::default(),
            tala_id: 0,
            input_id: EdgeId(0),
            from: NodeId(0),
            to: NodeId(1),
            route: RoutePoints::literal(&[
                Point { x: 96.0, y: 586.0 },
                Point { x: 96.0, y: 692.0 },
            ]),
            is_curve: false,
            source_arrow: false,
            target_arrow: false,
            source_arrowhead: None,
            target_arrowhead: None,
            source_arrowhead_label: None,
            target_arrowhead_label: None,
            style: crate::EdgeStyle::default(),
            source_table_column: None,
            target_table_column: None,
            source_table_column_count: None,
            target_table_column_count: None,
            min_width: 100.0,
            min_height: 31.0,
            label: Some(EdgeLabel {
                text: "request token".into(),
                size: Size {
                    width: 90.0,
                    height: 21.0,
                },
                position: LabelPosition::OutsideBottomCenter,
                percentage: 0.0,
            }),
        };

        assert_eq!(
            edge_label_top_left(&edge, LabelPosition::OutsideBottomCenter, 0.0, 90.0, 21.0,),
            Some(Point { x: -1.0, y: 629.0 })
        );
    }

    #[test]
    fn recovered_arrowhead_label_geometry_matches_d2_dependency() {
        let label = ArrowheadLabel {
            text: "port".into(),
            size: Size {
                width: 20.0,
                height: 10.0,
            },
            color: None,
        };
        let edge = ArenaEdge {
            invisible: false,
            label_position_fixed: false,
            source_label_positions: Default::default(),
            tala_id: 0,
            input_id: EdgeId(0),
            from: NodeId(0),
            to: NodeId(1),
            route: RoutePoints::literal(&[Point { x: 0.0, y: 0.0 }, Point { x: 100.0, y: 0.0 }]),
            is_curve: false,
            source_arrow: true,
            target_arrow: true,
            source_arrowhead: None,
            target_arrowhead: None,
            source_arrowhead_label: Some(label.clone()),
            target_arrowhead_label: Some(label),
            style: crate::EdgeStyle::default(),
            source_table_column: None,
            target_table_column: None,
            source_table_column_count: None,
            target_table_column_count: None,
            min_width: 10.0,
            min_height: 10.0,
            label: None,
        };

        assert_eq!(
            arrowhead_label_rect(&edge, false),
            Some(Rect {
                origin: Point { x: 5.0, y: -18.0 },
                size: Size {
                    width: 20.0,
                    height: 10.0,
                },
            })
        );
        assert_eq!(
            arrowhead_label_rect(&edge, true),
            Some(Rect {
                origin: Point { x: 75.0, y: -18.0 },
                size: Size {
                    width: 20.0,
                    height: 10.0,
                },
            })
        );
    }

    #[test]
    fn positioned_arrowhead_label_cost_matches_recovered_components() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 20.0, 20.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 20.0, 20.0, ShapeKind::Rectangle));
        input.add_edge(Edge { source, target });
        input.add_edge(Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 300.0, y: 0.0 });
        graph.set_turn_cost(10.0);
        let candidate = PositionedArrowheadLabel {
            rect: Rect {
                origin: Point { x: 21.0, y: 5.0 },
                size: Size {
                    width: 2.0,
                    height: 2.0,
                },
            },
            edge_index: 0,
            is_target: false,
            text: "first".into(),
        };
        let different = PositionedArrowheadLabel {
            edge_index: 1,
            is_target: false,
            text: "second".into(),
            ..candidate.clone()
        };
        assert!(positioned_arrowhead_label_cost(&graph, &candidate, &[different], 0).is_infinite());

        let matching = PositionedArrowheadLabel {
            edge_index: 1,
            is_target: false,
            text: "first".into(),
            ..candidate.clone()
        };
        // One node overlap contributes 4*turnCost; two overlapping routes
        // contribute one turnCost each.
        assert_eq!(
            positioned_arrowhead_label_cost(&graph, &candidate, &[matching], 2),
            60.0
        );
        assert!(positioned_arrowhead_label_overlaps_route(
            &candidate,
            &[Point { x: 0.0, y: 6.0 }, Point { x: 30.0, y: 6.0 }]
        ));
    }

    #[test]
    fn arrowhead_overlap_cost_excludes_other_component_storage() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 20.0, 20.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 20.0, 20.0, ShapeKind::Rectangle));
        let unrelated = input.add_node(node("unrelated", 20.0, 20.0, ShapeKind::Rectangle));
        input.add_edge(Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 300.0, y: 0.0 });
        graph.set_position(unrelated, Point { x: 100.0, y: 0.0 });
        graph.set_turn_cost(10.0);
        let candidate = PositionedArrowheadLabel {
            rect: Rect {
                origin: Point { x: 105.0, y: 5.0 },
                size: Size {
                    width: 2.0,
                    height: 2.0,
                },
            },
            edge_index: 0,
            is_target: false,
            text: "label".into(),
        };
        assert_eq!(
            positioned_arrowhead_label_cost(&graph, &candidate, &[], 0),
            40.0
        );
        graph.node_order = vec![source, target];
        assert_eq!(
            positioned_arrowhead_label_cost(&graph, &candidate, &[], 0),
            0.0
        );
    }

    #[test]
    fn fixed_edge_labels_are_obstacles_and_are_not_repositioned() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 100.0, 100.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 100.0, 100.0, ShapeKind::Rectangle));
        let edge = input.add_edge(Edge { source, target });
        let label = EdgeLabel {
            text: "fixed".into(),
            size: Size {
                width: 50.0,
                height: 20.0,
            },
            position: LabelPosition::UnlockedTop,
            percentage: 0.17,
        };
        input.set_edge_label(edge, Some(label.clone()));
        input.tala_edge_label_fixed[edge.0 as usize] = true;
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 200.0, y: 0.0 });
        graph.edge_records[0].route.replace(RoutePoints::literal(&[
            Point { x: 100.0, y: 50.0 },
            Point { x: 200.0, y: 50.0 },
        ]));
        assert_eq!(initial_label_obstacles(&graph, true).len(), 1);
        place_edge_labels(&mut graph);
        assert_eq!(graph.edge_records[0].label, Some(label.clone()));
        place_new_edge_labels(&mut graph, &[0], &ReorderRuntimeContext).unwrap();
        assert_eq!(graph.edge_records[0].label, Some(label));
    }

    #[test]
    fn recovered_edge_label_score_weights_exact_labels_before_nodes() {
        assert_eq!(edge_label_overlap_score(100.0, 50.0, 2, 4, 5, 3, 7), 47.0);
    }

    #[test]
    fn unset_edge_label_is_a_seed_failure_but_loops_skip_scoring() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 100.0, 100.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 100.0, 100.0, ShapeKind::Rectangle));
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: "unset".into(),
                size: Size {
                    width: 0.0,
                    height: 21.0,
                },
                position: LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 200.0, y: 0.0 });
        assert!(matches!(
            score_existing_label_placements_checked(&graph),
            Err(crate::LayoutError::SeedInvariant)
        ));
        graph.edge_records[0].to = source;
        assert_eq!(
            score_existing_label_placements_checked(&graph).unwrap(),
            1.0
        );
    }

    #[test]
    fn existing_label_scoring_uses_live_nodes_and_dereferences_only_visited_boxes() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 100.0, 100.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 100.0, 100.0, ShapeKind::Rectangle));
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: "label".into(),
                size: Size {
                    width: 50.0,
                    height: 20.0,
                },
                position: LabelPosition::InsideMiddleCenter,
                percentage: 0.5,
            }),
        );
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 200.0, y: 0.0 });
        graph.edge_records[0].route.replace(RoutePoints::literal(&[
            Point { x: 0.0, y: 50.0 },
            Point { x: 200.0, y: 50.0 },
        ]));
        assert_eq!(
            score_existing_label_placements_checked(&graph).unwrap(),
            0.25
        );
        graph.node_order.clear();
        assert_eq!(
            score_existing_label_placements_checked(&graph).unwrap(),
            1.0
        );
        let label = graph.edge_records[0].label.as_mut().unwrap();
        label.position = LabelPosition::Unset;
        label.size.width = 0.0;
        assert!(
            score_existing_label_placements_checked(&graph)
                .unwrap()
                .is_nan()
        );
        graph.edge_records[0].label.as_mut().unwrap().size.width = 50.0;
        assert_eq!(
            score_existing_label_placements_checked(&graph).unwrap(),
            1.0
        );
        graph.node_order.push(source);
        assert!(matches!(
            score_existing_label_placements_checked(&graph),
            Err(crate::LayoutError::SeedInvariant)
        ));
    }

    #[test]
    fn existing_edge_label_score_includes_endpoint_overlap_area() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", 100.0, 100.0, ShapeKind::Rectangle));
        let target = input.add_node(node("target", 100.0, 100.0, ShapeKind::Rectangle));
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: "documents".into(),
                size: Size {
                    width: 50.0,
                    height: 20.0,
                },
                position: LabelPosition::InsideMiddleCenter,
                percentage: 0.5,
            }),
        );

        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 200.0, y: 0.0 });
        graph.edge_records[edge.0 as usize]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: 0.0, y: 50.0 },
                Point { x: 200.0, y: 50.0 },
            ]));

        // TALA puts the source and target into nonAncestors. The label
        // overlaps 25x20 of the source, and non-container overlap adds the
        // complete 50x20 label area: 2*(1500/1000) = 3, then normalize.
        assert_eq!(score_existing_label_placements(&graph), 0.25);
    }

    #[test]
    fn recovered_segment_intersection_accepts_collinear_overlap() {
        assert!(segments_intersect(
            Point { x: 0.0, y: 10.0 },
            Point { x: 20.0, y: 10.0 },
            Point { x: 15.0, y: 10.0 },
            Point { x: 30.0, y: 10.0 },
        ));
        assert!(segments_intersect(
            Point { x: 0.0, y: 10.0 },
            Point { x: 20.0, y: 10.0 },
            Point { x: 20.0, y: 10.0 },
            Point { x: 30.0, y: 10.0 },
        ));
    }

    #[test]
    fn recovered_segment_intersection_rejects_disjoint_collinear_segments() {
        assert!(!segments_intersect(
            Point { x: 0.0, y: 10.0 },
            Point { x: 20.0, y: 10.0 },
            Point { x: 21.0, y: 10.0 },
            Point { x: 30.0, y: 10.0 },
        ));
    }

    #[test]
    fn shared_segments_are_swept_into_one_recovered_obstacle() {
        let ranges = merged_shared_ranges(vec![
            AxisSegment {
                fixed: 40.0,
                start: 0.0,
                end: 100.0,
            },
            AxisSegment {
                fixed: 40.0,
                start: 20.0,
                end: 80.0,
            },
            AxisSegment {
                fixed: 40.0,
                start: 30.0,
                end: 120.0,
            },
        ]);

        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].fixed, 40.0);
        assert_eq!(ranges[0].start, 20.0);
        assert_eq!(ranges[0].end, 100.0);
    }

    #[test]
    fn outside_node_labels_become_edge_label_obstacles() {
        assert_eq!(
            external_node_label_rect(
                Rect {
                    origin: Point { x: 275.0, y: 315.0 },
                    size: Size {
                        width: 10.0,
                        height: 10.0,
                    },
                },
                ExternalLabel {
                    size: Size {
                        width: 9.0,
                        height: 21.0,
                    },
                    side: crate::ExternalSide::Top,
                    alignment: crate::ExternalAlignment::Center,
                    automatic: false,
                    reserve_space: true,
                },
            ),
            Rect {
                origin: Point { x: 275.5, y: 289.0 },
                size: Size {
                    width: 9.0,
                    height: 21.0,
                },
            }
        );
    }
}
