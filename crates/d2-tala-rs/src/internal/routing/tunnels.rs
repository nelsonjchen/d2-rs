// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned routing/tunnel.go range selection, with live geometry and OVG guards.
use super::channel_solver::{maximum, minimum};
use super::ovg_groups::group;
use super::ovg_intersections::{Budget, check, descendant, rect, step};
use super::{ArenaGraph, NodeId, Rect};
use crate::internal::owner_groups::OwnerGroupKind::Sequence;

// Use the source's rejection comparisons: a NaN is not ordered, so replacing
// these with positive overlap comparisons changes VisibilityGraphCandidate.
fn visibility_candidate(first: Rect, second: Rect, horizontal: bool) -> bool {
    if horizontal {
        !(first.origin.y > second.bottom() + 0.0 || first.bottom() + 0.0 < second.origin.y)
    } else {
        !(first.origin.x > second.right() + 0.0 || first.right() + 0.0 < second.origin.x)
    }
}
fn is_blocked(obstacle: Rect, first: Rect, second: Rect, horizontal: bool) -> bool {
    if horizontal {
        if !(obstacle.origin.x >= first.right() && obstacle.right() <= second.origin.x) {
            return false;
        }
        obstacle.origin.y <= maximum(first.origin.y, second.origin.y)
            && obstacle.bottom() >= minimum(first.bottom(), second.bottom())
    } else {
        if !(obstacle.origin.y >= first.bottom() && obstacle.bottom() <= second.origin.y) {
            return false;
        }
        obstacle.origin.x <= maximum(first.origin.x, second.origin.x)
            && obstacle.right() >= minimum(first.right(), second.right())
    }
}
fn subtract_range(next: &mut Vec<(f64, f64)>, (start, end): (f64, f64), lo: f64, hi: f64) {
    if lo <= start && hi >= end {
        return;
    }
    if lo > start && hi < end {
        next.extend([(start, lo), (hi, end)]);
    } else if lo <= start && hi < end && hi > start {
        next.push((hi, end));
    } else if lo > start && hi >= end && lo < end {
        next.push((start, lo));
    } else {
        next.push((start, end));
    }
}
fn sequence_right(graph: &ArenaGraph, node: NodeId, rect: Rect) -> f64 {
    if group(graph, node, Sequence).is_some_and(|sequence| sequence.last(graph) != Some(node)) {
        rect.right() - 35.0
    } else {
        rect.right()
    }
}

/// `tunnelRangesBetween`: None is a nil range slice, Some may contain an empty
/// nonnil slice. The caller supplies the current Graph.Nodes header order.
/// No final check is present in the source.
pub(super) fn tunnel_ranges_between(
    graph: &ArenaGraph,
    nodes: &[NodeId],
    first: NodeId,
    second: NodeId,
    filter_short: bool,
    budget: Budget<'_, '_>,
) -> Result<Option<(Vec<(f64, f64)>, bool)>, ()> {
    check(budget)?;
    step(budget)?;
    let horizontal = visibility_candidate(rect(graph, first), rect(graph, second), true);
    if !horizontal {
        step(budget)?;
        if !visibility_candidate(rect(graph, first), rect(graph, second), false) {
            return Ok(None);
        }
    }
    let a = rect(graph, first);
    let b = rect(graph, second);
    let initial = if horizontal {
        (
            maximum(a.origin.y, b.origin.y),
            minimum(a.bottom(), b.bottom()),
        )
    } else {
        let ar = sequence_right(graph, first, a);
        let br = sequence_right(graph, second, b);
        (maximum(a.origin.x, b.origin.x), minimum(ar, br))
    };
    // A callback here can change the boxes, but not the captured initial range.
    step(budget)?;
    let mut ranges = vec![initial];
    for &other in nodes {
        step(budget)?;
        if other == first || other == second {
            continue;
        }
        // All four relations are evaluated, even when the first is true.
        let below_a = descendant(graph, Some(other), first, budget)?;
        let above_a = descendant(graph, Some(first), other, budget)?;
        let below_b = descendant(graph, Some(other), second, budget)?;
        let above_b = descendant(graph, Some(second), other, budget)?;
        if below_a || above_a || below_b || above_b {
            continue;
        }
        let (o, a, b) = (rect(graph, other), rect(graph, first), rect(graph, second));
        if is_blocked(o, a, b, horizontal) || is_blocked(o, b, a, horizontal) {
            return Ok(None);
        }
        let (a, b, v) = if horizontal {
            (a.origin.x, b.origin.x, o.origin.x)
        } else {
            (a.origin.y, b.origin.y, o.origin.y)
        };
        if !((a < v && v < b) || (b < v && v < a)) {
            continue;
        }
        let mut next = Vec::new();
        for range in ranges {
            step(budget)?;
            let obstacle = rect(graph, other);
            let (lo, hi) = if horizontal {
                (obstacle.origin.y, obstacle.bottom())
            } else {
                (obstacle.origin.x, obstacle.right())
            };
            subtract_range(&mut next, range, lo, hi);
        }
        ranges = Vec::new();
        check(budget)?;
        for range in next {
            step(budget)?;
            if !filter_short || range.1 - range.0 >= 40.0 {
                ranges.push(range);
            }
        }
    }
    Ok(Some((ranges, horizontal)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use crate::internal::routing::route_search_guard::OvgBuildBudget;
    use std::cell::Cell;

    fn graph() -> ArenaGraph {
        let data = serde_json::json!({"nodes":[
            {"id":1,"width":100,"height":100,"topLeft":{"x":0,"y":0}},
            {"id":2,"width":100,"height":100,"topLeft":{"x":500,"y":0}},
            {"id":3,"width":100,"height":20,"topLeft":{"x":300,"y":40}}
        ],"edges":[],"containers":{"0":[1,2,3]}});
        ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&data).unwrap()).unwrap(),
        )
    }
    struct MoveAt {
        polls: Cell<u64>,
        at: u64,
        point: crate::internal::owner_points::SharedOwnerPoint,
    }
    impl Context for MoveAt {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            if poll == self.at {
                self.point.set(crate::Point { x: 300.0, y: 200.0 });
            }
            None
        }
    }
    #[test]
    fn initial_range_precedes_its_allocation_callback() {
        let mut g = graph();
        g.materialize_owner_node_point(NodeId(0)).unwrap();
        let context = MoveAt {
            polls: Cell::new(0),
            at: 3,
            point: g
                .owner_node_point(NodeId(0))
                .unwrap()
                .shared_point()
                .unwrap(),
        };
        let budget = OvgBuildBudget::with_context(&context);
        let ranges = tunnel_ranges_between(
            &g,
            &[NodeId(0), NodeId(1)],
            NodeId(0),
            NodeId(1),
            true,
            Some(&budget),
        )
        .unwrap();
        assert_eq!(ranges, Some((vec![(0.0, 100.0)], true)));
        assert_eq!(g.position(NodeId(0)).unwrap().y, 200.0);
        assert_eq!(context.polls.get(), 5);
    }
    #[test]
    fn obstacle_range_uses_geometry_after_its_step() {
        let mut g = graph();
        g.materialize_owner_node_point(NodeId(2)).unwrap();
        let context = MoveAt {
            polls: Cell::new(0),
            at: 11,
            point: g
                .owner_node_point(NodeId(2))
                .unwrap()
                .shared_point()
                .unwrap(),
        };
        let budget = OvgBuildBudget::with_context(&context);
        let ranges = tunnel_ranges_between(
            &g,
            &g.graph_node_order(),
            NodeId(0),
            NodeId(1),
            true,
            Some(&budget),
        )
        .unwrap();
        assert_eq!(ranges, Some((vec![(0.0, 100.0)], true)));
        assert_eq!(context.polls.get(), 13);
    }
    #[test]
    fn nan_overlap_preserves_source_rejection_and_minimum_rules() {
        let mut g = graph();
        g.nodes[0].rect.size.height = f64::NAN;
        let ranges = tunnel_ranges_between(
            &g,
            &[NodeId(0), NodeId(1)],
            NodeId(0),
            NodeId(1),
            true,
            None,
        )
        .unwrap()
        .unwrap();
        assert!(ranges.1);
        assert_eq!(ranges.0[0].0, 0.0);
        assert_eq!(ranges.0[0].1.to_bits(), 0x7ff8_0000_0000_0001);
    }

    #[test]
    fn blocked_requires_the_whole_obstacle_between_endpoint_boxes() {
        let left = Rect {
            origin: crate::Point { x: 0.0, y: 0.0 },
            size: crate::Size {
                width: 40.0,
                height: 100.0,
            },
        };
        let right = Rect {
            origin: crate::Point { x: 200.0, y: 0.0 },
            size: left.size,
        };
        let fully_between = Rect {
            origin: crate::Point { x: 80.0, y: 0.0 },
            size: crate::Size {
                width: 40.0,
                height: 100.0,
            },
        };
        let corridor_overlap = Rect {
            origin: crate::Point { x: 20.0, y: 0.0 },
            size: crate::Size {
                width: 80.0,
                height: 100.0,
            },
        };
        assert!(is_blocked(fully_between, left, right, true));
        assert!(!is_blocked(corridor_overlap, left, right, true));
    }

    #[test]
    fn range_subtraction_preserves_the_recovered_four_overlap_cases() {
        fn subtract_range(ranges: &[(f64, f64)], lo: f64, hi: f64) -> Vec<(f64, f64)> {
            let mut next = Vec::new();
            for &range in ranges {
                super::subtract_range(&mut next, range, lo, hi);
            }
            next
        }
        assert_eq!(
            subtract_range(&[(0.0, 100.0)], 40.0, 60.0),
            vec![(0.0, 40.0), (60.0, 100.0),]
        );
        assert_eq!(
            subtract_range(&[(0.0, 100.0)], -10.0, 40.0),
            vec![(40.0, 100.0)]
        );
        assert_eq!(
            subtract_range(&[(0.0, 100.0)], 60.0, 110.0),
            vec![(0.0, 60.0)]
        );
        assert!(subtract_range(&[(0.0, 100.0)], -10.0, 110.0).is_empty());
    }
}
