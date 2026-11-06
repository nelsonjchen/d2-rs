// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Final geometry evaluation and diagnostic [`crate::Score`] construction.
//!
//! Invalidities, collisions, crossings, bends, length, area, and alignment are
//! measured from published boxes and routes after the pipeline finishes.

use super::*;
use crate::Score;

pub(super) fn evaluate_layout(
    graph: &Graph,
    boxes: &BTreeMap<NodeId, Rect>,
    routes: &BTreeMap<EdgeId, Vec<Point>>,
) -> Score {
    let mut invalidities = 0;
    let mut node_area = 0.0;
    let mut max_right: f64 = 0.0;
    let mut max_bottom: f64 = 0.0;
    let ordered_boxes = boxes.iter().collect::<Vec<_>>();

    for (index, (node, rect)) in ordered_boxes.iter().enumerate() {
        if !rect.origin.x.is_finite() || !rect.origin.y.is_finite() {
            invalidities += 1;
        }
        node_area += rect.size.width * rect.size.height;
        max_right = max_right.max(rect.right());
        max_bottom = max_bottom.max(rect.bottom());

        if let Some(parent) = graph.nodes[node.0 as usize].parent {
            let parent_rect = boxes[&parent];
            if rect.origin.x < parent_rect.origin.x
                || rect.origin.y < parent_rect.origin.y
                || rect.right() > parent_rect.right()
                || rect.bottom() > parent_rect.bottom()
            {
                invalidities += 1;
            }
        }

        for (other, other_rect) in ordered_boxes.iter().skip(index + 1) {
            let related =
                is_ancestor(graph, **node, **other) || is_ancestor(graph, **other, **node);
            if !related && rect.overlaps(**other_rect) {
                invalidities += 1;
            }
        }
    }

    let mut bends = 0;
    let mut route_length = 0.0;
    let mut route_collisions = 0;
    for (edge_id, route) in routes {
        bends += route.len().saturating_sub(2) as u64;
        route_length += route
            .windows(2)
            .map(|segment| {
                (segment[0].x - segment[1].x).abs() + (segment[0].y - segment[1].y).abs()
            })
            .sum::<f64>();
        let edge = &graph.edges[edge_id.0 as usize];
        for (node, rect) in boxes {
            if *node == edge.source
                || *node == edge.target
                || is_ancestor(graph, *node, edge.source)
                || is_ancestor(graph, *node, edge.target)
            {
                continue;
            }
            route_collisions += route
                .windows(2)
                .filter(|segment| segment_hits_rect(segment[0], segment[1], *rect))
                .count() as u64;
        }
    }

    let unaligned_pairs = graph
        .edges()
        .filter(|(edge_id, edge)| {
            if !routes.contains_key(edge_id) {
                return false;
            }
            let source = boxes[&edge.source].center();
            let target = boxes[&edge.target].center();
            (source.x - target.x).abs() > 0.001 && (source.y - target.y).abs() > 0.001
        })
        .count() as u64;

    let edge_order = routes.keys().copied().collect::<Vec<_>>();
    Score {
        invalidities,
        route_collisions,
        crossings: count_crossings(routes, &edge_order),
        label_collisions: 0,
        bends,
        route_length_milli: (route_length * 1000.0).max(0.0) as u64,
        area_milli: (((max_right * max_bottom) / node_area.max(1.0)) * 1000.0) as u64,
        unaligned_pairs,
    }
}

/// Recovered TALA candidate score components. The pinned D2 wrapper compares
/// the quality penalty (route turns, diagonal segments, crossings, and label
/// penalty) exactly, then compares unrounded `Graph.Area()`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct RaceScoreBreakdown {
    pub(super) route_turns: f64,
    pub(super) diagonal_segments: f64,
    pub(super) crossings: f64,
    pub(super) label_score: f64,
    pub(super) label_penalty: f64,
}

impl RaceScoreBreakdown {
    /// Penalty compared first by D2 v0.9.0's RaceSeeds wrapper.
    pub(super) fn selection_penalty(self) -> f64 {
        self.route_turns + self.diagonal_segments + self.crossings + self.label_penalty
    }
}

/// Exact ordering tuple used by the pinned D2 wrapper: quality penalty
/// first, unrounded graph area second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct RaceSelectionScore {
    pub(super) penalty: f64,
    pub(super) area: f64,
}

impl RaceSelectionScore {
    pub(super) fn compare(self, other: Self) -> std::cmp::Ordering {
        compare_finite(self.penalty, other.penalty, false)
            .then_with(|| compare_finite(self.area, other.area, true))
    }
}

fn compare_finite(left: f64, right: f64, nonnegative: bool) -> std::cmp::Ordering {
    let valid = |value: f64| value.is_finite() && (!nonnegative || value >= 0.0);
    match (valid(left), valid(right)) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => std::cmp::Ordering::Equal,
        (true, true) if left < right => std::cmp::Ordering::Less,
        (true, true) if left > right => std::cmp::Ordering::Greater,
        (true, true) => std::cmp::Ordering::Equal,
    }
}

pub(super) fn evaluate_race_seed_layout(
    routes: &BTreeMap<EdgeId, Vec<Point>>,
    clustered_edges: &BTreeSet<EdgeId>,
    label_score: f64,
    edge_order: &[EdgeId],
) -> RaceScoreBreakdown {
    let mut route_turns = 0.0;
    let mut diagonal_segments = 0.0;
    for (edge_id, route) in routes {
        if clustered_edges.contains(edge_id) {
            continue;
        }
        route_turns += route.len().saturating_sub(2) as f64 * 0.5;
        for segment in route.windows(2) {
            if segment[0].x != segment[1].x && segment[0].y != segment[1].y {
                diagonal_segments += 3.0;
            }
        }
    }
    let crossings = count_crossings(routes, edge_order) as f64;

    RaceScoreBreakdown {
        route_turns,
        diagonal_segments,
        crossings,
        label_score,
        label_penalty: 1.0 - label_score,
    }
}

/// Recovered TALA graph bounds, with the node rounding mode selected by the
/// caller. Edge bounds always round their corners, matching `Edge.getBoundingBox`.
fn race_seed_bounds(graph: &ArenaGraph, round_node_dimensions: bool) -> Option<(Point, Point)> {
    // Match the source's live Graph.Nodes slice. The stable arena can retain
    // aliases that the current graph owner has already removed.
    let all_nodes = graph.node_order.clone();
    let mut bounds = if round_node_dimensions {
        graph.fixed_node_bounds(&all_nodes)
    } else {
        graph.unrounded_fixed_node_bounds(&all_nodes)
    };
    // The rounded legacy helper predates the direct unrounded node bounds and
    // still needs this adapter-side recovery for labels not materialized as
    // ExternalLabel carriers.
    if round_node_dimensions {
        for node in &graph.nodes {
            let outside = matches!(
                node.label_position,
                LabelPosition::OutsideTopLeft
                    | LabelPosition::OutsideTopCenter
                    | LabelPosition::OutsideTopRight
                    | LabelPosition::OutsideLeftTop
                    | LabelPosition::OutsideLeftMiddle
                    | LabelPosition::OutsideLeftBottom
                    | LabelPosition::OutsideRightTop
                    | LabelPosition::OutsideRightMiddle
                    | LabelPosition::OutsideRightBottom
                    | LabelPosition::OutsideBottomLeft
                    | LabelPosition::OutsideBottomCenter
                    | LabelPosition::OutsideBottomRight
            );
            let Some(label_rect) = outside
                .then(|| labels::positioned_node_label_rect(graph, node.input_id))
                .flatten()
            else {
                continue;
            };
            let Some(position) = node.position else {
                continue;
            };
            let leftmost = graph
                .nodes
                .iter()
                .filter_map(|other| other.position)
                .all(|other| other.x >= position.x);
            let topmost = graph
                .nodes
                .iter()
                .filter_map(|other| other.position)
                .all(|other| other.y >= position.y);
            let rightmost = graph.nodes.iter().all(|other| {
                other.position.is_none_or(|other_position| {
                    other_position.x + other.rect.size.width <= position.x + node.rect.size.width
                })
            });
            let bottommost = graph.nodes.iter().all(|other| {
                other.position.is_none_or(|other_position| {
                    other_position.y + other.rect.size.height <= position.y + node.rect.size.height
                })
            });
            let expand =
                |bounds: &mut Option<(Point, Point)>, top_left: Point, bottom_right: Point| {
                    *bounds = Some(match *bounds {
                        Some((mut low, mut high)) => {
                            low.x = low.x.min(top_left.x);
                            low.y = low.y.min(top_left.y);
                            high.x = high.x.max(bottom_right.x);
                            high.y = high.y.max(bottom_right.y);
                            (low, high)
                        }
                        None => (top_left, bottom_right),
                    });
                };
            if label_rect.origin.x < position.x {
                let padding = if leftmost { 5.0 } else { 10.0 };
                expand(
                    &mut bounds,
                    Point {
                        x: (label_rect.origin.x - padding).floor(),
                        y: label_rect.origin.y,
                    },
                    Point {
                        x: label_rect.origin.x,
                        y: label_rect.origin.y,
                    },
                );
            }
            if label_rect.origin.y < position.y {
                let padding = if topmost { 5.0 } else { 10.0 };
                expand(
                    &mut bounds,
                    Point {
                        x: label_rect.origin.x,
                        y: (label_rect.origin.y - padding).floor(),
                    },
                    Point {
                        x: label_rect.origin.x,
                        y: label_rect.origin.y,
                    },
                );
            }
            if label_rect.origin.x > position.x + node.rect.size.width {
                let padding = if rightmost { 5.0 } else { 10.0 };
                expand(
                    &mut bounds,
                    label_rect.origin,
                    Point {
                        x: (label_rect.origin.x + label_rect.size.width + padding).ceil(),
                        y: label_rect.origin.y,
                    },
                );
            }
            if label_rect.origin.y > position.y + node.rect.size.height {
                let padding = if bottommost { 5.0 } else { 10.0 };
                expand(
                    &mut bounds,
                    label_rect.origin,
                    Point {
                        x: label_rect.origin.x,
                        y: (label_rect.origin.y + label_rect.size.height + padding).ceil(),
                    },
                );
            }
        }
    }
    let mut extend = |top_left: Point, bottom_right: Point| {
        bounds = Some(match bounds {
            Some((mut low, mut high)) => {
                low.x = low.x.min(top_left.x);
                low.y = low.y.min(top_left.y);
                high.x = high.x.max(bottom_right.x);
                high.y = high.y.max(bottom_right.y);
                (low, high)
            }
            None => (top_left, bottom_right),
        });
    };

    for edge_id in &graph.edge_order.read() {
        let edge = &graph.edge_records[edge_id.0 as usize];
        if let Some((top_left, bottom_right)) = bounds::routed_edge_bounds(edge) {
            extend(top_left, bottom_right);
        }
    }

    bounds
}

/// Recovered legacy `Graph.getArea` score term, retained for trace diagnostics.
/// This is not the area tie-breaker used by D2 v0.9.0's outer seed selector.
pub(super) fn race_seed_area_term(graph: &ArenaGraph) -> f64 {
    let Some((top_left, bottom_right)) = race_seed_bounds(graph, true) else {
        return 0.0;
    };
    let width = (bottom_right.x.round() - top_left.x.round()).abs();
    let height = (bottom_right.y.round() - top_left.y.round()).abs();
    let area = (width * height).trunc() as u64;
    format!("0.{area}").parse::<f64>().unwrap_or(0.0)
}

/// Recovered `Graph.Area()` used as the strict second key by the pinned
/// wrapper. Node extents remain unrounded; edge bounds use the source's rounded
/// corners and labels/icons are included in both.
pub(super) fn race_seed_graph_area(graph: &ArenaGraph) -> f64 {
    if graph.node_order.is_empty() {
        return 0.0;
    }
    let Some((top_left, bottom_right)) = race_seed_bounds(graph, false) else {
        return 0.0;
    };
    (top_left.x - bottom_right.x).abs() * (top_left.y - bottom_right.y).abs()
}

fn is_ancestor(graph: &Graph, ancestor: NodeId, descendant: NodeId) -> bool {
    let mut cursor = graph.nodes[descendant.0 as usize].parent;
    while let Some(parent) = cursor {
        if parent == ancestor {
            return true;
        }
        cursor = graph.nodes[parent.0 as usize].parent;
    }
    false
}

fn segment_hits_rect(first: Point, second: Point, rect: Rect) -> bool {
    if first.x == second.x {
        first.x > rect.origin.x
            && first.x < rect.right()
            && first.y.max(second.y) > rect.origin.y
            && first.y.min(second.y) < rect.bottom()
    } else if first.y == second.y {
        first.y > rect.origin.y
            && first.y < rect.bottom()
            && first.x.max(second.x) > rect.origin.x
            && first.x.min(second.x) < rect.right()
    } else {
        false
    }
}

fn count_crossings(routes: &BTreeMap<EdgeId, Vec<Point>>, edge_order: &[EdgeId]) -> u64 {
    let mut crossings = 0;
    let routed = edge_order
        .iter()
        .filter_map(|edge_id| routes.get(edge_id).map(|route| (edge_id, route)))
        .collect::<Vec<_>>();
    for (index, (_edge_id, route)) in routed.iter().enumerate() {
        for (_other_id, other_route) in routed.iter().skip(index + 1) {
            for first in route.windows(2) {
                for second in other_route.windows(2) {
                    if segments_cross(first[0], first[1], second[0], second[1]) {
                        crossings += 1;
                    }
                }
            }
        }
    }
    crossings
}

fn segments_cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    // TALA's Edge.nonSharedCrossingCount first calls nonParallelIntersection
    // (the orientation test below), then suppresses intersections where an
    // endpoint of the other segment lies on the current segment's axis. It
    // does not identify shared graph endpoints; this is why the graph-edge
    // identity shortcut is intentionally absent from count_crossings.
    let first_orientation = orientation(a, b, c);
    let second_orientation = orientation(a, b, d);
    if equal_signs(first_orientation, second_orientation) {
        return false;
    }
    let third_orientation = orientation(c, d, a);
    let fourth_orientation = orientation(c, d, b);
    if equal_signs(third_orientation, fourth_orientation) {
        return false;
    }

    if a.x == b.x && (c.x == a.x || d.x == a.x) {
        return false;
    }
    if a.y == b.y && (c.y == a.y || d.y == a.y) {
        return false;
    }
    true
}

fn orientation(p: Point, q: Point, r: Point) -> f64 {
    (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
}

fn equal_signs(left: f64, right: f64) -> bool {
    (left > 0.0 && right > 0.0) || (left == 0.0 && right == 0.0) || (left < 0.0 && right < 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn race_score_breakdown_preserves_each_release_component() {
        let routes = BTreeMap::from([(
            EdgeId(0),
            vec![point(0.0, 0.0), point(5.0, 5.0), point(20.0, 20.0)],
        )]);
        let breakdown = evaluate_race_seed_layout(&routes, &BTreeSet::new(), 0.75, &[EdgeId(0)]);

        assert_eq!(breakdown.route_turns, 0.5);
        assert_eq!(breakdown.diagonal_segments, 6.0);
        assert_eq!(breakdown.crossings, 0.0);
        assert_eq!(breakdown.label_score, 0.75);
        assert_eq!(breakdown.label_penalty, 0.25);
        assert_eq!(breakdown.selection_penalty(), 6.75);
    }

    #[test]
    fn race_score_skips_route_terms_for_clustered_edges() {
        let routes = BTreeMap::from([(
            EdgeId(0),
            vec![point(0.0, 0.0), point(5.0, 5.0), point(20.0, 20.0)],
        )]);
        let breakdown =
            evaluate_race_seed_layout(&routes, &BTreeSet::from([EdgeId(0)]), 1.0, &[EdgeId(0)]);

        assert_eq!(breakdown.route_turns, 0.0);
        assert_eq!(breakdown.diagonal_segments, 0.0);
        assert_eq!(breakdown.crossings, 0.0);
        assert_eq!(breakdown.label_penalty, 0.0);
        assert_eq!(breakdown.selection_penalty(), 0.0);
    }

    #[test]
    fn race_score_clamps_turns_for_routes_shorter_than_two_points() {
        for points in [Vec::new(), vec![point(3.0, 7.0)]] {
            let routes = BTreeMap::from([(EdgeId(0), points)]);
            let breakdown = evaluate_race_seed_layout(&routes, &BTreeSet::new(), 1.0, &[EdgeId(0)]);
            assert_eq!(breakdown.route_turns, 0.0);
            assert_eq!(breakdown.selection_penalty(), 0.0);
        }
    }

    #[test]
    fn race_score_crossing_predicate_counts_diagonal_intersection() {
        let routes = BTreeMap::from([
            (EdgeId(0), vec![point(0.0, 0.0), point(10.0, 10.0)]),
            (EdgeId(1), vec![point(0.0, 10.0), point(10.0, 0.0)]),
        ]);
        assert_eq!(count_crossings(&routes, &[EdgeId(0), EdgeId(1)]), 1);
    }

    #[test]
    fn race_score_crossing_predicate_suppresses_axis_endpoint_contact() {
        let routes = BTreeMap::from([
            (EdgeId(0), vec![point(0.0, 0.0), point(10.0, 0.0)]),
            (EdgeId(1), vec![point(5.0, 0.0), point(5.0, 10.0)]),
        ]);
        assert_eq!(count_crossings(&routes, &[EdgeId(0), EdgeId(1)]), 0);
    }

    #[test]
    fn race_score_preserves_tala_edge_order_for_asymmetric_endpoint_contacts() {
        let routes = BTreeMap::from([
            (EdgeId(0), vec![point(0.0, 0.0), point(10.0, 0.0)]),
            (EdgeId(1), vec![point(5.0, -5.0), point(6.0, 0.0)]),
        ]);

        // TALA's nonSharedCrossingCount applies the axis-endpoint exclusion
        // to the edge on the left side of the pair only.  The final scalar
        // therefore depends on Graph.Edges order, even though the geometry
        // and the pair set are unchanged.
        assert_eq!(count_crossings(&routes, &[EdgeId(0), EdgeId(1)]), 0);
        assert_eq!(count_crossings(&routes, &[EdgeId(1), EdgeId(0)]), 1);
    }
}
