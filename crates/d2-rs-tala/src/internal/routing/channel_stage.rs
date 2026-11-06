// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Connected channel pass over an explicit shared-point graph.
//! The ArenaGraph bridge, full quality inspector and exact route-stage work
//! accounting are separate qualification requirements.
use super::{
    channel_candidates as candidates, channel_geometry as geometry,
    channel_points::{self as points, PointId, Proposal},
    channel_safety as safety,
};
use crate::Point;

#[derive(Clone)]
pub(super) struct Node {
    pub origin: PointId,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone)]
pub(super) struct Edge {
    pub from: usize,
    pub to: usize,
    pub points: Vec<PointId>,
    pub is_curve: bool,
    /// Upstream tree/loop/special-edge classification, supplied by the adapter.
    pub locked: bool,
    /// Non-endpoint ancestor nodes only, supplied by the adapter.
    pub allowed_ancestors: Vec<usize>,
}
#[derive(Clone)]
pub(super) struct Graph {
    pub points: Vec<Point>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}
#[derive(Clone, Copy, Default)]
pub(super) struct Quality {
    pub obstructions: usize,
    pub crossings: f64,
    pub text_occlusions: usize,
    pub length: f64,
}
pub(super) trait Guard {
    type Error;
    fn reserve(&mut self, units: u64) -> Result<(), Self::Error>;
    fn quality(&mut self, graph: &Graph) -> Result<Quality, Self::Error>;
}

struct Evaluation<'a, G> {
    graph: &'a Graph,
    groups: Vec<Vec<[PointId; 2]>>,
    horizontal: bool,
    guard: &'a mut G,
}
struct SafetyGuard<'a, G> {
    graph: &'a Graph,
    guard: &'a mut G,
}
impl<G: Guard> safety::Guard for SafetyGuard<'_, G> {
    type Error = G::Error;
    fn step(&mut self) -> Result<(), Self::Error> {
        self.guard.reserve(1)
    }
    fn route_clear(
        &mut self,
        index: usize,
        before: &[Point],
        after: &[Point],
    ) -> Result<bool, Self::Error> {
        let obstacles: Vec<_> = self
            .graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| safety::Obstacle {
                top_left: Some(self.graph.points[n.origin.0]),
                width: n.width,
                height: n.height,
                allowed_ancestor: self.graph.edges[index].allowed_ancestors.contains(&i),
            })
            .collect();
        safety::route_clear(before, after, &obstacles, |u| self.guard.reserve(u))
    }
}
impl<G: Guard> candidates::Evaluator for Evaluation<'_, G> {
    type Error = G::Error;
    type Proposal = Proposal;
    fn reserve(&mut self, u: u64) -> Result<(), Self::Error> {
        self.guard.reserve(u)
    }
    fn proposal(&mut self, p: &[f64]) -> Result<Option<Proposal>, Self::Error> {
        points::propose(&self.graph.points, &self.groups, p, self.horizontal, |u| {
            self.guard.reserve(u)
        })
    }
    fn wire_length(&mut self, p: Option<&Proposal>) -> Result<f64, Self::Error> {
        let edges: Vec<_> = self.graph.edges.iter().map(|e| e.points.clone()).collect();
        points::wire_length(&self.graph.points, &edges, p, |u| self.guard.reserve(u))
    }
    fn safe(&mut self, p: &Proposal) -> Result<bool, Self::Error> {
        let before: Vec<Vec<_>> = self
            .graph
            .edges
            .iter()
            .map(|e| e.points.iter().map(|id| self.graph.points[id.0]).collect())
            .collect();
        let after: Vec<Vec<_>> = self
            .graph
            .edges
            .iter()
            .map(|e| {
                e.points
                    .iter()
                    .map(|&id| p.point(id, &self.graph.points))
                    .collect()
            })
            .collect();
        let edges: Vec<_> = self
            .graph
            .edges
            .iter()
            .enumerate()
            .map(|(i, e)| safety::Edge {
                before: &before[i],
                after: &after[i],
                is_curve: e.is_curve,
            })
            .collect();
        safety::check(
            &edges,
            &mut SafetyGuard {
                graph: self.graph,
                guard: self.guard,
            },
        )
    }
}

fn inventory<G: Guard>(
    graph: &Graph,
    horizontal: bool,
    guard: &mut G,
) -> Result<
    (
        Vec<geometry::Segment>,
        Vec<[PointId; 2]>,
        Vec<geometry::Segment>,
    ),
    G::Error,
> {
    let mut fixed = vec![false; graph.points.len()];
    for node in &graph.nodes {
        fixed[node.origin.0] = true;
    }
    for edge in &graph.edges {
        if edge.points.len() >= 2 {
            fixed[edge.points[0].0] = true;
            fixed[edge.points.last().unwrap().0] = true;
        }
    }
    let mut segments = Vec::new();
    let mut identities = Vec::new();
    for (owner, edge) in graph.edges.iter().enumerate() {
        guard.reserve(1)?;
        let malformed = edge.points.windows(2).any(|p| {
            let a = graph.points[p[0].0];
            let b = graph.points[p[1].0];
            a == b || (a.x != b.x && a.y != b.y)
        });
        for pair in edge.points.windows(2) {
            guard.reserve(1)?;
            let a = graph.points[pair[0].0];
            let b = graph.points[pair[1].0];
            if (horizontal && a.x != b.x) || (!horizontal && a.y != b.y) {
                continue;
            }
            let ids = if (horizontal && a.y < b.y) || (!horizontal && a.x < b.x) {
                [pair[0], pair[1]]
            } else {
                [pair[1], pair[0]]
            };
            let start = graph.points[ids[0].0];
            let end = graph.points[ids[1].0];
            let mut locked =
                edge.locked || edge.is_curve || malformed || fixed[ids[0].0] || fixed[ids[1].0];
            for node in &graph.nodes {
                guard.reserve(1)?;
                if node.width == 1.0 && node.height == 1.0 {
                    let origin = graph.points[node.origin.0];
                    for p in [start, end] {
                        if (p.x == origin.x || p.x == origin.x + 1.0)
                            && (p.y == origin.y || p.y == origin.y + 1.0)
                        {
                            locked = true;
                        }
                    }
                }
            }
            identities.push(ids);
            segments.push(geometry::Segment {
                start,
                end,
                owner,
                fixed: locked,
            });
        }
    }
    let mut walls = Vec::new();
    for (owner, node) in graph.nodes.iter().enumerate() {
        guard.reserve(1)?;
        let p = graph.points[node.origin.0];
        let pairs = if horizontal {
            [
                (
                    p,
                    Point {
                        x: p.x,
                        y: p.y + node.height,
                    },
                ),
                (
                    Point {
                        x: p.x + node.width,
                        y: p.y,
                    },
                    Point {
                        x: p.x + node.width,
                        y: p.y + node.height,
                    },
                ),
            ]
        } else {
            [
                (
                    p,
                    Point {
                        x: p.x + node.width,
                        y: p.y,
                    },
                ),
                (
                    Point {
                        x: p.x,
                        y: p.y + node.height,
                    },
                    Point {
                        x: p.x + node.width,
                        y: p.y + node.height,
                    },
                ),
            ]
        };
        for (start, end) in pairs {
            walls.push(geometry::Segment {
                start,
                end,
                owner,
                fixed: true,
            });
        }
    }
    Ok((segments, identities, walls))
}

/// Complete pass on a validated, closed inventory. Restores all point values
/// and route inventories on error. Admission/validation and external alias
/// closure must be checked by the caller before invoking this internal stage.
pub(super) fn run<G: Guard>(graph: &mut Graph, guard: &mut G) -> Result<(), G::Error> {
    let snapshot = graph.clone();
    let result = run_inner(graph, guard);
    if result.is_err() {
        *graph = snapshot;
    }
    result
}
fn run_inner<G: Guard>(graph: &mut Graph, guard: &mut G) -> Result<(), G::Error> {
    for horizontal in [true, false] {
        let (segments, ids, walls) = inventory(graph, horizontal, guard)?;
        let channels = geometry::build(&segments, &walls, horizontal, |u| guard.reserve(u))?;
        if channels.problem.groups.is_empty() {
            continue;
        }
        let snapshot = graph.clone();
        let before = guard.quality(graph)?;
        let mut changed = false;
        for component in candidates::components(&channels.problem) {
            let groups = component
                .indices
                .iter()
                .map(|&i| channels.segments[i].iter().map(|&s| ids[s]).collect())
                .collect();
            let positions: Vec<_> = component
                .indices
                .iter()
                .map(|&i| channels.positions[i])
                .collect();
            let mut evaluation = Evaluation {
                graph,
                groups,
                horizontal,
                guard,
            };
            if let Some(proposal) =
                candidates::select(&component.problem, &positions, &mut evaluation)?
            {
                // Explicit slot order is deterministic but not upstream Go map
                // order. Exact commit-trace/work-limit qualification is pending.
                for (id, value) in proposal.updates() {
                    guard.reserve(1)?;
                    graph.points[id.0] = value;
                }
                changed = true;
            }
        }
        if !changed {
            continue;
        }
        let after = guard.quality(graph)?;
        if after.obstructions > before.obstructions
            || after.crossings > before.crossings
            || after.text_occlusions > before.text_occlusions
            || after.length > before.length + 1e-6
        {
            *graph = snapshot;
            continue;
        }
        for (i, edge) in graph.edges.iter_mut().enumerate() {
            if !edge
                .points
                .iter()
                .zip(&snapshot.edges[i].points)
                .any(|(new, old)| graph.points[new.0] != snapshot.points[old.0])
            {
                continue;
            }
            if edge.points.len() <= 1 {
                guard.reserve(1)?;
                continue;
            }
            let mut retained = vec![edge.points[0]];
            for pair in edge.points.windows(2) {
                guard.reserve(1)?;
                if graph.points[pair[0].0] != graph.points[pair[1].0] {
                    retained.push(pair[1]);
                }
            }
            if retained.len() == 1 {
                let mut p = graph.points[retained[0].0];
                p.x += 1.0;
                retained.push(PointId(graph.points.len()));
                graph.points.push(p);
            }
            edge.points = retained;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestGuard {
        remaining: usize,
        reject_quality: bool,
        inspections: usize,
    }
    impl Guard for TestGuard {
        type Error = &'static str;
        fn reserve(&mut self, u: u64) -> Result<(), Self::Error> {
            if u as usize > self.remaining {
                Err("work limit")
            } else {
                self.remaining -= u as usize;
                Ok(())
            }
        }
        fn quality(&mut self, g: &Graph) -> Result<Quality, Self::Error> {
            self.inspections += 1;
            // These fixtures have visible rectangular nodes and no labels.
            // Production classification/label positioning belongs to the bridge.
            let nodes: Vec<_> = g
                .nodes
                .iter()
                .map(|n| crate::internal::quality::Node {
                    rect: crate::Rect {
                        origin: g.points[n.origin.0],
                        size: crate::Size {
                            width: n.width,
                            height: n.height,
                        },
                    },
                    parent: None,
                    invisible: false,
                    rectangular: true,
                    container: false,
                })
                .collect();
            let edges: Vec<_> = g
                .edges
                .iter()
                .map(|e| crate::internal::quality::Edge {
                    from: e.from,
                    to: e.to,
                    points: e.points.iter().map(|id| g.points[id.0]).collect(),
                    invisible: false,
                    curve: e.is_curve,
                })
                .collect();
            let metrics =
                crate::internal::quality::inspect(&nodes, &edges, &[], |u| self.reserve(u))?;
            Ok(Quality {
                length: metrics.route_length,
                obstructions: metrics.route_obstructions as usize,
                crossings: metrics.crossings,
                text_occlusions: metrics.text_occlusions as usize
                    + usize::from(self.reject_quality && self.inspections.is_multiple_of(2)),
            })
        }
    }
    fn z_graph() -> Graph {
        Graph {
            points: vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 300.0, y: 200.0 },
                Point { x: 100.0, y: 50.0 },
                Point { x: 120.0, y: 50.0 },
                Point { x: 120.0, y: 250.0 },
                Point { x: 300.0, y: 250.0 },
            ],
            nodes: vec![
                Node {
                    origin: PointId(0),
                    width: 100.0,
                    height: 100.0,
                },
                Node {
                    origin: PointId(1),
                    width: 100.0,
                    height: 100.0,
                },
            ],
            edges: vec![Edge {
                from: 0,
                to: 1,
                points: (2..6).map(PointId).collect(),
                is_curve: false,
                locked: false,
                allowed_ancestors: vec![],
            }],
        }
    }
    fn guard() -> TestGuard {
        TestGuard {
            remaining: 100000,
            reject_quality: false,
            inspections: 0,
        }
    }
    #[test]
    fn upstream_z_corridor_centers_and_is_idempotent() {
        let mut graph = z_graph();
        let initial = graph.points.clone();
        run(&mut graph, &mut guard()).unwrap();
        assert!((graph.points[3].x - 200.0).abs() < 1e-6);
        assert!((graph.points[4].x - 200.0).abs() < 1e-6);
        for i in [0, 1, 2, 5] {
            assert_eq!(graph.points[i], initial[i]);
        }
        let first = graph.points.clone();
        run(&mut graph, &mut guard()).unwrap();
        assert_eq!(graph.points, first);
    }
    #[test]
    fn quality_failure_restores_points_and_route_ids() {
        let mut graph = z_graph();
        let initial = graph.clone();
        let mut g = guard();
        g.reject_quality = true;
        run(&mut graph, &mut g).unwrap();
        assert_eq!(graph.points, initial.points);
        assert_eq!(graph.edges[0].points, initial.edges[0].points);
    }
    #[test]
    fn work_failure_at_every_reservation_restores_snapshot() {
        let initial = z_graph();
        let mut measured = initial.clone();
        let mut g = guard();
        run(&mut measured, &mut g).unwrap();
        let work = 100000 - g.remaining;
        for budget in 0..work {
            let mut graph = initial.clone();
            let mut g = guard();
            g.remaining = budget;
            assert_eq!(run(&mut graph, &mut g), Err("work limit"));
            assert_eq!(graph.points, initial.points, "budget {budget}");
            assert_eq!(graph.edges[0].points, initial.edges[0].points);
        }
    }
    #[test]
    fn upstream_shared_trunks_and_locked_loop_are_preserved() {
        let mut graph = z_graph();
        let mut duplicate = graph.edges[0].clone();
        duplicate.points = duplicate
            .points
            .iter()
            .map(|id| {
                let value = graph.points[id.0];
                let new = PointId(graph.points.len());
                graph.points.push(value);
                new
            })
            .collect();
        graph.edges.push(duplicate);
        let point = PointId(graph.points.len());
        graph.points.push(Point { x: 50.0, y: 50.0 });
        graph.edges.push(Edge {
            from: 0,
            to: 0,
            points: vec![point, point],
            is_curve: false,
            locked: true,
            allowed_ancestors: vec![],
        });
        run(&mut graph, &mut guard()).unwrap();
        for (a, b) in graph.edges[0].points.iter().zip(&graph.edges[1].points) {
            assert_eq!(graph.points[a.0], graph.points[b.0]);
        }
        assert!((graph.points[3].x - 200.0).abs() < 1e-6);
        assert_eq!(graph.edges[2].points, vec![point, point]);
        assert_eq!(graph.points[point.0], Point { x: 50.0, y: 50.0 });
    }
    #[test]
    fn upstream_unbounded_outer_route_stays_fixed() {
        let mut graph = z_graph();
        graph.points[1].x = 0.0;
        graph.points[3].x = 160.0;
        graph.points[4].x = 160.0;
        graph.points[5].x = 100.0;
        let initial = graph.points.clone();
        run(&mut graph, &mut guard()).unwrap();
        assert_eq!(graph.points, initial);
    }
}
