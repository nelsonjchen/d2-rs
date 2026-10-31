// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Positioned drawing inspection, translated from D2 v0.9.0 quality/inspection*.
//! Input must be validated and labels positioned by the upstream-shaped adapter.
use super::routing::{
    channel_points::go_hypot,
    channel_safety::crossing,
    channel_solver::{maximum, minimum},
};
use crate::{Point, Rect};

mod owner_inspect;
pub(super) use owner_inspect::{inspect_owner, owner_ancestry_parent};

pub(super) struct Node {
    pub rect: Rect,
    pub parent: Option<usize>,
    pub invisible: bool,
    pub rectangular: bool,
    pub container: bool,
}
pub(super) struct Edge {
    pub from: usize,
    pub to: usize,
    pub points: Vec<Point>,
    pub invisible: bool,
    pub curve: bool,
}
pub(super) enum Owner {
    Node(usize),
    Edge(usize),
}
pub(super) struct Label {
    pub rect: Rect,
    pub owner: Owner,
}
#[derive(Default, Debug, PartialEq)]
pub(super) struct Metrics {
    pub node_overlaps: u64,
    pub route_obstructions: u64,
    pub text_occlusions: u64,
    pub crossings: f64,
    pub detour: f64,
    pub route_length: f64,
}

fn overlaps(a: Rect, b: Rect) -> bool {
    if a.origin.x >= b.right() || b.origin.x >= a.right() {
        return false;
    }
    a.origin.y < b.bottom() && b.origin.y < a.bottom()
}
fn covers(a: Rect, b: Rect) -> bool {
    b.origin.x >= a.origin.x
        && b.origin.y >= a.origin.y
        && b.right() <= a.right()
        && b.bottom() <= a.bottom()
}
fn enters(rect: Rect, a: Point, b: Point) -> bool {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 || a == b {
        return false;
    }
    let mut low = 0.0;
    let mut high = 1.0;
    for (start, delta, min, max) in [
        (a.x, b.x - a.x, rect.origin.x, rect.right()),
        (a.y, b.y - a.y, rect.origin.y, rect.bottom()),
    ] {
        if delta == 0.0 {
            if start <= min || start >= max {
                return false;
            }
            continue;
        }
        let mut first = (min - start) / delta;
        let mut last = (max - start) / delta;
        if first > last {
            std::mem::swap(&mut first, &mut last);
        }
        low = maximum(low, first);
        high = minimum(high, last);
        if low >= high {
            return false;
        }
    }
    low < high
}
fn descendant<E>(
    nodes: &[Node],
    mut node: usize,
    ancestor: usize,
    work: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<bool, E> {
    // Topology depth and cycle validation belongs to the validated input boundary.
    loop {
        work(1)?;
        if node == ancestor {
            return Ok(true);
        }
        let Some(parent) = nodes[node].parent else {
            return Ok(false);
        };
        node = parent;
    }
}
fn endpoint_ancestor<E>(
    nodes: &[Node],
    edge: &Edge,
    node: usize,
    work: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<bool, E> {
    if descendant(nodes, edge.from, node, work)? {
        return Ok(true);
    }
    descendant(nodes, edge.to, node, work)
}

/// Metric computation after graph validation and label-box positioning.
/// Label inventory acquisition charges, guard finish, finite-result validation
/// and public error formatting are not represented by this internal boundary.
pub(super) fn inspect<E>(
    nodes: &[Node],
    edges: &[Edge],
    labels: &[Label],
    mut work: impl FnMut(u64) -> Result<(), E>,
) -> Result<Metrics, E> {
    let mut score = Metrics::default();
    let mut scale = 0.0;
    let mut count = 0;
    for node in nodes {
        work(1)?;
        if !node.container && !node.invisible {
            scale += (node.rect.size.width + node.rect.size.height) / 2.0;
            count += 1;
        }
    }
    scale = maximum(1.0, scale / (count.max(1) as f64));
    let mut visible = Vec::new();
    for (index, edge) in edges.iter().enumerate() {
        work(1)?;
        if edge.invisible {
            continue;
        }
        visible.push(index);
        let mut length = 0.0;
        for pair in edge.points.windows(2) {
            work(1)?;
            length += go_hypot(pair[1].x - pair[0].x, pair[1].y - pair[0].y);
        }
        score.route_length += length / scale;
        if edge.points.len() < 2 || edge.from == edge.to {
            continue;
        }
        let first = edge.points[0];
        let last = *edge.points.last().unwrap();
        let direct = (last.x - first.x).abs() + (last.y - first.y).abs();
        score.detour += maximum(0.0, length - direct) / maximum(scale, direct);
    }
    for (i, &index) in visible.iter().enumerate() {
        work(1)?;
        for &other in &visible[i + 1..] {
            work(1)?;
            for a in edges[index].points.windows(2) {
                work(1)?;
                for b in edges[other].points.windows(2) {
                    work(1)?;
                    score.crossings += f64::from(crossing(a[0], a[1], b[0], b[1]));
                }
            }
        }
    }
    for (i, node) in nodes.iter().enumerate() {
        work(1)?;
        if node.invisible || !node.rectangular {
            continue;
        }
        for (j, other) in nodes.iter().enumerate().skip(i + 1) {
            work(1)?;
            if other.invisible || !other.rectangular || !overlaps(node.rect, other.rect) {
                continue;
            }
            if !descendant(nodes, i, j, &mut work)? && !descendant(nodes, j, i, &mut work)? {
                score.node_overlaps += 1;
            }
        }
        for edge in edges {
            work(1)?;
            if edge.invisible || edge.curve {
                continue;
            }
            if endpoint_ancestor(nodes, edge, i, &mut work)? && edge.from != i && edge.to != i {
                continue;
            }
            for (segment, pair) in edge.points.windows(2).enumerate() {
                work(1)?;
                if (segment == 0 && edge.from == i)
                    || (segment + 2 == edge.points.len() && edge.to == i)
                {
                    continue;
                }
                if enters(node.rect, pair[0], pair[1]) {
                    score.route_obstructions += 1;
                    break;
                }
            }
        }
    }
    // Label inventory is already positioned and filtered; preserve its order.
    for (i, placed) in labels.iter().enumerate() {
        work(1)?;
        for other in &labels[..i] {
            work(1)?;
            if overlaps(placed.rect, other.rect) {
                score.text_occlusions += 1;
            }
        }
        for (j, node) in nodes.iter().enumerate() {
            work(1)?;
            if node.invisible || !node.rectangular || !overlaps(placed.rect, node.rect) {
                continue;
            }
            let ancestor = match placed.owner {
                Owner::Node(n) => descendant(nodes, n, j, &mut work)?,
                Owner::Edge(e) => {
                    endpoint_ancestor(nodes, &edges[e], j, &mut work)?
                        && edges[e].from != j
                        && edges[e].to != j
                }
            };
            if ancestor && covers(node.rect, placed.rect) {
                continue;
            }
            score.text_occlusions += 1;
        }
        for &e in &visible {
            work(1)?;
            if matches!(placed.owner,Owner::Edge(owner) if owner==e) || edges[e].curve {
                continue;
            }
            for pair in edges[e].points.windows(2) {
                work(1)?;
                if enters(placed.rect, pair[0], pair[1]) {
                    score.text_occlusions += 1;
                    break;
                }
            }
        }
    }
    Ok(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Size;
    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect {
            origin: Point { x, y },
            size: Size {
                width: w,
                height: h,
            },
        }
    }
    fn node(x: f64, y: f64) -> Node {
        Node {
            rect: rect(x, y, 20.0, 20.0),
            parent: None,
            invisible: false,
            rectangular: true,
            container: false,
        }
    }
    fn allow(_: u64) -> Result<(), ()> {
        Ok(())
    }
    #[test]
    fn upstream_shaped_repair_trigger_counts_obstruction_once_per_node() {
        let nodes = vec![node(0.0, 0.0), node(100.0, 0.0), node(50.0, 30.0)];
        let edges: Vec<_> = (0..2)
            .map(|_| Edge {
                from: 0,
                to: 1,
                points: vec![
                    Point { x: 10.0, y: 20.0 },
                    Point { x: 10.0, y: 40.0 },
                    Point { x: 110.0, y: 40.0 },
                    Point { x: 110.0, y: 20.0 },
                ],
                invisible: false,
                curve: false,
            })
            .collect();
        let labels = vec![
            Label {
                rect: rect(40.0, 35.0, 40.0, 10.0),
                owner: Owner::Edge(0),
            },
            Label {
                rect: rect(40.0, 35.0, 40.0, 10.0),
                owner: Owner::Edge(1),
            },
        ];
        let score = inspect(&nodes, &edges, &labels, allow).unwrap();
        assert_eq!(score.route_obstructions, 2);
        assert_eq!(score.route_length, 14.0);
        assert_eq!(score.detour, 0.8);
        assert_eq!(score.crossings, 0.0);
        assert_eq!(score.text_occlusions, 5);
    }
    #[test]
    fn open_rectangle_excludes_borders_but_detects_diagonal_interior() {
        let r = rect(0.0, 0.0, 20.0, 20.0);
        assert!(!enters(
            r,
            Point { x: 0.0, y: 0.0 },
            Point { x: 20.0, y: 0.0 }
        ));
        assert!(enters(
            r,
            Point { x: -1.0, y: -1.0 },
            Point { x: 21.0, y: 21.0 }
        ));
        assert!(!enters(
            r,
            Point { x: -1.0, y: 1.0 },
            Point { x: 1.0, y: -1.0 }
        ));
    }
    #[test]
    fn own_node_label_and_contained_descendant_do_not_occlude() {
        let mut nodes = vec![node(0.0, 0.0), node(5.0, 5.0)];
        nodes[1].rect = rect(5.0, 5.0, 5.0, 5.0);
        nodes[1].parent = Some(0);
        let labels = [Label {
            rect: rect(6.0, 6.0, 2.0, 2.0),
            owner: Owner::Node(1),
        }];
        let m = inspect(&nodes, &[], &labels, allow).unwrap();
        assert_eq!(m.node_overlaps, 0);
        assert_eq!(m.text_occlusions, 0);
        assert!(inspect(&nodes, &[], &labels, |_| Err::<(), _>("cancelled")).is_err());
    }
}
