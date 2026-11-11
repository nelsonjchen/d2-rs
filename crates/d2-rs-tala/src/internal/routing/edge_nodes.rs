// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Incremental edge perimeter and halfway nodes from pinned routing/ovg.go.
use super::channel_solver::{maximum, minimum};
use super::ovg_groups::group;
use super::ovg_intersections::{Budget, PortIndex, check, rect, step};
use super::ovg_ports::PortBuild;
use super::{ArenaGraph, NodeId, Orientation, Point, PortSide};
use crate::internal::owner_groups::OwnerGroupKind::{Cluster, Sequence};
use std::collections::BTreeSet;

// A nil tight bound is returned by Go and dereferenced only when a selected
// halfway direction materializes coordinates. Do not move that panic earlier.
pub(super) fn tight_bounds(
    graph: &ArenaGraph,
    nodes: &[NodeId],
    budget: Budget<'_, '_>,
) -> Result<Option<(Point, Point)>, ()> {
    let Some(&first) = nodes.first() else {
        check(budget)?;
        return Ok(Some((
            Point {
                x: f64::NEG_INFINITY,
                y: f64::NEG_INFINITY,
            },
            Point {
                x: f64::INFINITY,
                y: f64::INFINITY,
            },
        )));
    };
    let Some(origin) = graph.position(first) else {
        check(budget)?;
        return Ok(None);
    };
    let size = graph.nodes[first.0 as usize].rect.size;
    let (mut tl, mut br) = (
        origin,
        Point {
            x: origin.x + size.width,
            y: origin.y + size.height,
        },
    );
    for &node in nodes {
        step(budget)?;
        let Some(origin) = graph.position(node) else {
            return Ok(None);
        };
        let size = graph.nodes[node.0 as usize].rect.size;
        tl.x = minimum(tl.x, origin.x);
        tl.y = minimum(tl.y, origin.y);
        br.x = maximum(br.x, origin.x + size.width);
        br.y = maximum(br.y, origin.y + size.height);
    }
    check(budget)?;
    Ok(Some((tl, br)))
}
fn required(bounds: Option<(Point, Point)>) -> (Point, Point) {
    bounds.expect("runtime error: invalid memory address or nil pointer dereference")
}
impl PortBuild {
    fn perimeter_points(
        &self,
        graph: &ArenaGraph,
        source: NodeId,
        target: NodeId,
        budget: Budget<'_, '_>,
    ) -> Result<Vec<Point>, ()> {
        let a = rect(graph, source);
        let b = rect(graph, target);
        let top = minimum(a.origin.y, b.origin.y) - 100.0;
        let bottom = maximum(a.bottom(), b.bottom()) + 100.0;
        let left = minimum(a.origin.x, b.origin.x) - 100.0;
        let right = maximum(a.right(), b.right()) + 100.0;
        let mut points = Vec::new();
        for owner in [source, target] {
            step(budget)?;
            for side in [
                PortSide::Top,
                PortSide::Bottom,
                PortSide::Left,
                PortSide::Right,
            ] {
                for index in self.ports_by_orientation(owner, side, budget)? {
                    step(budget)?;
                    let p = self.nodes[index].point();
                    points.push(match side {
                        PortSide::Top => Point { x: p.x, y: top },
                        PortSide::Bottom => Point { x: p.x, y: bottom },
                        PortSide::Left => Point { x: left, y: p.y },
                        PortSide::Right => Point { x: right, y: p.y },
                    });
                }
            }
        }
        points.extend([
            Point { x: left, y: top },
            Point { x: right, y: top },
            Point { x: left, y: bottom },
            Point {
                x: right,
                y: bottom,
            },
        ]);
        check(budget)?;
        Ok(points)
    }
    fn halfway_points(
        &self,
        graph: &ArenaGraph,
        source: NodeId,
        target: NodeId,
        budget: Budget<'_, '_>,
    ) -> Result<Vec<Point>, ()> {
        let mut points = Vec::new();
        let orientation = match (graph.position(source), graph.position(target)) {
            (Some(a), Some(b)) => graph.sized_box_orientation(
                (a, graph.nodes[source.0 as usize].rect.size),
                (b, graph.nodes[target.0 as usize].rect.size),
            ),
            _ => Orientation::None,
        };
        if orientation == Orientation::None {
            check(budget)?;
            return Ok(points);
        }
        let (top, right, bottom, left) = match orientation {
            Orientation::Top => (false, false, true, false),
            Orientation::TopLeft => (false, true, true, false),
            Orientation::TopRight => (false, false, true, true),
            Orientation::Bottom => (true, false, false, false),
            Orientation::BottomLeft => (true, true, false, false),
            Orientation::BottomRight => (true, false, false, true),
            Orientation::Left => (false, true, false, false),
            Orientation::Right => (false, false, false, true),
            Orientation::None => unreachable!(),
        };
        let a = rect(graph, source);
        let b = rect(graph, target);
        let mut source_bounds = Some((
            a.origin,
            Point {
                x: a.right().round(),
                y: a.bottom().round(),
            },
        ));
        let mut target_bounds = Some((
            b.origin,
            Point {
                x: b.right().round(),
                y: b.bottom().round(),
            },
        ));
        if let Some(cluster) = group(graph, source, Cluster) {
            source_bounds = tight_bounds(graph, &cluster.members(graph), budget)?;
        }
        if let Some(cluster) = group(graph, target, Cluster) {
            target_bounds = tight_bounds(graph, &cluster.members(graph), budget)?;
        }
        let mut source_ports = self
            .ports
            .get(&source)
            .into_iter()
            .flatten()
            .map(|&(i, _)| i)
            .collect::<Vec<_>>();
        let mut target_ports = self
            .ports
            .get(&target)
            .into_iter()
            .flatten()
            .map(|&(i, _)| i)
            .collect::<Vec<_>>();
        if let Some(sequence) = group(graph, source, Sequence) {
            let members = sequence.members(graph);
            let members = &members;
            source_bounds = tight_bounds(graph, members, budget)?;
            for &node in members {
                step(budget)?;
                if node != source {
                    source_ports
                        .extend(self.ports.get(&node).into_iter().flatten().map(|&(i, _)| i));
                }
            }
        }
        if let Some(sequence) = group(graph, target, Sequence) {
            let members = sequence.members(graph);
            let members = &members;
            target_bounds = tight_bounds(graph, members, budget)?;
            for &node in members {
                step(budget)?;
                if node != target {
                    target_ports
                        .extend(self.ports.get(&node).into_iter().flatten().map(|&(i, _)| i));
                }
            }
        }
        let all_ports = || source_ports.iter().chain(&target_ports);
        for (enabled, horizontal, from_top_left) in [
            (top, true, true),
            (bottom, true, false),
            (left, false, true),
            (right, false, false),
        ] {
            if !enabled {
                continue;
            }
            let (atl, abr) = required(source_bounds);
            let (btl, bbr) = required(target_bounds);
            let coordinate = match (horizontal, from_top_left) {
                (true, true) => (atl.y + bbr.y) / 2.0,
                (true, false) => (btl.y + abr.y) / 2.0,
                (false, true) => (atl.x + bbr.x) / 2.0,
                (false, false) => (btl.x + abr.x) / 2.0,
            };
            for &index in all_ports() {
                step(budget)?;
                let p = self.nodes[index].point();
                points.push(if horizontal {
                    Point {
                        x: p.x,
                        y: coordinate,
                    }
                } else {
                    Point {
                        x: coordinate,
                        y: p.y,
                    }
                });
            }
        }
        let (atl, abr) = required(source_bounds);
        let (btl, bbr) = required(target_bounds);
        if right && bottom {
            points.push(Point {
                x: (btl.x + abr.x) / 2.0,
                y: (btl.y + abr.y) / 2.0,
            });
        }
        if right && top {
            points.push(Point {
                x: (btl.x + abr.x) / 2.0,
                y: (atl.y + bbr.y) / 2.0,
            });
        }
        if left && top {
            points.push(Point {
                x: (atl.x + bbr.x) / 2.0,
                y: (atl.y + bbr.y) / 2.0,
            });
        }
        if left && bottom {
            points.push(Point {
                x: (atl.x + bbr.x) / 2.0,
                y: (btl.y + abr.y) / 2.0,
            });
        }
        check(budget)?;
        Ok(points)
    }
    fn edge_fill_points(
        &self,
        graph: &ArenaGraph,
        source: NodeId,
        target: NodeId,
        budget: Budget<'_, '_>,
    ) -> Result<Vec<Point>, ()> {
        let mut points = self.perimeter_points(graph, source, target, budget)?;
        let halfway = self.halfway_points(graph, source, target, budget)?;
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_EDGE_GROUPS") {
            eprintln!(
                "EDGE_GROUP_RUST from={} to={} perimeter={} halfway={}",
                source.0,
                target.0,
                points.len(),
                halfway.len()
            );
        }
        points.extend(halfway);
        Ok(points)
    }
    pub(super) fn add_edge_nodes(
        &mut self,
        graph: &ArenaGraph,
        graph_nodes: &[NodeId],
        budget: Budget<'_, '_>,
    ) -> Result<(), ()> {
        check(budget)?;
        let fixed = self.fixed_overlaps(graph, graph_nodes, budget)?;
        let index = PortIndex::new(self, graph, graph_nodes, &fixed, budget)?;
        let scope = graph_nodes.iter().copied().collect::<BTreeSet<_>>();
        // Arena Graph.Edges spans split graphs. This projection is the scope's
        // actual edge slice; nearby obstacles do not contribute graph edges.
        for edge_id in &graph.edge_order.read() {
            let edge = &graph.edge_records[edge_id.0 as usize];
            if !scope.contains(&edge.from) || !scope.contains(&edge.to) {
                continue;
            }
            step(budget)?;
            if edge.from == edge.to {
                continue;
            }
            if graph.nodes[edge.from.0 as usize]
                .hierarchy
                .as_ref()
                .zip(graph.nodes[edge.to.0 as usize].hierarchy.as_ref())
                .is_some_and(|(a, b)| a.id == b.id)
            {
                continue;
            }
            for point in self.edge_fill_points(graph, edge.from, edge.to, budget)? {
                step(budget)?;
                step(budget)?;
                let mut near = false;
                for &node in graph_nodes {
                    step(budget)?;
                    if graph.nodes[node.0 as usize].is_container {
                        continue;
                    }
                    let r = rect(graph, node);
                    if r.origin.x - 20.0 <= point.x
                        && point.x <= r.right() + 20.0
                        && r.origin.y - 20.0 <= point.y
                        && point.y <= r.bottom() + 20.0
                    {
                        near = true;
                        break;
                    }
                }
                if near {
                    continue;
                }
                if index.unobstructed(self, graph, point, 1, budget)? {
                    #[cfg(feature = "diagnostic-traces")]
                    let before = self.nodes.len();
                    self.add_point(point, budget)?;
                    #[cfg(feature = "diagnostic-traces")]
                    if before != self.nodes.len()
                        && crate::internal::trace_env_enabled("TALA_TRACE_EDGE_NODE_ACCEPTS")
                    {
                        eprintln!("EDGE_NODE_ACCEPT_RUST {:.17} {:.17}", point.x, point.y);
                    }
                }
            }
        }
        check(budget)
    }
}

// Retained geometry tests use the production point-generation methods. The
// returned list intentionally precedes proximity rejection and point interning.
#[cfg(test)]
pub(super) fn edge_nodes(
    graph: &ArenaGraph,
    scope_nodes: &[NodeId],
    boxes: &std::collections::BTreeMap<NodeId, super::Rect>,
    ports: &std::collections::BTreeMap<NodeId, Vec<super::Port>>,
) -> Vec<Point> {
    let mut build = PortBuild::default();
    let ports = ports
        .iter()
        .flat_map(|(&node, ports)| ports.iter().map(move |&port| (node, port)))
        .collect::<Vec<_>>();
    build
        .import_ports(&ports, &super::route_search_guard::OvgBuildBudget::new())
        .unwrap();
    let scope = scope_nodes.iter().copied().collect::<BTreeSet<_>>();
    let mut points = Vec::new();
    for edge_id in &graph.edge_order.read() {
        let edge = &graph.edge_records[edge_id.0 as usize];
        if edge.from == edge.to
            || !scope.contains(&edge.from)
            || !scope.contains(&edge.to)
            || !boxes.contains_key(&edge.from)
            || !boxes.contains_key(&edge.to)
        {
            continue;
        }
        if graph.nodes[edge.from.0 as usize]
            .hierarchy
            .as_ref()
            .zip(graph.nodes[edge.to.0 as usize].hierarchy.as_ref())
            .is_some_and(|(a, b)| a.id == b.id)
        {
            continue;
        }
        points.extend(
            build
                .edge_fill_points(
                    graph,
                    edge.from,
                    edge.to,
                    &super::route_search_guard::OvgBuildBudget::new(),
                )
                .unwrap(),
        );
    }
    points
}

#[cfg(test)]
mod tests {
    use super::super::route_search_guard::OvgBuildBudget;
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::Cell;
    struct Caller {
        polls: Cell<u64>,
        mutate: Option<crate::internal::owner_points::SharedOwnerPoint>,
    }
    impl Context for Caller {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            self.polls.set(self.polls.get() + 1);
            if self.polls.get() == 1
                && let Some(point) = &self.mutate
            {
                point.set(Point {
                    x: 9876.5,
                    y: -1234.5,
                });
            }
            None
        }
    }
    fn graph(size: f64, target: Point) -> ArenaGraph {
        let value = serde_json::json!({"nodes":[
            {"id":1,"width":size,"height":size,"topLeft":{"x":0,"y":0}},
            {"id":2,"width":size,"height":size,"topLeft":{"x":target.x,"y":target.y}}
        ],"edges":[{"id":11,"from":1,"to":2,"points":[]}],"containers":{"0":[1,2]}});
        ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&value).unwrap()).unwrap(),
        )
    }
    #[test]
    fn edge_fill_reserves_real_nodes_and_reuses_the_intersection_overlap_cache() {
        // Pinned addPorts/addNodesIntersections/addEdgesNodes observer values.
        for (size, target, work, nodes, candidates, polls) in [
            (0.0, Point { x: 0.0, y: 0.0 }, 1200, 5, 1, 781),
            (40.0, Point { x: 200.0, y: 120.0 }, 3490, 83, 100, 3781),
            (-40.0, Point { x: 200.0, y: 120.0 }, 3450, 83, 100, 3741),
        ] {
            let graph = graph(size, target);
            let caller = Caller {
                polls: Cell::new(0),
                mutate: None,
            };
            let budget = OvgBuildBudget::with_context(&caller);
            budget.check().unwrap();
            let mut build = PortBuild::default();
            let scope = graph.graph_node_order();
            build.add_ports(&graph, &scope, &budget).unwrap();
            build
                .add_nodes_intersections(&graph, &scope, &budget)
                .unwrap();
            assert_eq!(build.fixed_cache.len(), 1);
            build.add_edge_nodes(&graph, &scope, &budget).unwrap();
            assert_eq!(build.fixed_cache.len(), 1);
            assert_eq!(build.nodes.len(), nodes as usize);
            assert_eq!(budget.observed_resources(), [work, nodes, 0, candidates]);
            assert_eq!(caller.polls.get(), polls);
        }
    }
    #[test]
    fn coincident_port_is_emitted_once_for_each_of_its_four_directions() {
        let graph = graph(0.0, Point { x: 0.0, y: 0.0 });
        let mut build = PortBuild::default();
        build
            .add_ports(
                &graph,
                &graph.graph_node_order(),
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        for side in [
            PortSide::Top,
            PortSide::Bottom,
            PortSide::Left,
            PortSide::Right,
        ] {
            assert_eq!(
                build
                    .ports_by_orientation(
                        NodeId(0),
                        side,
                        &super::super::route_search_guard::OvgBuildBudget::new()
                    )
                    .unwrap(),
                [0]
            );
        }
        let points = build
            .perimeter_points(
                &graph,
                NodeId(0),
                NodeId(1),
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        assert_eq!(points.len(), 12);
        assert_eq!(
            &points[..4],
            &[
                Point { x: 0.0, y: -100.0 },
                Point { x: 0.0, y: 100.0 },
                Point { x: -100.0, y: 0.0 },
                Point { x: 100.0, y: 0.0 }
            ]
        );
    }
    #[test]
    fn perimeter_reads_before_its_first_poll_and_halfway_reads_afterward() {
        let mut graph = graph(40.0, Point { x: 200.0, y: 120.0 });
        graph.materialize_owner_node_point(NodeId(0)).unwrap();
        let point = graph
            .owner_node_point(NodeId(0))
            .unwrap()
            .shared_point()
            .unwrap();
        let mut build = PortBuild::default();
        build
            .add_ports(
                &graph,
                &graph.graph_node_order(),
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        let caller = Caller {
            polls: Cell::new(0),
            mutate: Some(point),
        };
        let budget = OvgBuildBudget::with_context(&caller);
        let perimeter = build
            .perimeter_points(&graph, NodeId(0), NodeId(1), &budget)
            .unwrap();
        assert_eq!(perimeter[0], Point { x: 10.0, y: -100.0 });
        let halfway = build
            .halfway_points(&graph, NodeId(0), NodeId(1), &budget)
            .unwrap();
        // Go rounds the newly read bottom-right (-1194.5) away from zero.
        assert_eq!(halfway[0], Point { x: 10.0, y: -537.5 });
    }
    #[test]
    fn nil_member_tight_bounds_return_before_the_final_context_check() {
        let mut graph = graph(40.0, Point { x: 200.0, y: 120.0 });
        graph.clear_position(NodeId(1));
        let caller = Caller {
            polls: Cell::new(0),
            mutate: None,
        };
        let budget = OvgBuildBudget::with_context(&caller);
        assert_eq!(
            tight_bounds(&graph, &[NodeId(0), NodeId(1)], &budget).unwrap(),
            None
        );
        assert_eq!(caller.polls.get(), 2);
        assert_eq!(budget.observed_resources(), [2, 0, 0, 0]);
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn decoded_sequence_pointers_supply_bounds_and_unblocked_member_ports() {
        let fixtures: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/live-graphjson-source-v090.json"
        ))
        .unwrap();
        let value = fixtures["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"].as_str().unwrap().contains("dejitter_sequence/"))
            .unwrap()["source"]
            .clone();
        let graph = ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&value).unwrap()).unwrap(),
        );
        let scope = graph.graph_node_order();
        assert!(
            scope
                .iter()
                .any(|&node| graph.nodes[node.0 as usize].sequence.is_none()
                    && group(&graph, node, Sequence).is_some())
        );
        let mut build = PortBuild::default();
        build
            .add_ports(
                &graph,
                &scope,
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        build
            .add_nodes_intersections(
                &graph,
                &scope,
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        build
            .add_edge_nodes(
                &graph,
                &scope,
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        assert_eq!(build.nodes.len(), 66); // Unmodified Go stage observer.
    }
    #[test]
    fn zero_width_step_percentages_use_the_source_integer_conversion() {
        let mut graph = graph(0.0, Point { x: 0.0, y: 0.0 });
        for node in &mut graph.nodes {
            node.shape = crate::ShapeKind::Step;
        }
        let mut build = PortBuild::default();
        build
            .add_ports(
                &graph,
                &graph.graph_node_order(),
                &super::super::route_search_guard::OvgBuildBudget::new(),
            )
            .unwrap();
        assert_eq!(build.nodes.len(), 1);
        assert_eq!(build.nodes[0].point().x.to_bits(), 0);
        assert_eq!(build.nodes[0].point().y.to_bits(), 0);
        for owner in [NodeId(0), NodeId(1)] {
            assert_eq!(build.ports[&owner].len(), 10);
            assert!(build.ports[&owner].iter().all(|&(index, _)| index == 0));
        }
    }
}
