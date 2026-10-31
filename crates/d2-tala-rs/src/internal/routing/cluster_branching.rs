// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pinned postprocess.go462-698 over actual graph, incident and point storage.
use super::nudge::{edges, nodes};
use super::route_stage_guard::{self, RouteAtomicGuard, RouteStageError, RuntimeRouteContext};
use super::*;
use crate::EdgeId;
#[cfg(test)]
use crate::internal::ClusterArrangement;
use crate::internal::owner_groups::{OwnerGroupId, OwnerGroupKind};
use crate::internal::owner_points::SharedOwnerPoint;
use crate::internal::{OwnerGraphId, limits};
type Result<T> = std::result::Result<T, RouteStageError>;

// cluster_route_guard.go: every cost is evaluated against the current live
// routes, including earlier candidates' slot writes into shared backings.
fn estimate_cost(g: &ArenaGraph, edge: EdgeId, guard: &RouteAtomicGuard<'_>) -> Result<f64> {
    let route = &g.edge_records[edge.0 as usize].route;
    let mut length = 0.0;
    for i in 0..route.len().saturating_sub(1) {
        guard.step()?;
        length += euclidean_distance(route.point(i), route.point(i + 1));
    }
    // This stage scores only four-point routes. Go math.Pow(turnPenalty, 2)
    // multiplies the two rounded float64 operands, unlike a constant expression.
    debug_assert_eq!(route.len(), 4);
    let penalty: f64 = 1.4;
    let mut cost = length * (penalty * penalty);
    let mut crossings = 0_u64;
    for other in edges(g) {
        guard.step()?;
        if edge == other {
            continue;
        }
        let other = &g.edge_records[other.0 as usize].route;
        for i in 0..route.len().saturating_sub(1) {
            for j in 0..other.len().saturating_sub(1) {
                guard.step()?;
                let (a, b, c, d) = (
                    route.point(i),
                    route.point(i + 1),
                    other.point(j),
                    other.point(j + 1),
                );
                if super::straight::non_parallel_intersection(a, b, c, d)
                    && !(a.x == b.x && (a.x == c.x || a.x == d.x))
                    && !(a.y == b.y && (a.y == c.y || a.y == d.y))
                {
                    crossings += 1;
                }
            }
        }
    }
    // Go's untyped constant is rounded once, after constant multiplication.
    cost += 0.110592 * crossings as f64;
    Ok(cost)
}
fn intersects(g: &ArenaGraph, edge: EdgeId, guard: &RouteAtomicGuard<'_>) -> Result<bool> {
    let e = &g.edge_records[edge.0 as usize];
    for node in nodes(g) {
        guard.step()?;
        if super::shortcut::descendant(g, Some(e.from), node)
            || super::shortcut::descendant(g, Some(e.to), node)
        {
            continue;
        }
        for i in 0..e.route.len().saturating_sub(1) {
            guard.step()?;
            if let Some(origin) = g.position(node) {
                let rect = crate::Rect {
                    origin,
                    size: g.nodes[node.0 as usize].rect.size,
                };
                if super::route_geometry::recovered_segment_intersects_box(
                    e.route.point(i),
                    e.route.point(i + 1),
                    rect,
                ) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
fn copy_slot(g: &ArenaGraph, edge: EdgeId, slot: usize, value: Point) {
    g.edge_records[edge.0 as usize]
        .route
        .header()
        .write_capacity_slot(slot, Some(SharedOwnerPoint::new(value)))
        .unwrap();
}
fn move_point(g: &ArenaGraph, edge: EdgeId, p: &SharedOwnerPoint, is_from: bool, row: bool) {
    let slot = if is_from { 1 } else { 2 };
    copy_slot(g, edge, slot, p.get());
    let other = g.edge_records[edge.0 as usize]
        .route
        .slot(3 - slot)
        .unwrap();
    let mut value = other.get();
    if row {
        value.y = p.get().y;
    } else {
        value.x = p.get().x;
    }
    other.set(value);
}
fn run(g: &mut ArenaGraph, guard: &RouteAtomicGuard<'_>) -> Result<()> {
    for node in nodes(g) {
        guard.step()?;
        let incident = g.owner_node_incident_header(node).unwrap();
        if incident.len() <= 1
            || g.owner_node_group_id(node, OwnerGroupKind::Cluster)
                .unwrap()
                .is_some()
        {
            continue;
        }
        let mut groups = BTreeMap::<OwnerGroupId, (bool, Vec<EdgeId>)>::new();
        for edge in incident.iter_slots().map(Option::unwrap) {
            guard.step()?;
            let e = &g.edge_records[edge.0 as usize];
            if e.route.len() != 4 {
                continue;
            }
            let adjacent = if e.from == node { e.to } else { e.from };
            if let Some(cluster) = g
                .owner_node_group_id(adjacent, OwnerGroupKind::Cluster)
                .unwrap()
            {
                let policy = g.owner_cluster_policy(cluster).unwrap();
                if policy.desired_arrangement != policy.arrangement {
                    continue;
                }
                let group = groups.entry(cluster).or_default();
                group.0 = e.from == node;
                group.1.push(edge);
            }
        }
        // Go ranges a pointer-keyed map here. Different source map orders are
        // observable for interacting groups; no stable ordering is promised.
        for (cluster, (is_from, group_edges)) in groups {
            guard.step()?;
            let row = g.owner_cluster_policy(cluster).unwrap().arrangement == "Row";
            let (mut back, mut front) = (false, false);
            for &edge in &group_edges {
                guard.step()?;
                let route = &g.edge_records[edge.0 as usize].route;
                let (a, b) = if is_from {
                    (route.point(3), route.point(0))
                } else {
                    (route.point(0), route.point(3))
                };
                let (a, b) = if row { (a.x, b.x) } else { (a.y, b.y) };
                back |= a < b;
                front |= a > b;
            }
            if !back || !front {
                continue;
            }
            let mut originals = BTreeMap::new();
            let mut candidates = Vec::new();
            for &edge in &group_edges {
                guard.step()?;
                let route = &g.edge_records[edge.0 as usize].route;
                candidates.push(route.slot(if is_from { 1 } else { 2 }).unwrap());
                originals.insert(edge, (route.point(1), route.point(2)));
            }
            let restore = |g: &ArenaGraph, guard: &RouteAtomicGuard<'_>| -> Result<()> {
                for &edge in &group_edges {
                    guard.step()?;
                    let (a, b) = originals[&edge];
                    copy_slot(g, edge, 1, a);
                    copy_slot(g, edge, 2, b);
                }
                Ok(())
            };
            let mut best = None;
            let mut best_cost = 0.0;
            'candidate: for candidate in candidates {
                guard.step()?;
                restore(g, guard)?;
                let mut cost = 0.0;
                for &edge in &group_edges {
                    guard.step()?;
                    let original_cost = estimate_cost(g, edge, guard)?;
                    if g.edge_records[edge.0 as usize]
                        .route
                        .slot(if is_from { 1 } else { 2 })
                        .unwrap()
                        .same_allocation(&candidate)
                    {
                        cost += original_cost;
                        continue;
                    }
                    move_point(g, edge, &candidate, is_from, row);
                    if intersects(g, edge, guard)? {
                        continue 'candidate;
                    }
                    let new_cost = estimate_cost(g, edge, guard)?;
                    cost += new_cost;
                    if new_cost > original_cost {
                        continue 'candidate;
                    }
                }
                if best.is_none() || cost < best_cost {
                    best = Some(candidate);
                    best_cost = cost;
                }
            }
            restore(g, guard)?;
            if let Some(best) = best {
                for &edge in &group_edges {
                    guard.step()?;
                    move_point(g, edge, &best, is_from, row);
                }
            }
        }
    }
    guard.finish()
}
pub(super) fn branch_with_context(
    g: &mut ArenaGraph,
    context: Option<&dyn limits::Context>,
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<()> {
    route_stage_guard::run_atomic_route_stage_typed(
        g,
        Some(OwnerGraphId(0)),
        context,
        "FixClusterEdgeBranching",
        &[],
        budget,
        run,
    )
}
pub(in crate::internal) fn fix_cluster_edge_branching_with_budget(
    g: &mut ArenaGraph,
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<()> {
    branch_with_context(g, Some(&RuntimeRouteContext), budget)
}
pub(in crate::internal) fn fix_cluster_edge_branching(g: &mut ArenaGraph) {
    let _ = fix_cluster_edge_branching_with_budget(
        g,
        &super::route_search_guard::RouteStageWork::new(
            super::route_search_guard::MAX_ROUTE_STAGE_WORK_UNITS,
        ),
    );
}
#[cfg(test)]
fn trial_points(
    originals: &[Vec<Point>],
    edge_offset: usize,
    candidate: Point,
    back_edge: bool,
    arrangement: ClusterArrangement,
) -> Vec<Point> {
    let mut points = originals[edge_offset].clone();
    if back_edge {
        points[1] = candidate;
        match arrangement {
            ClusterArrangement::Row => points[2].y = candidate.y,
            ClusterArrangement::Column => points[2].x = candidate.x,
        }
    } else {
        points[2] = candidate;
        match arrangement {
            ClusterArrangement::Row => points[1].y = candidate.y,
            ClusterArrangement::Column => points[1].x = candidate.x,
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, Size};

    fn publish_cluster(graph: &mut ArenaGraph, members: &[NodeId]) {
        let mut vessel_input = Graph::default();
        let id = vessel_input.add_node(branch_test_node("cluster-vessel"));
        vessel_input.set_initial_position(
            id,
            Some(Point {
                x: 11000.0,
                y: 11000.0,
            }),
        );
        let mut vessel_entity = ArenaGraph::from_input(&vessel_input).nodes.remove(0);
        vessel_entity.tala_id = graph.nodes.iter().map(|n| n.tala_id).max().unwrap() + 1;
        let vessel = graph.retain_topology_node(vessel_entity);
        graph
            .set_owner_node_graph_reference(vessel, Some(OwnerGraphId(0)))
            .unwrap();
        let group = graph.new_owner_group(OwnerGroupKind::Cluster);
        let mut fields = graph.owner_group_fields(group).unwrap();
        fields.vessel = Some(vessel);
        fields.graph = Some(OwnerGraphId(0));
        fields.nodes = graph
            .allocate_owner_member_header(
                &members.iter().copied().map(Some).collect::<Vec<_>>(),
                members.len(),
            )
            .unwrap();
        graph.set_owner_group_fields(group, fields).unwrap();
        graph
            .set_owner_cluster_policy(
                group,
                crate::internal::owner_groups::OwnerClusterPolicy {
                    arrangement: "Row".into(),
                    desired_arrangement: "Row".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        for &node in members {
            graph
                .set_owner_node_group(node, OwnerGroupKind::Cluster, Some(group))
                .unwrap();
        }
    }
    fn branch_test_node(id: &str) -> Node {
        let mut node = Node::new(
            id,
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        node.content_insets = crate::Insets::uniform(0.0);
        node
    }

    #[test]
    fn recovered_branch_trial_moves_the_shared_point_and_parallel_leg() {
        let originals = vec![vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 10.0, y: 0.0 },
            Point { x: 10.0, y: 30.0 },
            Point { x: 40.0, y: 30.0 },
        ]];

        assert_eq!(
            trial_points(
                &originals,
                0,
                Point { x: 20.0, y: 15.0 },
                true,
                ClusterArrangement::Row,
            ),
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 20.0, y: 15.0 },
                Point { x: 10.0, y: 15.0 },
                Point { x: 40.0, y: 30.0 },
            ]
        );
        assert_eq!(
            trial_points(
                &originals,
                0,
                Point { x: 20.0, y: 15.0 },
                false,
                ClusterArrangement::Column,
            ),
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 20.0, y: 0.0 },
                Point { x: 20.0, y: 15.0 },
                Point { x: 40.0, y: 30.0 },
            ]
        );
    }

    #[test]
    fn upstream_lowest_cost_cluster_branching_fixture_matches() {
        let mut input = Graph::default();
        // Keep a non-branch edge before the cluster group. Upstream scores
        // branch candidates against this full graph route list, whose indices
        // include edges outside the current group.
        let unrelated_source = input.add_node(branch_test_node("unrelated-source"));
        let unrelated_target = input.add_node(branch_test_node("unrelated-target"));
        input.set_initial_position(
            unrelated_source,
            Some(Point {
                x: 30_000.0,
                y: 30_000.0,
            }),
        );
        input.set_initial_position(
            unrelated_target,
            Some(Point {
                x: 31_000.0,
                y: 31_000.0,
            }),
        );
        let unrelated_edge = input.add_edge(Edge {
            source: unrelated_source,
            target: unrelated_target,
        });
        input.set_edge_route(
            unrelated_edge,
            vec![
                Point {
                    x: 10_000.0,
                    y: 10_000.0,
                },
                Point {
                    x: 10_000.0,
                    y: 10_010.0,
                },
            ],
        );
        let external = input.add_node(branch_test_node("external"));
        let left = input.add_node(branch_test_node("left"));
        let right = input.add_node(branch_test_node("right"));
        input.set_initial_position(
            external,
            Some(Point {
                x: 10_000.0,
                y: 10_000.0,
            }),
        );
        input.set_initial_position(
            left,
            Some(Point {
                x: 11_000.0,
                y: 11_000.0,
            }),
        );
        input.set_initial_position(
            right,
            Some(Point {
                x: 12_000.0,
                y: 12_000.0,
            }),
        );

        let left_edge = input.add_edge(Edge {
            source: external,
            target: left,
        });
        let right_edge = input.add_edge(Edge {
            source: external,
            target: right,
        });
        input.set_edge_route(
            left_edge,
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 0.0, y: -10.0 },
                Point {
                    x: -100.0,
                    y: -10.0,
                },
                Point {
                    x: -100.0,
                    y: 100.0,
                },
            ],
        );
        input.set_edge_route(
            right_edge,
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 0.0, y: 10.0 },
                Point { x: 100.0, y: 10.0 },
                Point { x: 100.0, y: -5.0 },
            ],
        );

        let mut graph = ArenaGraph::from_input(&input);
        let cluster_index = graph.clusters.len();
        graph.clusters.push(crate::internal::ClusterState {
            members: vec![left, right],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 0.0,
            vessel_tala_id: 4,
            fixed_size: false,
        });
        graph.nodes[left.0 as usize].cluster = Some(cluster_index);
        graph.nodes[right.0 as usize].cluster = Some(cluster_index);

        publish_cluster(&mut graph, &[left, right]);
        fix_cluster_edge_branching_with_budget(
            &mut graph,
            &super::super::route_search_guard::RouteStageWork::new(100000),
        )
        .unwrap();

        assert_eq!(
            graph.edge_records[left_edge.0 as usize].route.point(1).y,
            10.0
        );
        assert_eq!(
            graph.edge_records[right_edge.0 as usize].route.point(1).y,
            10.0
        );
        assert_eq!(
            graph.edge_records[left_edge.0 as usize].route.point(2).y,
            10.0
        );
    }

    #[test]
    fn sequential_trial_cost_rejects_all_cluster_branch_candidates() {
        let mut input = Graph::default();
        let external = input.add_node(branch_test_node("external"));
        let first = input.add_node(branch_test_node("first"));
        let second = input.add_node(branch_test_node("second"));
        let third = input.add_node(branch_test_node("third"));
        for (index, node) in [external, first, second, third].into_iter().enumerate() {
            let offset = 10_000.0 + index as f64 * 1_000.0;
            input.set_initial_position(
                node,
                Some(Point {
                    x: offset,
                    y: offset,
                }),
            );
        }

        let routes = [
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 0.0, y: -1.0 },
                Point { x: 7.0, y: -1.0 },
                Point { x: -30.0, y: 5.0 },
            ],
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 0.0, y: -19.0 },
                Point { x: 10.0, y: -19.0 },
                Point { x: 30.0, y: -12.0 },
            ],
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 0.0, y: -18.0 },
                Point { x: 17.0, y: -18.0 },
                Point { x: 17.0, y: -19.0 },
            ],
        ];
        let mut edges = Vec::new();
        for (target, route) in [first, second, third].into_iter().zip(routes.iter()) {
            let edge = input.add_edge(Edge {
                source: external,
                target,
            });
            input.set_edge_route(edge, route.clone());
            edges.push(edge);
        }

        let mut graph = ArenaGraph::from_input(&input);
        let cluster_index = graph.clusters.len();
        graph.clusters.push(crate::internal::ClusterState {
            members: vec![first, second, third],
            arrangement: ClusterArrangement::Row,
            desired_arrangement: ClusterArrangement::Row,
            padding: 0.0,
            vessel_tala_id: 5,
            fixed_size: false,
        });
        graph.nodes[first.0 as usize].cluster = Some(cluster_index);
        graph.nodes[second.0 as usize].cluster = Some(cluster_index);
        graph.nodes[third.0 as usize].cluster = Some(cluster_index);
        let original_routes = edges
            .iter()
            .map(|edge| graph.edge_records[edge.0 as usize].route.read().clone())
            .collect::<Vec<_>>();

        publish_cluster(&mut graph, &[first, second, third]);
        fix_cluster_edge_branching_with_budget(
            &mut graph,
            &super::super::route_search_guard::RouteStageWork::new(100000),
        )
        .unwrap();

        for (edge, original) in edges.iter().zip(original_routes) {
            assert_eq!(graph.edge_records[edge.0 as usize].route.read(), original);
        }
    }
    fn source_graph() -> ArenaGraph {
        let graph=crate::graphjson::graphjson_to_graph(br#"{"nodes":[{"id":1,"topLeft":{"x":10000,"y":10000},"width":10,"height":10},{"id":2,"topLeft":{"x":11000,"y":11000},"width":10,"height":10},{"id":3,"topLeft":{"x":12000,"y":12000},"width":10,"height":10}],"edges":[{"id":11,"from":1,"to":2,"points":[{"x":0,"y":0},{"x":0,"y":-10},{"x":-100,"y":-10},{"x":-100,"y":100}]},{"id":12,"from":1,"to":3,"points":[{"x":0,"y":0},{"x":0,"y":10},{"x":100,"y":10},{"x":100,"y":-5}]}],"containers":{"0":[1,2,3]},"clusters":{"99":{"Nodes":[2,3],"Arrangement":"Row","DesiredArrangement":"Row","Padding":0}},"clusterVessels":{"99":{"id":99,"topLeft":{"x":11000,"y":11000},"width":10,"height":10}}}"#).unwrap();
        ArenaGraph::from_input(&graph)
    }
    #[test]
    fn source_branching_preserves_route_backing_and_replaces_only_middle_pointers() {
        let mut graph = source_graph();
        let retained = graph.edge_records[0].route.clone();
        let points = (0..retained.len())
            .map(|i| retained.slot(i).unwrap())
            .collect::<Vec<_>>();
        let work = super::super::route_search_guard::RouteStageWork::new(100000);
        fix_cluster_edge_branching_with_budget(&mut graph, &work).unwrap();
        assert_eq!(work.used(), 202);
        let current = &graph.edge_records[0].route;
        for i in 0..retained.capacity() {
            assert_eq!(
                retained.header().slot_address(i),
                current.header().slot_address(i)
            );
        }
        for i in [0, 3] {
            assert!(points[i].same_allocation(&current.slot(i).unwrap()));
        }
        for i in [1, 2] {
            assert!(!points[i].same_allocation(&current.slot(i).unwrap()));
            assert_eq!(current.point(i).y, 10.0);
            assert_eq!(retained.point(i).y, 10.0);
            assert_eq!(points[i].get().y, -10.0);
        }
    }
    #[test]
    fn every_branching_work_limit_restores_retained_route_slots_and_points() {
        for limit in 0..202 {
            let mut graph = source_graph();
            let retained = graph
                .edge_records
                .iter()
                .map(|e| e.route.clone())
                .collect::<Vec<_>>();
            let slots = retained
                .iter()
                .map(|h| {
                    (0..h.capacity())
                        .map(|i| h.header().read_capacity_slot(i).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let values = retained
                .iter()
                .map(|h| h.read().clone())
                .collect::<Vec<_>>();
            let work = super::super::route_search_guard::RouteStageWork::new(limit);
            assert!(
                fix_cluster_edge_branching_with_budget(&mut graph, &work).is_err(),
                "limit {limit}"
            );
            for (edge, header) in retained.iter().enumerate() {
                assert_eq!(header.read(), values[edge]);
                assert_eq!(graph.edge_records[edge].route.read(), values[edge]);
                for (i, point) in slots[edge].iter().enumerate() {
                    assert_eq!(
                        header.header().slot_address(i),
                        graph.edge_records[edge].route.header().slot_address(i)
                    );
                    let now = header.header().read_capacity_slot(i).unwrap();
                    assert!(match (point, now) {
                        (Some(a), Some(b)) => a.same_allocation(&b),
                        (None, None) => true,
                        _ => false,
                    });
                }
            }
        }
    }
}
