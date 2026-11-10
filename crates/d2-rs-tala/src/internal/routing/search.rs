// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Candidate generation and ordered route selection.
//!
//! Routing flavors share immutable visibility topology but keep independent
//! occupied-port and accepted-route state. Completed flavors are folded in a
//! stable order so parallel evaluation does not change tie-breaking.

use super::*;
use d2_rs_core::stable_sort::StableSort;

fn same_base_port(first: Port, second: Port) -> bool {
    first.point == second.point
        && first.direction == second.direction
        && first.side == second.side
        && first.index == second.index
        && first.tunnel.is_none()
        && second.tunnel.is_none()
        && first.is_center == second.is_center
}

/// Tunnel ports are the endpoints of one specific OVG tunnel edge.  A route
/// may use both endpoints of that lane, or ordinary ports, but it must not
/// borrow one endpoint for an unrelated edge.  The flattened Rust inventory
/// contains the aliases independently, so enforce the ownership rule at each
/// candidate boundary just as the Go OVG does.
fn mismatched_tunnel_endpoints(source: Port, target: Port) -> bool {
    // Tunnel IDs describe the local endpoint insertion, while OVG identity is
    // established by the canonical coordinate.  Aliased endpoints can carry
    // different insertion IDs and are still connected by the same visibility
    // edge (the release's OVG stores a shared node pointer).  Coordinate and
    // owner checks in the visibility graph enforce admissibility; rejecting
    // unequal endpoint IDs here incorrectly discards valid release routes.
    let _ = (source, target);
    false
}

fn table_port_allowed(allowed: &Option<Vec<Port>>, candidate: Port) -> bool {
    allowed.as_ref().is_none_or(|ports| {
        ports
            .iter()
            .copied()
            .any(|port| same_base_port(port, candidate))
    })
}

fn follows_outward(delta: Point, side: PortSide) -> bool {
    let expected = outward(side);
    delta.x * expected.x + delta.y * expected.y > 0.0
        && (delta.x * expected.y - delta.y * expected.x).abs() < f64::EPSILON
}

fn candidate_route(
    graph: &ArenaGraph,
    source: Port,
    target: Port,
    source_node: NodeId,
    target_node: NodeId,
    boxes: &BTreeMap<NodeId, Rect>,
    explore_both_axis_orders: bool,
    allow_direct_intersection: bool,
) -> Vec<Point> {
    let candidates = if source.side == opposite(target.side) {
        match source.side {
            PortSide::Left | PortSide::Right => {
                let middle = ((source.point.x + target.point.x) * 0.5).floor();
                vec![vec![
                    source.point,
                    Point {
                        x: middle,
                        y: source.point.y,
                    },
                    Point {
                        x: middle,
                        y: target.point.y,
                    },
                    target.point,
                ]]
            }
            PortSide::Top | PortSide::Bottom => {
                let middle = ((source.point.y + target.point.y) * 0.5).floor();
                vec![vec![
                    source.point,
                    Point {
                        x: source.point.x,
                        y: middle,
                    },
                    Point {
                        x: target.point.x,
                        y: middle,
                    },
                    target.point,
                ]]
            }
        }
    } else if source.side == target.side {
        // The OVG contains both its padded subgraph boundary and the visible
        // axes immediately outside every obstacle band. This is one candidate
        // inventory for every shape; shape-specific port geometry and the
        // ordinary route score decide which visible track wins.
        let clearance = graph.cell_size + 1.0;
        let source_box = boxes[&source_node];
        let target_box = boxes[&target_node];
        let limit = match source.side {
            PortSide::Top => source_box.origin.y.min(target_box.origin.y) - clearance,
            PortSide::Bottom => source_box.bottom().max(target_box.bottom()) + clearance,
            PortSide::Left => source_box.origin.x.min(target_box.origin.x) - clearance,
            PortSide::Right => source_box.right().max(target_box.right()) + clearance,
        };
        // `newVisibilityGraph` also contributes the unbounded endpoint range
        // one hundred units beyond the outermost endpoint box.  Keep that
        // visible lane distinct from the padded subgraph boundary: the two
        // coincide only when the padding happens to be zero.
        let endpoint_boundary = match source.side {
            PortSide::Top => source_box.origin.y.min(target_box.origin.y) - 100.0,
            PortSide::Bottom => source_box.bottom().max(target_box.bottom()) + 100.0,
            PortSide::Left => source_box.origin.x.min(target_box.origin.x) - 100.0,
            PortSide::Right => source_box.right().max(target_box.right()) + 100.0,
        };
        let padded_boundary = match source.side {
            PortSide::Top | PortSide::Left => limit - 100.0,
            PortSide::Bottom | PortSide::Right => limit + 100.0,
        };
        let mut tracks = vec![endpoint_boundary, padded_boundary];
        for (node, rect) in boxes {
            if *node == source_node || *node == target_node {
                continue;
            }
            let obstacle_tracks = match source.side {
                PortSide::Top | PortSide::Bottom => {
                    [rect.origin.y - clearance, rect.bottom() + clearance]
                }
                PortSide::Left | PortSide::Right => {
                    [rect.origin.x - clearance, rect.right() + clearance]
                }
            };
            tracks.extend(
                obstacle_tracks
                    .into_iter()
                    .filter(|track| match source.side {
                        PortSide::Top | PortSide::Left => *track < limit,
                        PortSide::Bottom | PortSide::Right => *track > limit,
                    }),
            );
        }
        tracks.stable_sort_by(|left, right| match source.side {
            PortSide::Top | PortSide::Left => right.total_cmp(left),
            PortSide::Bottom | PortSide::Right => left.total_cmp(right),
        });
        tracks.dedup_by(|left, right| left == right);
        tracks
            .into_iter()
            .map(|track| match source.side {
                PortSide::Top | PortSide::Bottom => vec![
                    source.point,
                    Point {
                        x: source.point.x,
                        y: track,
                    },
                    Point {
                        x: target.point.x,
                        y: track,
                    },
                    target.point,
                ],
                PortSide::Left | PortSide::Right => vec![
                    source.point,
                    Point {
                        x: track,
                        y: source.point.y,
                    },
                    Point {
                        x: track,
                        y: target.point.y,
                    },
                    target.point,
                ],
            })
            .collect()
    } else {
        // Hierarchy scopes require an explicit visibility junction between
        // perpendicular rays. This direct candidate builder handles no such
        // junction, so it cannot invent one across scope boundaries. Tunnel
        // pairs are handled earlier; flat scopes may use the intersection below.
        if !allow_direct_intersection {
            return Vec::new();
        }
        // TALA's visibility graph includes the orthogonal intersection of
        // perpendicular endpoint rays. When that intersection is unobstructed,
        // findLShapedRoute returns the direct three-point path without adding
        // artificial clearance stubs.
        let direct_source_axis = Point {
            x: source.point.x,
            y: target.point.y,
        };
        let direct_target_axis = Point {
            x: target.point.x,
            y: source.point.y,
        };
        let source_delta = outward(source.side);
        let first = Point {
            x: source.point.x + source_delta.x * 30.0,
            y: source.point.y + source_delta.y * 30.0,
        };
        let target_delta = outward(target.side);
        let last = Point {
            x: target.point.x + target_delta.x * 30.0,
            y: target.point.y + target_delta.y * 30.0,
        };
        let first_corner = if (first.x - last.x).abs() < (first.y - last.y).abs() {
            Point {
                x: first.x,
                y: last.y,
            }
        } else {
            Point {
                x: last.x,
                y: first.y,
            }
        };
        let second_corner = if first_corner.x == first.x {
            Point {
                x: last.x,
                y: first.y,
            }
        } else {
            Point {
                x: first.x,
                y: last.y,
            }
        };
        // Recovered findLShapedRoute explores both axis orders. Preserve the
        // shorter-axis-first order as the tie preference. The alternate is
        // active with the represented flat flavor search below; the current
        // hierarchy-aware approximation retains its existing candidate set.
        let mut routes = Vec::new();
        if allow_direct_intersection {
            routes.push(vec![source.point, direct_source_axis, target.point]);
            routes.push(vec![source.point, direct_target_axis, target.point]);
        }
        routes.push(vec![source.point, first, first_corner, last, target.point]);
        if explore_both_axis_orders {
            routes.push(vec![source.point, first, second_corner, last, target.point]);
        }
        routes
    };
    for route in candidates {
        let route = simplify_route(route);
        let leaves_source = route.get(1).is_some_and(|next| {
            follows_outward(
                Point {
                    x: next.x - source.point.x,
                    y: next.y - source.point.y,
                },
                source.side,
            )
        });
        let enters_target = route
            .len()
            .checked_sub(2)
            .and_then(|index| route.get(index))
            .is_some_and(|previous| {
                follows_outward(
                    Point {
                        x: previous.x - target.point.x,
                        y: previous.y - target.point.y,
                    },
                    target.side,
                )
            });
        // Recovered `addNodesIntersections` calls `Graph.isPointNearANode`,
        // which rejects an intersection inside the fixed twenty-unit band of
        // any non-container node. Containers are deliberately skipped. This
        // is an OVG-topology rule, not an endpoint-route surcharge: a direct
        // perpendicular candidate through some third node's exclusion band
        // does not exist in TALA's graph at all.
        let clears_turn_band = is_ancestor(graph, source_node, target_node)
            || is_ancestor(graph, target_node, source_node)
            || route[1..route.len() - 1].iter().all(|turn| {
                boxes.iter().all(|(node, rect)| {
                    graph.nodes[node.0 as usize].is_container
                        || !point_near_rect(*turn, *rect, 20.0)
                })
            });
        if leaves_source
            && enters_target
            && clears_turn_band
            && route_is_clear(graph, &route, boxes, source_node, target_node)
        {
            return route;
        }
    }
    Vec::new()
}

fn length(route: &[Point]) -> f64 {
    route
        .windows(2)
        .map(|pair| (pair[0].x - pair[1].x).abs() + (pair[0].y - pair[1].y).abs())
        .sum()
}

pub(super) fn ideal_turn_axes_for_slingshot(source: Rect, target: Rect) -> Vec<(bool, f64)> {
    let source_after_target_x = source.origin.x > target.right();
    let target_after_source_x = target.origin.x > source.right();
    let source_after_target_y = source.origin.y > target.bottom();
    let target_after_source_y = target.origin.y > source.bottom();

    if source_after_target_y && source_after_target_x {
        vec![
            (true, (target.right() + source.origin.x) * 0.5),
            (false, (target.bottom() + source.origin.y) * 0.5),
        ]
    } else if source_after_target_x && target_after_source_y {
        vec![
            (true, (target.right() + source.origin.x) * 0.5),
            (false, (target.origin.y + source.bottom()) * 0.5),
        ]
    } else if target_after_source_y && target_after_source_x {
        vec![
            (true, (target.origin.x + source.right()) * 0.5),
            (false, (target.origin.y + source.bottom()) * 0.5),
        ]
    } else if target_after_source_x && source_after_target_y {
        vec![
            (true, (target.origin.x + source.right()) * 0.5),
            (false, (target.bottom() + source.origin.y) * 0.5),
        ]
    } else if source_after_target_x {
        vec![(true, (target.right() + source.origin.x) * 0.5)]
    } else if target_after_source_x {
        vec![(true, (target.origin.x + source.right()) * 0.5)]
    } else if target_after_source_y {
        vec![(false, (target.origin.y + source.bottom()) * 0.5)]
    } else if source_after_target_y {
        vec![(false, (target.bottom() + source.origin.y) * 0.5)]
    } else {
        Vec::new()
    }
}

pub(super) fn distance_to_boundary_for_ovg(point: Point, rect: Rect) -> f64 {
    let right = point.x > rect.right();
    let left = point.x < rect.origin.x;
    let bottom = point.y > rect.bottom();
    let top = point.y < rect.origin.y;
    if bottom && right {
        crate::internal::routing::channel_points::go_hypot(
            point.x - rect.right(),
            point.y - rect.bottom(),
        )
    } else if right && top {
        crate::internal::routing::channel_points::go_hypot(
            point.x - rect.right(),
            point.y - rect.origin.y,
        )
    } else if top && left {
        crate::internal::routing::channel_points::go_hypot(
            point.x - rect.origin.x,
            point.y - rect.origin.y,
        )
    } else if left && bottom {
        crate::internal::routing::channel_points::go_hypot(
            point.x - rect.origin.x,
            point.y - rect.bottom(),
        )
    } else if right {
        point.x - rect.right()
    } else if left {
        rect.origin.x - point.x
    } else if top {
        rect.origin.y - point.y
    } else if bottom {
        point.y - rect.bottom()
    } else {
        0.0
    }
}

fn point_near_rect(point: Point, rect: Rect, margin: f64) -> bool {
    rect.origin.x - margin <= point.x
        && point.x <= rect.right() + margin
        && rect.origin.y - margin <= point.y
        && point.y <= rect.bottom() + margin
}

/// Recovered `Cluster.hasDesirableArrangementTo` for the materialized Rust
/// cluster state. An endpoint cluster prefers row members when the other node
/// is above or below its vessel, and column members when it is left or right.
fn cluster_has_desirable_arrangement_to(
    graph: &ArenaGraph,
    cluster_index: usize,
    other: NodeId,
    boxes: &BTreeMap<NodeId, Rect>,
) -> bool {
    let cluster = &graph.clusters[cluster_index];
    let bounds = if graph.cluster_is_active(cluster) {
        let Some(&owner) = cluster.members.first() else {
            return false;
        };
        let Some(origin) = graph
            .active_node_position(owner)
            .or_else(|| boxes.get(&owner).map(|rect| rect.origin))
        else {
            return false;
        };
        Rect {
            origin,
            size: graph.active_node_size(owner),
        }
    } else {
        let Some(first) = cluster.members.first().and_then(|member| boxes.get(member)) else {
            return false;
        };
        let Some(last) = cluster.members.last().and_then(|member| boxes.get(member)) else {
            return false;
        };
        Rect {
            origin: first.origin,
            size: crate::Size {
                width: last.right() - first.origin.x,
                height: last.bottom() - first.origin.y,
            },
        }
    };
    let other = boxes[&other];
    let orientation =
        graph.sized_box_orientation((bounds.origin, bounds.size), (other.origin, other.size));
    match orientation {
        Orientation::Top | Orientation::Bottom => {
            cluster.arrangement == super::super::ClusterArrangement::Row
        }
        Orientation::Left | Orientation::Right => {
            cluster.arrangement == super::super::ClusterArrangement::Column
        }
        _ => false,
    }
}

/// Recovered `Graph.getSourceAndTargetClusterNodes`.  Cluster membership is
/// graph-owned state, so it must be consulted independently of the endpoint's
/// current `Node.cluster` backlink (which can be stale while aggregate
/// vessels are being flattened or restored).
/// Turn contribution from recovered `ovgEdgeRouter.search`.
///
/// Ordinary OVG search slightly rewards turns on the midpoint axes returned
/// by `Node.getIdealTurnAxes`. It also charges an additional full turn when a
/// turn occurs within twenty units of either endpoint boundary.
fn search_turn_cost(
    graph: &ArenaGraph,
    source: NodeId,
    target: NodeId,
    route: &[Point],
    boxes: &BTreeMap<NodeId, Rect>,
) -> f64 {
    if route.len() < 3 {
        return 0.0;
    }
    let source_rect = boxes[&source];
    let target_rect = boxes[&target];
    let axes = ideal_turn_axes_for_slingshot(source_rect, target_rect);
    let has_ancestry = is_ancestor(graph, source, target) || is_ancestor(graph, target, source);
    let mut cost = 0.0;

    for index in 1..route.len() - 1 {
        let previous = route[index - 1];
        let turn = route[index];
        let next = route[index + 1];
        let arrived_horizontally = previous.y == turn.y;
        let leaves_horizontally = turn.y == next.y;
        if arrived_horizontally == leaves_horizontally {
            continue;
        }
        let multiplier = if axes.iter().any(|(vertical, position)| {
            if *vertical {
                (turn.x - position).abs() <= 4.0
            } else {
                (turn.y - position).abs() <= 4.0
            }
        }) {
            0.98
        } else {
            1.0
        };
        cost += multiplier * graph.routing_costs.1;
        if !has_ancestry {
            if distance_to_boundary_for_ovg(turn, target_rect) <= 20.0 {
                cost += graph.routing_costs.1;
            }
            if distance_to_boundary_for_ovg(turn, source_rect) <= 20.0 {
                cost += graph.routing_costs.1;
            }
        }
    }
    cost
}

/// Translation of recovered `Route.isOpposingColinear`.
///
/// A directed route that merely enters or leaves another route at a shared
/// segment endpoint is not opposing. This endpoint exclusion is material:
/// comparing segment direction with a dot product alone incorrectly rejects
/// the two warehouse-to-container routes that converge on the same target
/// lane.
fn route_is_opposing_collinear(
    route: &[Point],
    centers: Option<(Point, Point)>,
    from: Point,
    to: Point,
) -> bool {
    // Go scans OVGNodes from the source center through the last port,
    // excluding only the target-center connector. The initial connector can
    // return false for endpoint-only contact before a later opposing segment.
    let source_connector = centers.and_then(|(source, _)| {
        route
            .first()
            .filter(|point| **point != source)
            .map(|point| [source, *point])
    });
    let end = if centers.is_some_and(|(_, target)| route.last() == Some(&target)) {
        route.len().saturating_sub(1)
    } else {
        route.len()
    };
    for segment in source_connector
        .into_iter()
        .chain(route[..end].windows(2).map(|s| [s[0], s[1]]))
    {
        let start = segment[0];
        let end = segment[1];
        if from.y == to.y && start.y == from.y && end.y == from.y {
            let candidate_min = from.x.min(to.x);
            let candidate_max = from.x.max(to.x);
            let route_min = start.x.min(end.x);
            let route_max = start.x.max(end.x);
            if candidate_max < route_min || route_max < candidate_min {
                continue;
            }
            if route_min <= from.x
                && from.x <= route_max
                && (to.x < route_min || route_max < to.x)
                && (from == start || from == end)
            {
                return false;
            }
            if route_min <= to.x
                && to.x <= route_max
                && (from.x < route_min || route_max < from.x)
                && (to == start || to == end)
            {
                return false;
            }
            if (end.x > start.x && to.x < from.x) || (start.x > end.x && from.x < to.x) {
                return true;
            }
        } else if from.x == to.x && start.x == from.x && end.x == from.x {
            let candidate_min = from.y.min(to.y);
            let candidate_max = from.y.max(to.y);
            let route_min = start.y.min(end.y);
            let route_max = start.y.max(end.y);
            if candidate_max < route_min || route_max < candidate_min {
                continue;
            }
            if route_min <= from.y
                && from.y <= route_max
                && (to.y < route_min || route_max < to.y)
                && (from == start || from == end)
            {
                return false;
            }
            if route_min <= to.y
                && to.y <= route_max
                && (from.y < route_min || route_max < from.y)
                && (to == start || to == end)
            {
                return false;
            }
            if (end.y > start.y && to.y < from.y) || (start.y > end.y && from.y < to.y) {
                return true;
            }
        }
    }
    false
}

/// Direct translation of `Route.isEntireColinear`.
///
/// TALA applies this stricter test when the next OVG node is a tunnel: every
/// interior segment of the accepted route must be collinear with and contained
/// by the candidate tunnel span.
fn route_is_entire_collinear(route: &[Point], from: Point, to: Point) -> bool {
    if from.y == to.y {
        let min = from.x.min(to.x);
        let max = from.x.max(to.x);
        return route.windows(2).all(|segment| {
            segment[0].y == from.y
                && segment[1].y == from.y
                && min <= segment[0].x.min(segment[1].x)
                && segment[0].x.max(segment[1].x) <= max
        });
    }
    if from.x == to.x {
        let min = from.y.min(to.y);
        let max = from.y.max(to.y);
        return route.windows(2).all(|segment| {
            segment[0].x == from.x
                && segment[1].x == from.x
                && min <= segment[0].y.min(segment[1].y)
                && segment[0].y.max(segment[1].y) <= max
        });
    }
    false
}

fn route_edge_debug_id(graph: &ArenaGraph, edge_index: usize) -> String {
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
    let from_node = &graph.nodes[edge.from.0 as usize];
    let mut from = from_node
        .d2_id
        .clone()
        .unwrap_or_else(|| from_node.tala_id.to_string());
    if let Some(column) = edge.source_table_column {
        from.push_str(&format!("[{column}]"));
    }
    let to_node = &graph.nodes[edge.to.0 as usize];
    let mut to = to_node
        .d2_id
        .clone()
        .unwrap_or_else(|| to_node.tala_id.to_string());
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

type PointKey = (u64, u64);
/// One search pop's pointToRoute lookup and the point key it answers.
pub(super) type OccupiedMemo<'a> = Option<((u64, u64), &'a [usize])>;
type PortKey = (PointKey, PortSide, usize, Option<usize>);

pub(super) fn routing_precision_compare(left: f64, right: f64) -> std::cmp::Ordering {
    const PRECISION: f64 = 0.0001;
    if (left - right).abs() < PRECISION {
        std::cmp::Ordering::Equal
    } else {
        left.total_cmp(&right)
    }
}

fn interaction_point_key(point: Point) -> PointKey {
    super::go_point_map_key(point)
}

fn port_key(port: Port) -> PortKey {
    (
        interaction_point_key(port.point),
        port.side,
        port.index,
        port.tunnel,
    )
}

struct SymmetryEndpoint<'a> {
    node: NodeId,
    shape: ShapeKind,
    raw_ports: &'a [Port],
    shape_ports: &'a [Port],
    used: &'a BTreeSet<PortKey>,
}

/// centerSymmetricalPorts reserves the raw inventories, scans every port,
/// and caches marks by canonical OVG node identity across both endpoints.
fn center_symmetrical_ports(
    visibility: &super::visibility::VisibilityGraph,
    endpoints: &[SymmetryEndpoint<'_>; 2],
    budget: Option<&LocalRouteSearchWork>,
) -> Option<BTreeSet<usize>> {
    if let Some(budget) = budget {
        budget
            .reserve_sum(
                endpoints[0].raw_ports.len() as u64,
                endpoints[1].raw_ports.len() as u64,
            )
            .ok()?;
    }
    let mut marked = BTreeSet::new();
    for endpoint in endpoints {
        let mut mirrored = BTreeMap::new();
        for (index, port) in endpoint.shape_ports.iter().enumerate() {
            if let Some(mirror) = mirrored_port_index(endpoint.shape, index, endpoint.shape_ports)
                .and_then(|mirror| endpoint.shape_ports.get(mirror))
            {
                mirrored.insert(
                    interaction_point_key(port.point),
                    interaction_point_key(mirror.point),
                );
            }
        }
        for port in endpoint.raw_ports {
            if let Some(budget) = budget {
                budget.step().ok()?;
            }
            if mirrored
                .get(&interaction_point_key(port.point))
                .is_some_and(|mirror| endpoint.used.iter().any(|(point, ..)| point == mirror))
                && let Some(identity) = visibility.port_index(endpoint.node, port.point)
            {
                marked.insert(identity);
            }
        }
    }
    if let Some(budget) = budget {
        budget.check().ok()?;
    }
    Some(marked)
}

fn mirrored_center_port_coordinate_is_used(
    shape: ShapeKind,
    port: Port,
    ports: &[Port],
    used: &BTreeSet<PortKey>,
) -> bool {
    let Some(index) = ports.iter().position(|candidate| {
        candidate.side == port.side
            && candidate.index == port.index
            && candidate.point == port.point
    }) else {
        return false;
    };
    mirrored_port_index(shape, index, ports)
        .and_then(|mirror| ports.get(mirror))
        .is_some_and(|mirror| {
            let mirror_point = interaction_point_key(mirror.point);
            used.iter()
                .any(|(used_point, ..)| *used_point == mirror_point)
        })
}

fn parallel_axis_overlap(first: [Point; 2], second: [Point; 2]) -> Option<f64> {
    let first_vertical = first[0].x == first[1].x;
    let second_vertical = second[0].x == second[1].x;
    let first_horizontal = first[0].y == first[1].y;
    let second_horizontal = second[0].y == second[1].y;
    if first_vertical && second_vertical {
        let overlap = first[0].y.min(first[1].y) <= second[0].y.max(second[1].y)
            && second[0].y.min(second[1].y) <= first[0].y.max(first[1].y);
        return overlap.then_some((first[0].x - second[0].x).abs());
    }
    if first_horizontal && second_horizontal {
        let overlap = first[0].x.min(first[1].x) <= second[0].x.max(second[1].x)
            && second[0].x.min(second[1].x) <= first[0].x.max(first[1].x);
        return overlap.then_some((first[0].y - second[0].y).abs());
    }
    None
}

/// Incremental translation of TALA's `ovgEdgeRouter.addRoute` indexes.
///
/// TALA performs the geometric comparisons once after accepting a route.
/// Search then probes these edge- and point-keyed collections in constant
/// time; rebuilding the same facts for every Dijkstra hop is both structurally
/// unfaithful and catastrophic for dense OVGs.
pub(super) struct RouteInteractionIndex {
    /// Diagnostic-only owner of this accepted-route index. Each Go flavor has
    /// its own router and therefore its own edge set; retaining the label lets
    /// source comparisons separate otherwise interleaved worker traces.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) trace_flavor: Option<String>,
    static_index: super::static_edge_index::StaticEdgeIndex,
    route_centers: Vec<Option<(Point, Point)>>,
    tunnel_points: crate::internal::FloatKeyHashSet<PointKey>,
    point_to_routes: crate::internal::FloatKeyHashMap<PointKey, Vec<usize>>,
    /// Each append is a distinct Go Route object, even for the same GEdge.
    /// Point/overlap maps below contain indices into this retained inventory.
    routes: Vec<super::published_route::PublishedRoute>,
    /// Retained point allocation by OVG identity (None past a nil point or
    /// beyond the OVG), shared with the visibility graph's canonical view.
    shared_points: std::sync::Arc<[Option<crate::internal::owner_points::SharedOwnerPoint>]>,
    canonical_nodes: std::sync::Arc<Vec<usize>>,
    /// Insertion order of `routes`. Recovered `ovgEdgeRouter.routedEdges` is a
    /// slice, not an identity-sorted set: fixed routes are appended first and
    /// newly accepted routes follow routing order.
    accepted_edge_order: Vec<usize>,
    /// Test-only inventory of completed segment publication attempts.
    #[cfg(test)]
    routed_segments: Vec<[Point; 2]>,
    /// Retained source edge values, keyed by ordered node identity and Distance.
    routed_set: super::routed_edge_set::EdgeSet,
    consider_node_labels: bool,
    // Search visits only actual labels, as Go search's nodeLabelBoxes does.
    // Keep storage-ID order so cost additions and charged visits retain the
    // previous sparse inventory's order, including in split components.
    node_label_rects: Vec<(NodeId, Rect)>,
    positioned_arrowhead_labels: Vec<crate::internal::labels::PositionedArrowheadLabel>,
    /// Go search's sourceClusterNodes/targetClusterNodes, built once per
    /// search for (graph address, edge). Search holds `&ArenaGraph`, so the
    /// plain cluster fields cannot change before this search's last hop.
    search_clusters: Option<(usize, usize, super::interactions::SearchClusters)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RoutePublicationError {
    WorkLimit,
    OvgNodeIdentityMismatch,
    DuplicateOvgNode { index: usize, point: Point },
}

impl RoutePublicationError {
    fn into_layout_error(self, graph: &ArenaGraph, edge_index: usize) -> crate::LayoutError {
        match self {
            Self::WorkLimit => {
                crate::LayoutError::BadRouteState("route publication work limit exceeded".into())
            }
            Self::OvgNodeIdentityMismatch => crate::LayoutError::BadRouteState(format!(
                "OVG node identity count did not match route {}",
                route_edge_debug_id(graph, edge_index)
            )),
            Self::DuplicateOvgNode { index, point } => crate::LayoutError::DuplicateRouteOvgNode {
                index,
                point,
                edge: route_edge_debug_id(graph, edge_index),
            },
        }
    }
}

#[derive(Default)]
struct UsedPortState {
    source: BTreeSet<PortKey>,
    target: BTreeSet<PortKey>,
    duplicate_source: BTreeSet<PointKey>,
    duplicate_target: BTreeSet<PointKey>,
    shared_source: BTreeSet<PointKey>,
    shared_target: BTreeSet<PointKey>,
}

impl RouteInteractionIndex {
    /// Go usedPorts: retain repeated pointToRoute references and compare
    /// complete route node identities in separate endpoint port walks.
    fn used_ports(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        visibility: &super::visibility::VisibilityGraph,
        ports: &BTreeMap<NodeId, Vec<Port>>,
        source_members: &BTreeSet<NodeId>,
        target_members: &BTreeSet<NodeId>,
        undesirable: bool,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<UsedPortState> {
        let step = || -> Option<()> {
            if let Some(budget) = budget {
                budget.step().ok()?;
            }
            Some(())
        };
        let edge = &graph.edge_records[edge_index];
        let (source_center, target_center) = self.route_centers[edge_index]?;
        let source_ports = ports
            .get(&edge.from)
            .into_iter()
            .flatten()
            .copied()
            .map(|port| (port, visibility.port_index(edge.from, port.point)))
            .collect::<Vec<_>>();
        let target_ports = ports
            .get(&edge.to)
            .into_iter()
            .flatten()
            .copied()
            .map(|port| (port, visibility.port_index(edge.to, port.point)))
            .collect::<Vec<_>>();
        let mut state = UsedPortState::default();
        for center in [source_center, target_center] {
            step()?;
            for &other_index in self
                .point_to_routes
                .get(&interaction_point_key(center))
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                step()?;
                let published = &self.routes[other_index];
                let other = &graph.edge_records[published.edge];
                if other.from != edge.from
                    && other.to != edge.from
                    && other.from != edge.to
                    && other.to != edge.to
                {
                    continue;
                }
                // Node pointer identity and copied endpoints belong to this
                // Route occurrence, never to the last Route for its GEdge.
                let identities = published.identities.as_ref()?;
                let [from_port, to_port] = published.endpoints?;
                if other.from == edge.from && other.to == edge.to {
                    state
                        .duplicate_source
                        .insert(interaction_point_key(from_port));
                    state
                        .duplicate_target
                        .insert(interaction_point_key(to_port));
                }
                if !undesirable {
                    if other.to == edge.to && source_members.contains(&other.from) {
                        state.shared_target.insert(interaction_point_key(to_port));
                    }
                    if other.from == edge.from && target_members.contains(&other.to) {
                        state.shared_source.insert(interaction_point_key(from_port));
                    }
                }
                for &node_id in identities {
                    step()?;
                    for &(port, port_id) in &source_ports {
                        step()?;
                        if port_id == Some(node_id) {
                            state.source.insert(port_key(port));
                            break;
                        }
                    }
                    for &(port, port_id) in &target_ports {
                        step()?;
                        if port_id == Some(node_id) {
                            state.target.insert(port_key(port));
                            break;
                        }
                    }
                }
            }
        }
        if let Some(budget) = budget {
            budget.check().ok()?;
        }
        Some(state)
    }

    fn prepare_search_node_labels(
        &mut self,
        graph: &ArenaGraph,
        edge_index: usize,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<()> {
        // Pure graph reads with no work charge, so ahead of every poll.
        self.search_clusters = super::interactions::search_cluster_members(graph, edge_index)
            .map(|clusters| (graph as *const ArenaGraph as usize, edge_index, clusters));
        if !self.consider_node_labels {
            return Some(());
        }
        let edge = &graph.edge_records[edge_index];
        // Storage IDs remain global; only this component belongs to the
        // temporary routing graph constructed by SplitSubgraphs.
        let mut labels = vec![None; graph.nodes.len()];
        for &node_id in &graph.node_order {
            let node = &graph.nodes[node_id.0 as usize];
            if let Some(budget) = budget {
                budget.step().ok()?;
            }
            labels[node_id.0 as usize] = if node.input_id == edge.from
                || node.input_id == edge.to
                || !node.label_text_present
            {
                None
            } else {
                crate::internal::labels::positioned_node_label_rect(graph, node.input_id)
            };
        }
        #[cfg(feature = "diagnostic-traces")]
        if std::env::var("D2_TRACE_SEARCH_LABELS").as_deref() == Ok("1") {
            let inventory = graph.node_order.iter().map(|id| &graph.nodes[id.0 as usize]).map(|node| {
                let id = node.input_id;
                serde_json::json!({"node":node.tala_id,"external_id":node.external_id,"shape":format!("{:?}",node.shape),"endpoint":id==edge.from || id==edge.to,"text_present":node.label_text_present,"rect":labels[id.0 as usize].map(|rect| [format!("{:016x}",rect.origin.x.to_bits()),format!("{:016x}",rect.origin.y.to_bits()),format!("{:016x}",rect.size.width.to_bits()),format!("{:016x}",rect.size.height.to_bits())])})
            }).collect::<Vec<_>>();
            eprintln!(
                "{}",
                serde_json::json!({"event":"search-label-inventory","edge":edge.tala_id,"flavor":self.trace_flavor,"scope_nodes":graph.node_order.len(),"scope_edges":graph.edge_order.len(),"labels":inventory})
            );
        }
        // Publish only a completed per-search inventory. An interrupted scan
        // must not leave a partial label cache for the flavor's later edges.
        self.node_label_rects = labels
            .into_iter()
            .enumerate()
            .filter_map(|(index, rect)| rect.map(|rect| (NodeId(index as u32), rect)))
            .collect();
        Some(())
    }

    pub(super) fn new(visibility: &super::visibility::VisibilityGraph, graph: &ArenaGraph) -> Self {
        Self::new_with_node_labels(visibility, graph, false)
    }

    pub(super) fn new_with_node_labels(
        visibility: &super::visibility::VisibilityGraph,
        graph: &ArenaGraph,
        consider_node_labels: bool,
    ) -> Self {
        // `ovgEdgeRouter.addRoute` scans the complete OVG vertical and
        // horizontal edge maps.  That inventory includes direct tunnel edges
        // even though `connectNodes` omits tunnel vertices from its line
        // construction; accepted routes can therefore mark a tunnel edge as
        // nearby/overlapping and the next search must charge the recovered
        // shared-route cost.
        // Segment endpoints are pure reads with no poll between them, so
        // walking the inventory lazily reads the same points.
        let tunnel_points = visibility
            .segment_iter()
            .flatten()
            .filter(|point| visibility.is_tunnel_point(*point))
            .map(interaction_point_key)
            .collect::<crate::internal::FloatKeyHashSet<_>>();
        Self {
            #[cfg(feature = "diagnostic-traces")]
            trace_flavor: None,
            static_index: visibility.static_edge_index(),
            route_centers: graph
                .edge_records
                .iter()
                .map(|edge| {
                    Some((
                        visibility.center_point(edge.from)?,
                        visibility.center_point(edge.to)?,
                    ))
                })
                .collect(),
            tunnel_points,
            point_to_routes: Default::default(),

            routes: Vec::new(),
            shared_points: visibility.shared_route_points(),
            canonical_nodes: visibility.route_node_identities(),
            accepted_edge_order: Vec::new(),

            #[cfg(test)]
            routed_segments: Vec::new(),
            routed_set: super::routed_edge_set::EdgeSet::default(),
            consider_node_labels,
            node_label_rects: graph
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| {
                    (consider_node_labels && node.label_text_present)
                        .then(|| {
                            crate::internal::labels::positioned_node_label_rect(
                                graph,
                                node.input_id,
                            )
                        })
                        .flatten()
                        .map(|rect| (NodeId(index as u32), rect))
                })
                .collect(),
            positioned_arrowhead_labels: Vec::new(),
            search_clusters: None,
        }
    }

    pub(super) fn add_route(
        &mut self,
        graph: &ArenaGraph,
        edge_index: usize,
        route: &[Point],
        _trace_flavor: &str,
    ) {
        let _ = self.add_route_with_budget(graph, edge_index, route, _trace_flavor, None);
    }

    /// Publish an accepted route while charging the same mutation boundaries
    /// as v0.9.0's guarded `ovgEdgeRouter.addRoute`. The unbudgeted wrapper is
    /// retained for prepasses and tests; route flavors pass their local guard
    /// here so a failed publication leaves the flavor sticky-exhausted and the
    /// enclosing coordinator can discard the attempt.
    pub(super) fn add_route_with_budget(
        &mut self,
        graph: &ArenaGraph,
        edge_index: usize,
        route: &[Point],
        _trace_flavor: &str,
        budget: Option<&LocalRouteSearchWork>,
    ) -> bool {
        self.add_route_with_node_identity(
            graph,
            edge_index,
            route,
            None,
            true,
            None,
            None,
            _trace_flavor,
            budget,
        )
        .is_ok()
    }

    /// Publish a route while preserving the source OVG-node identities when
    /// the caller still has them. Fixed-route import uses this path because
    /// coordinate equality is not pointer equality in `ovgEdgeRouter.addRoute`.
    fn add_route_with_node_ids(
        &mut self,
        graph: &ArenaGraph,
        edge_index: usize,
        route: &[Point],
        ovg_node_ids: Option<&[usize]>,
        _trace_flavor: &str,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Result<(), RoutePublicationError> {
        self.add_route_with_node_identity(
            graph,
            edge_index,
            route,
            ovg_node_ids,
            false,
            None,
            None,
            _trace_flavor,
            budget,
        )
    }

    /// Publish a generated route with its exact visibility-graph node IDs.
    /// Unlike fixed routes, generated paths retain distinct center/port nodes
    /// that happen to share coordinates.
    fn add_generated_route_with_node_ids(
        &mut self,
        graph: &ArenaGraph,
        edge_index: usize,
        route: &[Point],
        ovg_node_ids: &[usize],
        visibility: Option<&super::visibility::VisibilityGraph>,
        _trace_flavor: &str,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Result<(), RoutePublicationError> {
        self.add_route_with_node_identity(
            graph,
            edge_index,
            route,
            Some(ovg_node_ids),
            true,
            None,
            visibility,
            _trace_flavor,
            budget,
        )
    }

    fn add_imported_route(
        &mut self,
        graph: &ArenaGraph,
        visibility: &super::visibility::VisibilityGraph,
        route: &super::ovg_import::ExistingRoute,
        trace_flavor: &str,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Result<(), RoutePublicationError> {
        self.add_route_with_node_identity(
            graph,
            route.edge.0 as usize,
            &[],
            Some(&route.nodes),
            false,
            Some((visibility, route)),
            None,
            trace_flavor,
            budget,
        )
    }

    fn add_route_with_node_identity(
        &mut self,
        graph: &ArenaGraph,
        edge_index: usize,
        route: &[Point],
        ovg_node_ids: Option<&[usize]>,
        preserve_coincident_centers: bool,
        live_route: Option<(
            &super::visibility::VisibilityGraph,
            &super::ovg_import::ExistingRoute,
        )>,
        generated_visibility: Option<&super::visibility::VisibilityGraph>,
        _trace_flavor: &str,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Result<(), RoutePublicationError> {
        if route.is_empty() && live_route.is_none() && generated_visibility.is_none() {
            return Ok(());
        }
        let mut route_nodes = Vec::with_capacity(route.len() + 2);
        // Generated Go routes normally contain the source and target center
        // pointers, even when a center port has the same coordinates. Those
        // are distinct OVG nodes and both occurrences feed pointToRoute and
        // route-work accounting. Imported fixed routes are different: their
        // routeAdditionalEdges constructor interns consecutive equal points,
        // and its identity list already reflects that collapsed path.
        if let Some((source, _)) = self.route_centers[edge_index]
            && (preserve_coincident_centers || route.first().is_none_or(|point| *point != source))
        {
            route_nodes.push(source);
        }
        route_nodes.extend(route.iter().copied());
        if let Some((_, target)) = self.route_centers[edge_index]
            && (preserve_coincident_centers || route.last().is_none_or(|point| *point != target))
        {
            route_nodes.push(target);
        }
        if let Some((visibility, imported)) = live_route {
            route_nodes = imported
                .nodes
                .iter()
                .map(|&node| visibility.point(node))
                .collect();
        }
        if let (Some(visibility), Some(ids)) = (generated_visibility, ovg_node_ids) {
            // bestRoute can traverse through the source center and finish
            // at another OVG node. Go retains that exact sequence, rather
            // than replacing the first node with the nominal source center.
            if let (Some(point), Some(&id)) = (route_nodes.first_mut(), ids.first()) {
                *point = visibility.point(id);
            }
            if let (Some(point), Some(&id)) = (route_nodes.last_mut(), ids.last()) {
                *point = visibility.point(id);
            }
        }
        // Generated paths can also reference imported shared points. Retain
        // those allocations independently of their originating graph edge.
        let mut live = ovg_node_ids.map_or_else(Vec::new, |ids| {
            ids.iter()
                .map(|&id| self.shared_points.get(id).and_then(Clone::clone))
                .collect::<Vec<_>>()
        });
        if live.iter().all(Option::is_none) {
            live.clear();
        }
        let point_at = |index: usize| {
            live.get(index)
                .and_then(Option::as_ref)
                .map_or_else(|| route_nodes[index], |point| point.get())
        };
        let snapshot_nodes = || (0..route_nodes.len()).map(point_at).collect::<Vec<_>>();
        // Generated Route endpoint values are copied before addRoute starts.
        let copied_endpoints = live_route
            .map(|(_, imported)| imported.endpoints)
            .or_else(|| {
                (route_nodes.len() >= 2).then(|| [point_at(1), point_at(route_nodes.len() - 2)])
            });
        #[cfg(feature = "diagnostic-traces")]
        let trace_publication = |phase: &str| {
            if std::env::var("D2_TRACE_PUBLICATION_WORK").as_deref() == Ok("1") {
                eprintln!(
                    "{}",
                    serde_json::json!({"event":"publication-work","scope_nodes":graph.node_order.len(),"scope_edges":graph.edge_order.len(), "phase":phase, "flavor":_trace_flavor, "edge":graph.edge_records[edge_index].tala_id, "nodes":route_nodes.len(), "work":budget.map(LocalRouteSearchWork::used)})
                );
            }
        };
        #[cfg(feature = "diagnostic-traces")]
        trace_publication("begin");
        #[cfg(feature = "diagnostic-traces")]
        if std::env::var("D2_TRACE_OVG_INVENTORY_EDGE").ok().as_deref()
            == Some(graph.edge_records[edge_index].tala_id.to_string().as_str())
        {
            let inventory = |axes: &super::static_edge_index::Axes| {
                axes.buckets
                    .values()
                    .flat_map(|bucket| {
                        bucket.edges.iter().map(|&id| {
                            let [from, to] = self.static_index.edge_points(id);
                            [
                                format!("{:016x}", bucket.axis.to_bits()),
                                format!("{:016x}", from.x.to_bits()),
                                format!("{:016x}", from.y.to_bits()),
                                format!("{:016x}", to.x.to_bits()),
                                format!("{:016x}", to.y.to_bits()),
                            ]
                        })
                    })
                    .collect::<Vec<_>>()
            };
            eprintln!(
                "{}",
                serde_json::json!({"event":"ovg-inventory","edge":graph.edge_records[edge_index].tala_id,"flavor":_trace_flavor,"scope_nodes":graph.node_order.len(),"scope_edges":graph.edge_order.len(),"vertical":inventory(&self.static_index.vertical),"horizontal":inventory(&self.static_index.horizontal)})
            );
        }
        // Go checks duplicate OVG node pointers before publishing anything
        // and charges one unit for every checked node. A loop's first and
        // last OVG nodes are the same center; upstream drops the first center
        // for this check while retaining the complete route for publication.
        let is_loop = edge_index < graph.edge_records.len()
            && graph.edge_records[edge_index].from == graph.edge_records[edge_index].to;
        if let Some(node_ids) = ovg_node_ids {
            if node_ids.len() != route_nodes.len() {
                return Err(RoutePublicationError::OvgNodeIdentityMismatch);
            }
            let duplicate_check_ids = if is_loop && !node_ids.is_empty() {
                &node_ids[1..]
            } else {
                node_ids
            };
            let mut seen = BTreeSet::new();
            for (index, node_id) in duplicate_check_ids.iter().copied().enumerate() {
                if !budget.is_none_or(|budget| budget.step().is_ok()) {
                    return Err(RoutePublicationError::WorkLimit);
                }
                if !seen.insert(node_id) {
                    return Err(RoutePublicationError::DuplicateOvgNode {
                        index,
                        point: point_at(index + usize::from(is_loop)),
                    });
                }
            }
        } else {
            // Routes whose OVG identity has already been erased still pay the
            // upstream scan cost, but coordinate equality cannot safely be
            // promoted to pointer equality (hierarchy OVGs retain coincident
            // owner-specific nodes).
            let duplicate_check_nodes = if is_loop && !route_nodes.is_empty() {
                &route_nodes[1..]
            } else {
                &route_nodes[..]
            };
            for _ in duplicate_check_nodes {
                if !budget.is_none_or(|budget| budget.step().is_ok()) {
                    return Err(RoutePublicationError::WorkLimit);
                }
            }
        }

        let edge = &graph.edge_records[edge_index];
        if edge.source_arrowhead_label.is_some() {
            if !budget.is_none_or(|budget| budget.add(route_nodes.len() as u64).is_ok()) {
                return Err(RoutePublicationError::WorkLimit);
            }
            let segment_endpoints =
                super::route_geometry::segment_endpoints_from_ovg_nodes(&snapshot_nodes())
                    .ok_or(RoutePublicationError::OvgNodeIdentityMismatch)?;
            if let Some(label) = crate::internal::labels::positioned_arrowhead_label_for_route(
                edge_index,
                edge,
                &segment_endpoints,
                false,
            ) {
                self.positioned_arrowhead_labels.push(label);
            }
        }
        if edge.target_arrowhead_label.is_some() {
            if !budget.is_none_or(|budget| budget.add(route_nodes.len() as u64).is_ok()) {
                return Err(RoutePublicationError::WorkLimit);
            }
            let segment_endpoints =
                super::route_geometry::segment_endpoints_from_ovg_nodes(&snapshot_nodes())
                    .ok_or(RoutePublicationError::OvgNodeIdentityMismatch)?;
            if let Some(label) = crate::internal::labels::positioned_arrowhead_label_for_route(
                edge_index,
                edge,
                &segment_endpoints,
                true,
            ) {
                self.positioned_arrowhead_labels.push(label);
            }
        }
        let route_id = self.routes.len();
        self.routes
            .push(super::published_route::PublishedRoute::new(
                edge_index,
                snapshot_nodes(),
                ovg_node_ids.map(<[usize]>::to_vec),
                copied_endpoints,
                live.clone(),
            ));
        self.accepted_edge_order.push(edge_index);
        // The Go router indexes every node in `route.OVGNodes`, including the
        // source/target centers. Generated paths retain center ports even when
        // their coordinates coincide with those centers; imported fixed paths
        // instead use the already-interned identity sequence above.
        for point_index in 0..route_nodes.len() {
            if !budget.is_none_or(|budget| budget.step().is_ok()) {
                return Err(RoutePublicationError::WorkLimit);
            }
            let point = point_at(point_index);
            let routes = self
                .point_to_routes
                .entry(interaction_point_key(point))
                .or_default();
            // `pointToRoute` is a map of slices in Go, and addRoute appends
            // for every OVG node occurrence. Repeated occurrences are
            // observable to used-port scans and work accounting, so preserve
            // them instead of treating the value as a set.
            routes.push(route_id);
            if point_index > 0 && point_index < route_nodes.len().saturating_sub(2) {
                #[cfg(test)]
                let segment = [point, point_at(point_index + 1)];
                let route = &self.routes[route_id];
                let retained = super::routed_edge_set::Edge {
                    from: route.node_ref(point_index, route_id, &self.canonical_nodes),
                    to: route.node_ref(point_index + 1, route_id, &self.canonical_nodes),
                    distance: 0.0,
                };
                self.routed_set
                    .add_guarded(
                        retained,
                        budget.map(|guard| guard as &dyn super::routed_edge_set::Work),
                    )
                    .map_err(|_| RoutePublicationError::WorkLimit)?;
                #[cfg(test)]
                self.routed_segments.push(segment);
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        trace_publication("indexed");
        let route = &self.routes[route_id];
        self.static_index
            .mark_route(
                route.len(),
                route_id,
                !edge.is_invisible(),
                |index| route.node_ref(index, route_id, &self.canonical_nodes),
                budget.map(|guard| guard as &dyn super::routed_edge_set::Work),
            )
            .map_err(|_| RoutePublicationError::WorkLimit)?;
        #[cfg(feature = "diagnostic-traces")]
        trace_publication("end");
        if !budget.is_none_or(|budget| budget.check().is_ok()) {
            return Err(RoutePublicationError::WorkLimit);
        }
        Ok(())
    }

    #[cfg(test)]
    fn charge_routed_edge_insert(
        &mut self,
        segment: [Point; 2],
        budget: Option<&LocalRouteSearchWork>,
    ) -> bool {
        self.routed_set
            .add_guarded(
                super::routed_edge_set::Edge::fixture(segment),
                budget.map(|guard| guard as &dyn super::routed_edge_set::Work),
            )
            .is_ok()
    }

    pub(super) fn accepted_edge_indices(&self) -> &[usize] {
        &self.accepted_edge_order
    }

    /// quickRoute reserves and copies every retained Route occurrence.
    fn materialize_quick_route_records(
        &self,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<Vec<(usize, Vec<Point>)>> {
        let mut records = Vec::new();
        for route in &self.routes {
            if let Some(budget) = budget {
                budget.add(route.len() as u64 + 1).ok()?;
            }
            records.push((route.edge, route.segment_endpoints()?));
        }
        Some(records)
    }

    pub(super) fn published_route(
        &self,
        route_id: usize,
    ) -> &super::published_route::PublishedRoute {
        &self.routes[route_id]
    }

    /// Translate `ovgEdgeRouter.findOverlappingRoutes`. The upstream helper
    /// first finds static OVG edges on the candidate's axis, then intersects
    /// the route slices attached to both endpoints. `point_to_routes` is an
    /// append-only map, but the returned route set is deduplicated in first
    /// encounter order exactly like Go's pointer-keyed `routeSet`.
    pub(super) fn overlapping_route_indices(
        &self,
        from: Point,
        to: Point,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<Vec<usize>> {
        let overlaps = self
            .routed_set
            .overlapping(
                &super::routed_edge_set::Edge::value([from, to]),
                budget.map(|guard| guard as &dyn super::routed_edge_set::Work),
            )
            .ok()?;
        let mut result = Vec::new();
        let mut seen = BTreeSet::new();
        for edge in overlaps {
            // findOverlappingRoutes visits each segment returned by the
            // guarded edge-set scan before intersecting its endpoint lists.
            if !budget.is_none_or(|guard| guard.step().is_ok()) {
                return None;
            }
            let from_routes = self
                .point_to_routes
                .get(&interaction_point_key(edge.from.point()))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for route_from in from_routes {
                if !budget.is_none_or(|guard| guard.step().is_ok()) {
                    return None;
                }
                let to_routes = self
                    .point_to_routes
                    .get(&interaction_point_key(edge.to.point()))
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                for route_to in to_routes {
                    if !budget.is_none_or(|guard| guard.step().is_ok()) {
                        return None;
                    }
                    if route_from == route_to && seen.insert(*route_to) {
                        result.push(*route_to);
                    }
                }
            }
        }
        Some(result)
    }

    #[cfg(test)]
    pub(super) fn publication_counts(&self) -> (usize, usize, usize, usize) {
        (
            self.point_to_routes.len(),
            self.routed_segments.len(),
            self.static_index.nearby.len(),
            self.static_index.overlapping.len(),
        )
    }

    fn accepted_route_points(&self, edge_index: usize, route: &[Point]) -> Vec<Point> {
        let mut points = Vec::with_capacity(route.len() + 2);
        if let Some((source, _)) = self.route_centers[edge_index] {
            points.push(source);
        }
        for point in route.iter().copied() {
            if points.last() != Some(&point) {
                points.push(point);
            }
        }
        if let Some((_, target)) = self.route_centers[edge_index]
            && points.last() != Some(&target)
        {
            points.push(target);
        }
        points
    }

    pub(super) fn candidate_arrowhead_label_cost_for_end_guarded(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        route: &[Point],
        is_target: bool,
        nodes: &[NodeId],
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<f64> {
        let Some(candidate) = crate::internal::labels::positioned_arrowhead_label_for_route(
            edge_index,
            &graph.edge_records[edge_index],
            route,
            is_target,
        ) else {
            return Some(0.0);
        };
        for other in &self.positioned_arrowhead_labels {
            if let Some(guard) = budget {
                guard.step().ok()?;
            }
            if candidate.edge_index == other.edge_index && candidate.is_target == other.is_target {
                continue;
            }
            if crate::internal::labels::overlaps(candidate.rect, other.rect, 0.0)
                && candidate.text != other.text
            {
                return Some(f64::INFINITY);
            }
        }
        let mut overlapping_nodes = 0;
        for &node in nodes {
            if let Some(guard) = budget {
                guard.step().ok()?;
            }
            if let Some(rect) = node_rect(graph, node)
                && crate::internal::labels::overlaps(candidate.rect, rect, 5.0)
            {
                overlapping_nodes += 1;
            }
        }
        let mut overlapping_edges = 0;
        for route in &self.routes {
            if let Some(guard) = budget {
                guard.step().ok()?;
            }
            if route.edge == edge_index {
                continue;
            }
            for index in 0..route.len().saturating_sub(1) {
                if let Some(guard) = budget {
                    guard.step().ok()?;
                }
                if crate::internal::labels::positioned_arrowhead_label_overlaps_route(
                    &candidate,
                    &[route.point(index), route.point(index + 1)],
                ) {
                    overlapping_edges += 1;
                    break;
                }
            }
        }
        Some(
            4.0 * graph.turn_cost_value() * overlapping_nodes as f64
                + graph.turn_cost_value() * overlapping_edges as f64,
        )
    }

    pub(super) fn candidate_arrowhead_label_cost_for_end(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        route: &[Point],
        is_target: bool,
    ) -> f64 {
        let Some(candidate) = crate::internal::labels::positioned_arrowhead_label_for_route(
            edge_index,
            &graph.edge_records[edge_index],
            route,
            is_target,
        ) else {
            return 0.0;
        };
        let overlapping_edge_count = self
            .routes
            .iter()
            .filter(|published| {
                published.edge != edge_index
                    && crate::internal::labels::positioned_arrowhead_label_overlaps_route(
                        &candidate,
                        &published.points(),
                    )
            })
            .count();
        crate::internal::labels::positioned_arrowhead_label_cost(
            graph,
            &candidate,
            &self.positioned_arrowhead_labels,
            overlapping_edge_count,
        )
    }

    /// Go arrowheadLabelOverlapPenalty extracts every interior OVG point,
    /// then rebuilds the non-ancestor inventory for this proposed route.
    pub(super) fn candidate_arrowhead_label_cost_for_route_guarded(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        route: &[Point],
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<f64> {
        let edge = &graph.edge_records[edge_index];
        if edge.source_arrowhead_label.is_none() && edge.target_arrowhead_label.is_none() {
            return Some(0.0);
        }
        for _ in route {
            if let Some(guard) = budget {
                guard.step().ok()?;
            }
        }
        let nodes = super::route_search_guard::filter_edge_ancestors(graph, edge_index, budget)?;
        let source = self.candidate_arrowhead_label_cost_for_end_guarded(
            graph, edge_index, route, false, &nodes, budget,
        )?;
        let target = self.candidate_arrowhead_label_cost_for_end_guarded(
            graph, edge_index, route, true, &nodes, budget,
        )?;
        Some(source + target)
    }

    pub(super) fn candidate_arrowhead_label_cost_for_route(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        route: &[Point],
    ) -> f64 {
        self.candidate_arrowhead_label_cost_for_end(graph, edge_index, route, false)
            + self.candidate_arrowhead_label_cost_for_end(graph, edge_index, route, true)
    }

    fn crosses_routed_segment(&self, candidate_segment: [Point; 2]) -> bool {
        self.routed_set
            .intersects(
                &super::routed_edge_set::Edge::value(candidate_segment),
                None,
            )
            .expect("unguarded edge-set query")
    }

    /// Once-per-pop work from search's occupiedEdges inventory and
    /// edgeCanOverlapEdgesGuarded. Multiplicity in pointToRoute is meaningful.
    pub(super) fn charge_occupied_routes(
        &self,
        point: Option<Point>,
        budget: &LocalRouteSearchWork,
    ) -> Option<()> {
        self.charge_occupied_routes_memo(point, budget, &mut None)
    }

    /// `charge_occupied_routes`, leaving the pop's lookup in `memo` for the
    /// same point's hop scoring.
    pub(super) fn charge_occupied_routes_memo<'a>(
        &'a self,
        point: Option<Point>,
        budget: &LocalRouteSearchWork,
        memo: &mut OccupiedMemo<'a>,
    ) -> Option<()> {
        let count = point.map_or(0, |point| self.occupied_routes_memo(point, memo).len());
        for _ in 0..count {
            budget.step().ok()?;
        }
        if count == 0 && crate::internal::cancellation_done().is_some() {
            budget.check().ok()?;
            return Some(());
        }
        for _ in 0..10 {
            budget.add(count as u64).ok()?;
        }
        budget.check().ok()?;
        Some(())
    }

    fn search_clusters_for(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
    ) -> Option<&super::interactions::SearchClusters> {
        self.search_clusters
            .as_ref()
            .filter(|(address, edge, _)| {
                *address == graph as *const ArenaGraph as usize && *edge == edge_index
            })
            .map(|(_, _, clusters)| clusters)
    }

    /// Go appends each Route's GEdge to a reused slice before the overlap
    /// rule; map retained route indices without allocating short inventories.
    fn with_route_edge_ids<R>(&self, routes: &[usize], read: impl FnOnce(&[usize]) -> R) -> R {
        const INLINE: usize = 16;
        if routes.len() <= INLINE {
            let mut ids = [0usize; INLINE];
            for (slot, &route) in ids.iter_mut().zip(routes) {
                *slot = self.routes[route].edge;
            }
            read(&ids[..routes.len()])
        } else {
            let ids = routes
                .iter()
                .map(|&route| self.routes[route].edge)
                .collect::<Vec<_>>();
            read(&ids)
        }
    }

    /// Source ovgEdgeSet.intersectsWithChecked, including charged misses and
    /// the sorted prefix skipped before perpendicular-axis geometry checks.
    fn intersects_routed_edges_guarded(
        &self,
        segment: [Point; 2],
        budget: &LocalRouteSearchWork,
    ) -> Option<bool> {
        self.routed_set
            .intersects_with(&super::routed_edge_set::Edge::value(segment), Some(budget))
            .ok()
    }

    pub(super) fn apply_route_interaction_cost(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        candidate_segment: [Point; 2],
        current_cost: f64,
    ) -> f64 {
        self.apply_route_interaction_cost_guarded(
            graph,
            edge_index,
            candidate_segment,
            current_cost,
            None,
        )
        .expect("unbounded interaction scan")
    }

    pub(super) fn apply_route_interaction_cost_guarded(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        candidate_segment: [Point; 2],
        current_cost: f64,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<f64> {
        self.apply_route_interaction_cost_for_edge(
            graph,
            edge_index,
            candidate_segment,
            current_cost,
            None,
            budget,
        )
    }

    /// `point_to_routes[point]`, reusing `memo` when it holds this point's
    /// key. The map changes only through `&mut self`, so a lookup memoized
    /// earlier in a search remains the answer for an equal key.
    pub(super) fn occupied_routes_memo<'a>(
        &'a self,
        point: Point,
        memo: &mut OccupiedMemo<'a>,
    ) -> &'a [usize] {
        let key = interaction_point_key(point);
        if let Some((cached, routes)) = *memo
            && cached == key
        {
            return routes;
        }
        let routes = self
            .point_to_routes
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        *memo = Some((key, routes));
        routes
    }

    pub(super) fn apply_route_interaction_cost_for_edge(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        candidate_segment: [Point; 2],
        current_cost: f64,
        static_edge: Option<usize>,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<f64> {
        let occupied_edges = self.occupied_routes_memo(candidate_segment[0], &mut None);
        self.apply_route_interaction_cost_with_occupied(
            graph,
            edge_index,
            candidate_segment,
            current_cost,
            static_edge,
            occupied_edges,
            budget,
        )
    }

    /// `apply_route_interaction_cost_for_edge` given `point_to_routes` at
    /// the candidate's first point (see `occupied_routes_memo`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_route_interaction_cost_with_occupied(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        candidate_segment: [Point; 2],
        current_cost: f64,
        static_edge: Option<usize>,
        occupied_edges: &[usize],
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<f64> {
        let adjacent_key = interaction_point_key(candidate_segment[1]);
        let edge_key = static_edge.or_else(|| self.static_index.edge_at(candidate_segment));
        // Accepted-route maps and the tunnel inventory change only through
        // `&mut self` publication, so these reads are poll-independent.
        let adjacent_is_on_route = || self.point_to_routes.contains_key(&adjacent_key);
        // The occupied-route verdict is consumed only by the crossing case of
        // an occupied current point whose adjacent point is off-route. Read
        // it at its original position, before any overlap charge.
        let clusters = self.search_clusters_for(graph, edge_index);
        let occupied_unshareable = !occupied_edges.is_empty()
            && !adjacent_is_on_route()
            && !self.with_route_edge_ids(occupied_edges, |ids| {
                super::interactions::edges_can_overlap_all_for_search_with(
                    graph, edge_index, ids, clusters,
                )
            });
        let overlapping_edges = edge_key
            .and_then(|id| self.static_index.overlapping.get(&id))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let directed = graph.edge_records[edge_index].source_arrow
            != graph.edge_records[edge_index].target_arrow;
        let adjacent_is_tunnel = self.tunnel_points.contains(&adjacent_key);
        let mut opposing_share = false;
        let mut entire_tunnel_share = false;
        for other_index in overlapping_edges {
            let published = &self.routes[*other_index];
            if let Some(budget) = budget {
                budget.reserve_product(published.len() as u64 + 1, 2).ok()?;
            }
            let other = &graph.edge_records[published.edge];
            let opposing_candidate = directed && other.source_arrow != other.target_arrow;
            // Materialize live route points only for a predicate that reads
            // them; neither predicate polls, so skipped reads are unobserved.
            let points = (opposing_candidate || adjacent_is_tunnel).then(|| published.points());
            let points = points.as_deref().unwrap_or(&[]);
            opposing_share = opposing_candidate
                && route_is_opposing_collinear(
                    &points[..points.len().saturating_sub(1)],
                    None,
                    candidate_segment[0],
                    candidate_segment[1],
                );
            #[cfg(feature = "diagnostic-traces")]
            if std::env::var("D2_TRACE_SEARCH_OCCUPIED_EDGE")
                .ok()
                .as_deref()
                == Some(graph.edge_records[edge_index].tala_id.to_string().as_str())
            {
                let points = published.points();
                let bits = |p: Point| {
                    [
                        format!("{:016x}", p.x.to_bits()),
                        format!("{:016x}", p.y.to_bits()),
                    ]
                };
                eprintln!(
                    "{}",
                    serde_json::json!({"event":"search-occupied-route","scope_nodes":graph.node_order.len(),"scope_edges":graph.edge_order.len(),"edge":graph.edge_records[edge_index].tala_id,"flavor":self.trace_flavor,"other":other.tala_id,"nodes":published.len(),"points":points.iter().copied().map(bits).collect::<Vec<_>>(),"opposing":opposing_share,"from":bits(candidate_segment[0]),"to":bits(candidate_segment[1]),"work":budget.map(LocalRouteSearchWork::used)})
                );
            }
            if opposing_share {
                break;
            }
            entire_tunnel_share = adjacent_is_tunnel && {
                let interior_points = &points
                    [1.min(points.len())..points.len().saturating_sub(1).max(1.min(points.len()))];
                route_is_entire_collinear(
                    interior_points,
                    candidate_segment[0],
                    candidate_segment[1],
                ) || route_is_entire_collinear(
                    interior_points,
                    candidate_segment[1],
                    candidate_segment[0],
                )
            };
            if entire_tunnel_share {
                break;
            }
        }
        if !opposing_share
            && !entire_tunnel_share
            && let Some(budget) = budget
        {
            budget.charge_overlap_checks(overlapping_edges.len()).ok()?;
        }
        let prohibited_share = opposing_share
            || entire_tunnel_share
            || (!overlapping_edges.is_empty()
                && !self.with_route_edge_ids(overlapping_edges, |ids| {
                    super::interactions::edges_can_overlap_all_for_search_with(
                        graph, edge_index, ids, clusters,
                    )
                }))
            || (occupied_edges.is_empty()
                && edge_key.is_some_and(|id| self.static_index.nearby.contains(&id))
                && !adjacent_is_on_route());
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_SEARCH_FILTERS") {
            eprintln!(
                "{}",
                serde_json::json!({
                    "diagnostic": "route-interaction-input",
                    "edge": graph.edge_records[edge_index].tala_id,
                    "flavor": self.trace_flavor,
                    "from": [candidate_segment[0].x, candidate_segment[0].y],
                    "to": [candidate_segment[1].x, candidate_segment[1].y],
                    "opposing": opposing_share,
                    "entire_tunnel": entire_tunnel_share,
                    "prohibited": prohibited_share,
                    "nearby": edge_key.is_some_and(|id| self.static_index.nearby.contains(&id)),
                    "occupied": occupied_edges.iter().map(|i| graph.edge_records[self.routes[*i].edge].tala_id).collect::<Vec<_>>(),
                    "adjacent_on_route": adjacent_is_on_route(),
                    "overlapping": overlapping_edges.iter().map(|i| serde_json::json!({
                        "edge": graph.edge_records[self.routes[*i].edge].tala_id,
                        "route": self.routes[*i].segment_endpoints().map(|r| r.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>()),
                    })).collect::<Vec<_>>(),
                })
            );
        }
        if prohibited_share {
            return Some(
                if graph.edge_records[edge_index].is_between_table_columns() {
                    1_000_000.0
                } else {
                    10_000_000.0
                },
            );
        }
        // Crossing reads the retained routed-edge set independently of the
        // static OVG edge object's nearby and overlap entries.
        let crosses_occupied_point = occupied_unshareable;
        let crosses_routed_segment = if crosses_occupied_point {
            false
        } else {
            match budget {
                Some(budget) => self.intersects_routed_edges_guarded(candidate_segment, budget)?,
                None => self.crosses_routed_segment(candidate_segment),
            }
        };
        Some(if crosses_occupied_point || crosses_routed_segment {
            current_cost + graph.routing_costs.0
        } else {
            current_cost
        })
    }

    pub(super) fn apply_label_obstacle_cost(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        candidate_segment: [Point; 2],
        current_cost: f64,
    ) -> f64 {
        self.apply_label_obstacle_cost_guarded(
            graph,
            edge_index,
            candidate_segment,
            current_cost,
            None,
        )
        .expect("unbounded label scan")
    }

    /// D2 search charges every positioned label visited, including misses.
    pub(super) fn apply_label_obstacle_cost_guarded(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        candidate_segment: [Point; 2],
        current_cost: f64,
        budget: Option<&LocalRouteSearchWork>,
    ) -> Option<f64> {
        let edge = &graph.edge_records[edge_index];
        let mut cost = current_cost;
        for label in &self.positioned_arrowhead_labels {
            if let Some(budget) = budget {
                budget.step().ok()?;
            }
            if super::route_geometry::recovered_segment_intersects_box(
                candidate_segment[0],
                candidate_segment[1],
                label.rect,
            ) {
                cost += 10_000_000.0;
                break;
            }
        }
        for &(node, rect) in &self.node_label_rects {
            if node == edge.from || node == edge.to {
                continue;
            }
            if let Some(budget) = budget {
                budget.step().ok()?;
            }
            if super::route_geometry::recovered_segment_intersects_box(
                candidate_segment[0],
                candidate_segment[1],
                rect,
            ) {
                cost += graph.routing_costs.1;
            }
        }
        Some(cost)
    }

    pub(super) fn apply_cost(
        &self,
        graph: &ArenaGraph,
        edge_index: usize,
        candidate_segment: [Point; 2],
        current_cost: f64,
    ) -> f64 {
        let cost =
            self.apply_route_interaction_cost(graph, edge_index, candidate_segment, current_cost);
        self.apply_label_obstacle_cost(graph, edge_index, candidate_segment, cost)
    }
}

pub(super) fn route_interaction_cost(
    graph: &ArenaGraph,
    edge_index: usize,
    candidate: &[Point],
    routed: &[Vec<Point>],
) -> f64 {
    let mut cost = 0.0;
    for candidate_segment in candidate.windows(2) {
        let current = candidate_segment[0];
        let adjacent = candidate_segment[1];
        let occupied_edges = routed
            .iter()
            .enumerate()
            .filter_map(|(other_index, route)| route.contains(&current).then_some(other_index))
            .collect::<Vec<_>>();
        let adjacent_is_on_route = routed.iter().any(|route| route.contains(&adjacent));
        let occupied_are_shareable = super::interactions::edges_can_overlap_all_for_search(
            graph,
            edge_index,
            &occupied_edges,
        );

        // addRoute registers a route on the exact static OVG edges it covers,
        // and separately marks OVG edges within five units as nearby. Search
        // consults those edge-keyed maps; it does not compare the candidate
        // against every geometric segment as the former Rust approximation
        // did. Rust routes omit TALA's synthetic center nodes, so every stored
        // segment here corresponds to the internal OVG-node range addRoute
        // indexes.
        let mut overlapping_edges = Vec::new();
        let mut has_nearby_edge = false;
        let mut opposing_share = false;
        let mut intersects_edge_set = false;
        for (other_index, route) in routed.iter().enumerate() {
            for routed_segment in route.windows(2) {
                if let Some(axis_distance) = parallel_axis_overlap(
                    [candidate_segment[0], candidate_segment[1]],
                    [routed_segment[0], routed_segment[1]],
                ) {
                    if axis_distance <= 5.0 {
                        has_nearby_edge = true;
                    }
                    if axis_distance == 0.0 {
                        overlapping_edges.push(other_index);
                        let directed = graph.edge_records[edge_index].source_arrow
                            != graph.edge_records[edge_index].target_arrow;
                        let other_directed = graph.edge_records[other_index].source_arrow
                            != graph.edge_records[other_index].target_arrow;
                        opposing_share |= directed
                            && other_directed
                            && route_is_opposing_collinear(
                                route,
                                Some((
                                    graph.nodes[graph.edge_records[other_index].from.0 as usize]
                                        .rect
                                        .center(),
                                    graph.nodes[graph.edge_records[other_index].to.0 as usize]
                                        .rect
                                        .center(),
                                )),
                                candidate_segment[0],
                                candidate_segment[1],
                            );
                    }
                }
                if segments_intersect(
                    candidate_segment[0],
                    candidate_segment[1],
                    routed_segment[0],
                    routed_segment[1],
                ) && !candidate_segment
                    .iter()
                    .any(|point| routed_segment.contains(point))
                {
                    intersects_edge_set = true;
                }
            }
        }
        overlapping_edges.sort_unstable();
        overlapping_edges.dedup();

        let prohibited_share = opposing_share
            || (!overlapping_edges.is_empty()
                && !super::interactions::edges_can_overlap_all_for_search(
                    graph,
                    edge_index,
                    &overlapping_edges,
                ))
            || (occupied_edges.is_empty() && !adjacent_is_on_route && has_nearby_edge);
        if prohibited_share {
            cost += if graph.edge_records[edge_index].is_between_table_columns() {
                1_000_000.0
            } else {
                10_000_000.0
            };
            continue;
        }

        let crosses_occupied_point =
            !occupied_edges.is_empty() && !adjacent_is_on_route && !occupied_are_shareable;
        if crosses_occupied_point || intersects_edge_set {
            cost += graph.routing_costs.0;
        }
    }
    cost
}

fn point_on_segment(point: Point, segment: &[Point]) -> bool {
    let [first, second] = segment else {
        return false;
    };
    if first.x == second.x {
        point.x == first.x && first.y.min(second.y) <= point.y && point.y <= first.y.max(second.y)
    } else if first.y == second.y {
        point.y == first.y && first.x.min(second.x) <= point.x && point.x <= first.x.max(second.x)
    } else {
        false
    }
}

fn connect_port_to_point(port: Port, point: Point) -> Option<Vec<Point>> {
    let outward = outward(port.side);
    let delta = Point {
        x: point.x - port.point.x,
        y: point.y - port.point.y,
    };
    if delta.x * outward.x + delta.y * outward.y <= 0.0 {
        return None;
    }
    let bend = match port.side {
        PortSide::Left | PortSide::Right => Point {
            x: point.x,
            y: port.point.y,
        },
        PortSide::Top | PortSide::Bottom => Point {
            x: port.point.x,
            y: point.y,
        },
    };
    Some(simplify_route(vec![port.point, bend, point]))
}

fn port_segment_join_points(port: Port, segment: &[Point]) -> Vec<Point> {
    let [first, second] = segment else {
        return Vec::new();
    };
    let direction = outward(port.side);
    let stub = Point {
        x: port.point.x + direction.x * 30.0,
        y: port.point.y + direction.y * 30.0,
    };
    let mut points = Vec::with_capacity(2);

    match port.side {
        PortSide::Left | PortSide::Right => {
            if first.x == second.x {
                points.push(Point {
                    x: first.x,
                    y: port.point.y,
                });
            } else if first.y == second.y {
                points.push(Point {
                    x: stub.x,
                    y: first.y,
                });
            }
        }
        PortSide::Top | PortSide::Bottom => {
            if first.y == second.y {
                points.push(Point {
                    x: port.point.x,
                    y: first.y,
                });
            } else if first.x == second.x {
                points.push(Point {
                    x: first.x,
                    y: stub.y,
                });
            }
        }
    }

    points.retain(|point| {
        point_on_segment(*point, segment)
            && connect_port_to_point(port, *point).is_some()
            && *point != *first
            && *point != *second
    });
    points.dedup();
    points
}

fn shared_route_candidates(
    graph: &ArenaGraph,
    edge_index: usize,
    source: Port,
    target: Port,
    boxes: &BTreeMap<NodeId, Rect>,
    routed: &[Vec<Point>],
) -> Vec<Vec<Point>> {
    let edge = &graph.edge_records[edge_index];
    let mut candidates = Vec::new();

    for (other_index, other_route) in routed.iter().enumerate() {
        if other_route.len() < 2
            || other_index == edge_index
            || !edges_can_overlap(graph, edge_index, other_index)
        {
            continue;
        }
        let other = &graph.edge_records[other_index];

        let oriented_to_target = if other.to == edge.to && other_route.last() == Some(&target.point)
        {
            Some(other_route.clone())
        } else if other.from == edge.to && other_route.first() == Some(&target.point) {
            Some(other_route.iter().rev().copied().collect())
        } else {
            None
        };
        if let Some(route_to_target) = oriented_to_target {
            for join_index in 0..route_to_target.len() - 1 {
                let join = route_to_target[join_index];
                let Some(mut candidate) = connect_port_to_point(source, join) else {
                    continue;
                };
                candidate.extend_from_slice(&route_to_target[join_index + 1..]);
                let candidate = simplify_route(candidate);
                if route_is_clear(graph, &candidate, boxes, edge.from, edge.to) {
                    candidates.push(candidate);
                }
            }
            for (segment_index, segment) in route_to_target.windows(2).enumerate() {
                for join in port_segment_join_points(source, segment) {
                    let Some(mut candidate) = connect_port_to_point(source, join) else {
                        continue;
                    };
                    candidate.extend_from_slice(&route_to_target[segment_index + 1..]);
                    let candidate = simplify_route(candidate);
                    if route_is_clear(graph, &candidate, boxes, edge.from, edge.to) {
                        candidates.push(candidate);
                    }
                }
            }
        }

        let oriented_from_source =
            if other.from == edge.from && other_route.first() == Some(&source.point) {
                Some(other_route.clone())
            } else if other.to == edge.from && other_route.last() == Some(&source.point) {
                Some(other_route.iter().rev().copied().collect())
            } else {
                None
            };
        if let Some(route_from_source) = oriented_from_source {
            for join_index in 1..route_from_source.len() {
                let join = route_from_source[join_index];
                let Some(mut connector) = connect_port_to_point(target, join) else {
                    continue;
                };
                connector.reverse();
                let mut candidate = route_from_source[..join_index].to_vec();
                candidate.append(&mut connector);
                let candidate = simplify_route(candidate);
                if route_is_clear(graph, &candidate, boxes, edge.from, edge.to) {
                    candidates.push(candidate);
                }
            }
            for (segment_index, segment) in route_from_source.windows(2).enumerate() {
                for join in port_segment_join_points(target, segment) {
                    let Some(mut connector) = connect_port_to_point(target, join) else {
                        continue;
                    };
                    connector.reverse();
                    let mut candidate = route_from_source[..=segment_index].to_vec();
                    candidate.extend(connector);
                    let candidate = simplify_route(candidate);
                    if route_is_clear(graph, &candidate, boxes, edge.from, edge.to) {
                        candidates.push(candidate);
                    }
                }
            }
        }
    }

    candidates
}

#[derive(Clone)]
struct RouteOvgContext {
    // Go's routers retain the same *OVG (coordinator.go, ovg_edge_router.go).
    // These adapter views describe that finished graph and are shared too.
    boxes: std::sync::Arc<BTreeMap<NodeId, Rect>>,
    obstacle_node_order: std::sync::Arc<[NodeId]>,
    node_ports: std::sync::Arc<BTreeMap<NodeId, Vec<Port>>>,
    /// Flavors only read the finished OVG; its node and edge objects were
    /// already shared by every handle copy, so flavors share one graph.
    visibility: std::sync::Arc<super::visibility::VisibilityGraph>,
    existing_routes: std::sync::Arc<[super::ovg_import::ExistingRoute]>,
}

fn build_route_ovg_context(
    graph: &ArenaGraph,
    scope_nodes: &[NodeId],
    nearby_nodes: &[NodeId],
    ordered_edges: &[usize],
    fixed_routes: Option<&[Vec<Point>]>,
) -> RouteOvgContext {
    build_route_ovg_context_checked(
        graph,
        scope_nodes,
        nearby_nodes,
        ordered_edges,
        fixed_routes,
        None,
    )
    .expect("unbudgeted OVG construction cannot fail")
}

fn build_route_ovg_context_checked(
    graph: &ArenaGraph,
    scope_nodes: &[NodeId],
    nearby_nodes: &[NodeId],
    ordered_edges: &[usize],
    fixed_routes: Option<&[Vec<Point>]>,
    ovg_budget: Option<&super::route_search_guard::OvgBuildBudget>,
) -> Result<RouteOvgContext, String> {
    let mut working = graph.clone();
    build_route_ovg_context_mut(
        &mut working,
        scope_nodes,
        nearby_nodes,
        ordered_edges,
        fixed_routes,
        ovg_budget,
    )
}

fn build_route_ovg_context_mut(
    graph: &mut ArenaGraph,
    scope_nodes: &[NodeId],
    nearby_nodes: &[NodeId],
    ordered_edges: &[usize],
    fixed_routes: Option<&[Vec<Point>]>,
    ovg_budget: Option<&super::route_search_guard::OvgBuildBudget>,
) -> Result<RouteOvgContext, String> {
    if let Some(budget) = ovg_budget {
        budget
            .check()
            .map_err(|_| "OVG construction cancelled".to_owned())?;
    }
    let mut ordinary_nodes = nearby_nodes.to_vec();
    let mut hierarchies = Vec::new();
    let mut seen = BTreeSet::new();
    for &node in scope_nodes {
        if let Some(budget) = ovg_budget {
            budget
                .reserve_work(1)
                .map_err(|_| "OVG classification cancelled".to_owned())?;
        }
        if let Some(hierarchy) = graph.nodes[node.0 as usize].hierarchy.clone() {
            if seen.insert(hierarchy.record.allocation_key()) {
                hierarchies.push(
                    super::ovg_hierarchy::build(graph, &hierarchy.record, ovg_budget).map_err(
                        |_| {
                            "OVG hierarchy construction cancelled or exceeded its resource limit"
                                .to_owned()
                        },
                    )?,
                );
            }
        } else {
            ordinary_nodes.push(node);
        }
    }
    let mut obstacle_node_order = ordinary_nodes.clone();
    for hierarchy in &hierarchies {
        obstacle_node_order.extend(&hierarchy.nodes);
    }
    let routing_nodes = ovg_routing_nodes(scope_nodes, nearby_nodes);
    let mut port_build = super::ovg_ports::PortBuild::default();
    port_build
        .add_ports(graph, &ordinary_nodes, ovg_budget)
        .map_err(|_| "OVG port construction cancelled or exceeded its resource limit".to_owned())?;
    let mut node_ports = port_build.node_ports();
    // The graph stores live node pointers throughout source OVG construction.
    // Refresh this numerical view after callbacks during port creation.
    let boxes: BTreeMap<_, _> = scope_nodes
        .iter()
        .chain(nearby_nodes)
        .filter_map(|&id| {
            Some((
                id,
                Rect {
                    origin: graph.position(id)?,
                    size: graph.nodes[id.0 as usize].rect.size,
                },
            ))
        })
        .collect();
    // OVG.addPorts ranges over the temporary SplitSubgraphs Graph.Nodes slice,
    // whose order is `scope_nodes`, independently of the owning arena's
    // global Graph.Nodes order.
    // NewOVGFromGraph prepends nearbyNodes to the routed graph's Nodes before
    // addPorts. Preserve that ownership boundary: isolated nearby nodes still
    // contribute port axes and intersections even though their own edges are
    // routed in another SplitSubgraphs result.
    let mut port_node_order = ovg_port_node_order(
        scope_nodes,
        &routing_nodes,
        nearby_nodes,
        node_ports.keys().copied(),
    );
    for node in node_ports.keys().copied() {
        if !port_node_order.contains(&node) {
            port_node_order.push(node);
        }
    }
    // buildOVG includes trees for routeAdditionalEdges as well.
    let tree_node_order = scope_nodes;
    let mut visibility = super::visibility::VisibilityGraph::from_ports_ordered_checked(
        graph,
        &boxes,
        &obstacle_node_order,
        scope_nodes,
        tree_node_order,
        port_node_order.into_iter().flat_map(|node| {
            node_ports[&node]
                .iter()
                .copied()
                .map(move |port| (node, port))
        }),
        ovg_budget,
        Some(&mut port_build),
        Some(&hierarchies),
    )?;
    // Construction moved the prepared build into the canonical view.
    drop(port_build);
    node_ports = visibility.canonical_node_ports();
    // Source existingRoutes is ordered by Graph.Edges occurrences. A repeated
    // edge pointer creates another Route; a one-node import creates none.
    let mut existing_routes = Vec::new();
    #[cfg(feature = "diagnostic-traces")]
    trace_ovg_import_stage(graph, &visibility, &node_ports, "base");
    if fixed_routes.is_some() {
        let mut imported = Vec::new();
        for edge_id in graph.edge_order.header().iter_slots() {
            if let Some(budget) = ovg_budget {
                budget.reserve_work(1).map_err(|_| {
                    "OVG fixed-route import cancelled or exceeded its work limit".to_owned()
                })?;
            }
            let edge_id =
                edge_id.expect("runtime error: invalid memory address or nil pointer dereference");
            let edge_index = edge_id.0 as usize;
            if ordered_edges.contains(&edge_index) {
                continue;
            }
            let route = visibility
                .import_existing_edge_route(graph, edge_id, ovg_budget)
                .map_err(|_| {
                    "OVG fixed-route import cancelled or exceeded its resource limit".to_owned()
                })?;
            imported.push((edge_id, route));
        }
        for route in visibility
            .publish_existing_edge_routes(imported)
            .into_iter()
            .flatten()
        {
            super::ovg_ports::push_pointer(&mut existing_routes, route);
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    trace_ovg_import_stage(graph, &visibility, &node_ports, "imported");
    Ok(RouteOvgContext {
        boxes: boxes.into(),
        obstacle_node_order: obstacle_node_order.into(),
        node_ports: node_ports.into(),
        visibility: visibility.into_shared(),
        existing_routes: existing_routes.into(),
    })
}

/// Account for `newOVGEdgeRouterWithWorkLimit` in source order. TALA charges
/// the flavor's edge inventory, its cluster-membership scan, both endpoint
/// adjacency lists for every candidate edge, and the two stable edge sorts
/// before it publishes pre-existing routes. Keep each source `step` separate
/// so cancellation and limit boundaries land on the same operation.
fn charge_route_router_construction(
    graph: &ArenaGraph,
    router_edges: &[usize],
    budget: &LocalRouteSearchWork,
) -> Result<(), ()> {
    budget.add(router_edges.len() as u64)?;
    for cluster in &graph.clusters {
        budget.step()?;
        for _ in &cluster.members {
            budget.step()?;
        }
    }
    for &edge_index in router_edges {
        budget.step()?;
        let edge = &graph.edge_records[edge_index];
        budget.reserve_sum(
            graph.nodes[edge.from.0 as usize].edges.len() as u64,
            graph.nodes[edge.to.0 as usize].edges.len() as u64,
        )?;
    }
    budget.reserve_sort(router_edges.len())?;
    budget.reserve_sort(router_edges.len())
}

type RouteFlavorResponse = (
    Vec<Vec<Point>>,
    usize,
    f64,
    Vec<Option<(Port, Port)>>,
    Option<crate::LayoutError>,
    Vec<usize>,
    Vec<super::published_route::PublishedRoute>,
);

struct PreparedRouteFlavor {
    boxes: std::sync::Arc<BTreeMap<NodeId, Rect>>,
    obstacle_node_order: std::sync::Arc<[NodeId]>,
    node_ports: std::sync::Arc<BTreeMap<NodeId, Vec<Port>>>,
    visibility: std::sync::Arc<super::visibility::VisibilityGraph>,
    interaction_index: RouteInteractionIndex,
    routes: Vec<Vec<Point>>,
    selected_ports: Vec<Option<(Port, Port)>>,
    preexisting_route_order: Vec<usize>,
    router_edges: Vec<usize>,
}

fn route_edges_in_order(
    graph: &ArenaGraph,
    scope_nodes: &[NodeId],
    nearby_nodes: &[NodeId],
    ordered_edges: &[usize],
    construction_order: &[usize],
    trace_flavor: &'static str,
    fixed_routes: Option<&[Vec<Point>]>,
    shared_context: Option<&RouteOvgContext>,
    ovg_budget: Option<&super::route_search_guard::OvgBuildBudget>,
    budget: Option<&LocalRouteSearchWork>,
) -> RouteFlavorResponse {
    match prepare_route_flavor(
        graph,
        scope_nodes,
        nearby_nodes,
        ordered_edges,
        construction_order,
        trace_flavor,
        fixed_routes,
        shared_context,
        ovg_budget,
        budget,
    ) {
        Ok(prepared) => generate_route_flavor(
            graph,
            scope_nodes,
            ordered_edges,
            trace_flavor,
            fixed_routes,
            budget,
            prepared,
        ),
        Err(response) => response,
    }
}

fn prepare_route_flavor(
    graph: &ArenaGraph,
    scope_nodes: &[NodeId],
    nearby_nodes: &[NodeId],
    ordered_edges: &[usize],
    construction_order: &[usize],
    trace_flavor: &str,
    fixed_routes: Option<&[Vec<Point>]>,
    shared_context: Option<&RouteOvgContext>,
    ovg_budget: Option<&super::route_search_guard::OvgBuildBudget>,
    budget: Option<&LocalRouteSearchWork>,
) -> Result<PreparedRouteFlavor, RouteFlavorResponse> {
    prepare_route_flavor_owned(
        graph,
        scope_nodes,
        nearby_nodes,
        ordered_edges,
        construction_order,
        trace_flavor,
        fixed_routes,
        shared_context.cloned(),
        ovg_budget,
        budget,
    )
}

/// `prepare_route_flavor` taking its own copy of the shared OVG context, so
/// the last flavor can take the coordinator's context instead of a clone.
fn prepare_route_flavor_owned(
    graph: &ArenaGraph,
    scope_nodes: &[NodeId],
    nearby_nodes: &[NodeId],
    ordered_edges: &[usize],
    construction_order: &[usize],
    trace_flavor: &str,
    fixed_routes: Option<&[Vec<Point>]>,
    shared_context: Option<RouteOvgContext>,
    ovg_budget: Option<&super::route_search_guard::OvgBuildBudget>,
    budget: Option<&LocalRouteSearchWork>,
) -> Result<PreparedRouteFlavor, RouteFlavorResponse> {
    let context = if let Some(shared_context) = shared_context {
        shared_context
    } else {
        match build_route_ovg_context_checked(
            graph,
            scope_nodes,
            nearby_nodes,
            ordered_edges,
            fixed_routes,
            ovg_budget,
        ) {
            Ok(context) => context,
            Err(error) => {
                return Err((
                    vec![Vec::new(); graph.edge_records.len()],
                    ordered_edges.len(),
                    f64::INFINITY,
                    vec![None; graph.edge_records.len()],
                    Some(crate::LayoutError::BadRouteState(error)),
                    Vec::new(),
                    Vec::new(),
                ));
            }
        }
    };
    let RouteOvgContext {
        boxes,
        obstacle_node_order,
        node_ports,
        visibility,
        existing_routes,
    } = context;
    let mut interaction_index =
        RouteInteractionIndex::new_with_node_labels(&visibility, graph, fixed_routes.is_some());
    #[cfg(feature = "diagnostic-traces")]
    {
        interaction_index.trace_flavor = Some(trace_flavor.to_owned());
    }
    let mut routes = fixed_routes
        .map(<[Vec<Point>]>::to_vec)
        .unwrap_or_else(|| vec![Vec::new(); graph.edge_records.len()]);
    let mut selected_ports = vec![None::<(Port, Port)>; graph.edge_records.len()];

    if fixed_routes.is_some() {
        if let Some(budget) = budget
            && charge_route_router_construction(graph, construction_order, budget).is_err()
        {
            return Err((
                routes,
                ordered_edges.len(),
                f64::INFINITY,
                selected_ports,
                Some(crate::LayoutError::BadRouteState(
                    "route-search work limit exceeded while preparing router".into(),
                )),
                Vec::new(),
                Vec::new(),
            ));
        }
        for existing in existing_routes.iter() {
            let edge_index = existing.edge.0 as usize;
            if budget.is_some_and(|budget| budget.step().is_err()) {
                return Err((
                    routes,
                    ordered_edges.len(),
                    f64::INFINITY,
                    selected_ports,
                    Some(crate::LayoutError::BadRouteState(
                        "route-search work limit exceeded while loading existing route".into(),
                    )),
                    Vec::new(),
                    Vec::new(),
                ));
            }
            let edge = &graph.edge_records[edge_index];
            let source = node_ports
                .get(&edge.from)
                .and_then(|ports| {
                    ports
                        .iter()
                        .find(|port| port.point == existing.endpoints[0])
                })
                .copied()
                .unwrap_or(Port {
                    point: existing.endpoints[0],
                    direction: PortSide::Top,
                    side: PortSide::Top,
                    index: usize::MAX,
                    tunnel: None,
                    is_center: false,
                });
            let target = node_ports
                .get(&edge.to)
                .and_then(|ports| {
                    ports
                        .iter()
                        .find(|port| port.point == existing.endpoints[1])
                })
                .copied()
                .unwrap_or(Port {
                    point: existing.endpoints[1],
                    direction: PortSide::Top,
                    side: PortSide::Top,
                    index: usize::MAX,
                    tunnel: None,
                    is_center: false,
                });
            selected_ports[edge_index] = Some((source, target));
            let publish_result = interaction_index.add_imported_route(
                graph,
                &visibility,
                existing,
                trace_flavor,
                budget,
            );
            if let Err(error) = publish_result {
                let failure = match error {
                    RoutePublicationError::WorkLimit => crate::LayoutError::BadRouteState(
                        "route publication work limit exceeded".to_owned(),
                    ),
                    RoutePublicationError::OvgNodeIdentityMismatch => {
                        crate::LayoutError::BadRouteState(format!(
                            "OVG node identity count did not match route {}",
                            route_edge_debug_id(graph, edge_index)
                        ))
                    }
                    RoutePublicationError::DuplicateOvgNode { index, point } => {
                        crate::LayoutError::DuplicateRouteOvgNode {
                            index,
                            point,
                            edge: route_edge_debug_id(graph, edge_index),
                        }
                    }
                };
                return Err((
                    routes,
                    ordered_edges.len(),
                    f64::INFINITY,
                    selected_ports,
                    Some(failure),
                    Vec::new(),
                    Vec::new(),
                ));
            }
        }
    }
    // TALA routes sentinel and loop edges into `existingRoutes` while walking
    // Graph.Nodes, then constructs each flavor router from the remaining
    // edges. Keep that source order and defer guarded publication until after
    // the constructor inventory has been charged.
    let mut preexisting_route_order = Vec::new();
    let mut preexisting_route_node_ids = BTreeMap::new();
    let mut tree_port_arrowheads = BTreeMap::<usize, BTreeSet<String>>::new();
    if fixed_routes.is_none() {
        for node in scope_nodes.iter().copied() {
            if let Some(tree) = graph.tree_routing_nodes.get(&node) {
                // `routeEdgesWithResourceGuards` uses an if/else here: a node
                // with a NodeToTree record does not also contribute loop
                // pre-routes, even when its sentinel edge is not routable.
                if !graph.is_tree_edge(tree.sentinel_edge) {
                    continue;
                }
                let edge = &graph.edge_records[tree.sentinel_edge.0 as usize];
                let parent_arrowhead = if edge.from == node {
                    &edge.target_arrowhead
                } else {
                    &edge.source_arrowhead
                };
                let Some(mut route) = tree_edges::route_tree_edge(graph, node, &boxes, |port| {
                    visibility
                        .port_index(tree.parent, port.point)
                        .and_then(|identity| tree_port_arrowheads.get(&identity))
                        .is_none_or(|used| {
                            used.is_empty()
                                || used.contains(parent_arrowhead.as_deref().unwrap_or_default())
                        })
                }) else {
                    continue;
                };
                let Some(expanded_ids) = visibility.s_shape_route_indices(
                    route.points[0],
                    route.source_midpoint,
                    route.target_midpoint,
                    *route.points.last().unwrap(),
                    route.route_orientation,
                ) else {
                    continue;
                };
                let edge_index = tree.sentinel_edge.0 as usize;
                let edge = &graph.edge_records[edge_index];
                let (Some(source_center), Some(target_center)) = (
                    visibility.center_index(edge.from),
                    visibility.center_index(edge.to),
                ) else {
                    continue;
                };
                let mut node_ids = Vec::with_capacity(expanded_ids.len() + 2);
                node_ids.push(source_center);
                node_ids.extend(expanded_ids.iter().copied());
                node_ids.push(target_center);
                for (identity, arrowhead) in [
                    (expanded_ids[0], &edge.source_arrowhead),
                    (*expanded_ids.last().unwrap(), &edge.target_arrowhead),
                ] {
                    tree_port_arrowheads
                        .entry(identity)
                        .or_default()
                        .insert(arrowhead.clone().unwrap_or_default());
                }
                route.points = expanded_ids
                    .iter()
                    .map(|index| visibility.point(*index))
                    .collect();
                preexisting_route_node_ids.insert(edge_index, node_ids);
                selected_ports[edge_index] = Some((route.source_port, route.target_port));
                routes[edge_index] = route.points;
                preexisting_route_order.push(edge_index);
                continue;
            }
            for (edge_index, route, consumed) in
                crate::internal::routing::self_loop_routes(graph, node, boxes[&node])
            {
                if !ordered_edges.contains(&edge_index) {
                    continue;
                }
                if crate::internal::trace_env_enabled("TALA_TRACE_LOOP_ROUTES") {
                    eprintln!(
                        "LOOP_ROUTE_RUST id={} edge={} route={:?}",
                        graph.nodes[node.0 as usize].tala_id,
                        graph.edge_records[edge_index].tala_id,
                        route
                    );
                }
                if let [source, target] = consumed.as_slice() {
                    selected_ports[edge_index] = Some((*source, *target));
                }
                let Some(center_index) =
                    visibility.center_index(graph.edge_records[edge_index].from)
                else {
                    continue;
                };
                let mut node_ids = Vec::with_capacity(route.len() + 2);
                node_ids.push(center_index);
                node_ids.extend((0..route.len()).map(|index| usize::MAX - index));
                node_ids.push(center_index);
                preexisting_route_node_ids.insert(edge_index, node_ids);
                routes[edge_index] = route.clone();
                preexisting_route_order.push(edge_index);
            }
        }
    }

    let preexisting_edges = preexisting_route_order
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let router_edges = construction_order
        .iter()
        .copied()
        .filter(|edge_index| !preexisting_edges.contains(edge_index))
        .collect::<Vec<_>>();
    if fixed_routes.is_none()
        && let Some(budget) = budget
        && charge_route_router_construction(graph, &router_edges, budget).is_err()
    {
        return Err((
            routes,
            router_edges.len(),
            f64::INFINITY,
            selected_ports,
            Some(crate::LayoutError::BadRouteState(
                "route-search work limit exceeded while preparing router".into(),
            )),
            #[cfg(feature = "diagnostic-traces")]
            preexisting_route_order.clone(),
            #[cfg(not(feature = "diagnostic-traces"))]
            Vec::new(),
            Vec::new(),
        ));
    }
    for edge_index in preexisting_route_order.iter().copied() {
        if budget.is_some_and(|budget| budget.step().is_err()) {
            return Err((
                routes,
                router_edges.len(),
                f64::INFINITY,
                selected_ports,
                Some(crate::LayoutError::BadRouteState(
                    "route-search work limit exceeded while loading existing route".into(),
                )),
                #[cfg(feature = "diagnostic-traces")]
                preexisting_route_order.clone(),
                #[cfg(not(feature = "diagnostic-traces"))]
                Vec::new(),
                Vec::new(),
            ));
        }
        let published = if let Some(node_ids) = preexisting_route_node_ids.get(&edge_index) {
            interaction_index
                .add_generated_route_with_node_ids(
                    graph,
                    edge_index,
                    &routes[edge_index],
                    node_ids,
                    Some(&visibility),
                    trace_flavor,
                    budget,
                )
                .is_ok()
        } else {
            interaction_index.add_route_with_budget(
                graph,
                edge_index,
                &routes[edge_index],
                trace_flavor,
                budget,
            )
        };
        if !published {
            return Err((
                routes,
                router_edges.len(),
                f64::INFINITY,
                selected_ports,
                Some(crate::LayoutError::BadRouteState(
                    "route publication work limit exceeded".into(),
                )),
                #[cfg(feature = "diagnostic-traces")]
                preexisting_route_order.clone(),
                #[cfg(not(feature = "diagnostic-traces"))]
                Vec::new(),
                Vec::new(),
            ));
        }
    }

    Ok(PreparedRouteFlavor {
        boxes,
        obstacle_node_order,
        node_ports,
        visibility,
        interaction_index,
        routes,
        selected_ports,
        preexisting_route_order,
        router_edges,
    })
}

fn generate_route_flavor(
    graph: &ArenaGraph,
    _scope_nodes: &[NodeId],
    ordered_edges: &[usize],
    trace_flavor: &'static str,
    fixed_routes: Option<&[Vec<Point>]>,
    budget: Option<&LocalRouteSearchWork>,
    prepared: PreparedRouteFlavor,
) -> RouteFlavorResponse {
    let PreparedRouteFlavor {
        boxes,
        obstacle_node_order,
        mut node_ports,
        visibility,
        mut interaction_index,
        mut routes,
        mut selected_ports,
        preexisting_route_order,
        router_edges,
    } = prepared;
    let mut fixed_overlaps_cache = None::<std::sync::Arc<BTreeSet<NodeId>>>;
    let mut unrouted = 0;
    let mut total_score = 0.0;
    let router_edge_set = router_edges.iter().copied().collect::<BTreeSet<_>>();
    let flavor_router_edges = ordered_edges
        .iter()
        .copied()
        .filter(|edge_index| router_edge_set.contains(edge_index))
        .collect::<Vec<_>>();
    let router_edge_count = flavor_router_edges.len();
    for (this_edge_order_index, edge_index) in flavor_router_edges.iter().copied().enumerate() {
        let edge = &graph.edge_records[edge_index];
        #[cfg(feature = "diagnostic-traces")]
        let trace_generation = |phase: &str| {
            if std::env::var("D2_TRACE_GENERATION_WORK").as_deref() == Ok("1") {
                eprintln!(
                    "{}",
                    serde_json::json!({"event":"generation-work","scope_nodes":graph.node_order.len(),"scope_edges":graph.edge_order.len(),"edge":edge.tala_id,"flavor":trace_flavor,"phase":phase,"work":budget.map(LocalRouteSearchWork::used)})
                );
            }
        };
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("begin");
        if budget.is_some_and(|guard| guard.step().is_err()) {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        }
        let score_before = total_score;
        let trace_route_cost = |branch: &str, total_score: f64, route_len: usize| {
            if crate::internal::trace_env_enabled("TALA_TRACE_ROUTE_COSTS") {
                eprintln!(
                    "ROUTE_COST_RUST flavor={} edge={} from={} to={} branch={} distance={:.17e} points={}",
                    trace_flavor,
                    edge.tala_id,
                    graph.nodes[edge.from.0 as usize].tala_id,
                    graph.nodes[edge.to.0 as usize].tala_id,
                    branch,
                    total_score - score_before,
                    route_len,
                );
            }
        };
        let orientation = graph.sized_orientation(edge.from, edge.to);
        let reverse_orientation = graph.sized_orientation(edge.to, edge.from);
        let overlap = orientation == Orientation::None || reverse_orientation == Orientation::None;
        let disable_slingshot =
            crate::internal::trace_env_value("TALA_TRACE_DISABLE_SLINGSHOT_EDGE")
                .is_some_and(|value| value == edge.tala_id.to_string());
        if !disable_slingshot
            && !super::slingshot::charge_outer_call(graph, edge_index, orientation, budget)
        {
            // Go's slingshot worker reports the resource error through the
            // flavor guard. The sticky exhausted bit is checked by the
            // enclosing route stage after all workers have joined.
            unrouted += router_edge_count - this_edge_order_index;
            break;
        }
        let slingshot_checkers = super::slingshot::SlingshotCheckers::default();
        #[cfg(feature = "diagnostic-traces")]
        let slingshot_checkers = slingshot_checkers.with_trace_flavor(trace_flavor, edge.tala_id);
        if !disable_slingshot
            && let Some(route) = super::slingshot::l_route(
                graph,
                &visibility,
                edge_index,
                &node_ports,
                &boxes,
                &obstacle_node_order,
                &routes,
                &interaction_index,
                trace_flavor,
                budget,
                &slingshot_checkers,
            )
            && !mismatched_tunnel_endpoints(route.source, route.target)
        {
            #[cfg(feature = "diagnostic-traces")]
            trace_generation("slingshot-done");
            selected_ports[edge_index] = Some((route.source, route.target));
            total_score += route.cost;
            routes[edge_index] = route.points;
            if let Err(error) = interaction_index.add_generated_route_with_node_ids(
                graph,
                edge_index,
                &routes[edge_index],
                &route.node_ids,
                Some(&visibility),
                trace_flavor,
                budget,
            ) {
                // generateRoutes returns addRoute's error immediately. A
                // nonempty numerical path does not make failed publication a
                // successful worker response or permit selected-route commit.
                return (
                    routes,
                    router_edge_count - this_edge_order_index,
                    f64::INFINITY,
                    selected_ports,
                    Some(error.into_layout_error(graph, edge_index)),
                    Vec::new(),
                    Vec::new(),
                );
            }
            trace_route_cost("slingshot-l", total_score, routes[edge_index].len());
            continue;
        }
        if !disable_slingshot
            && let Some(route) = super::slingshot::s_route(
                graph,
                &visibility,
                edge_index,
                &node_ports,
                &boxes,
                &obstacle_node_order,
                &routes,
                &interaction_index,
                trace_flavor,
                budget,
                &slingshot_checkers,
            )
            && !mismatched_tunnel_endpoints(route.source, route.target)
        {
            #[cfg(feature = "diagnostic-traces")]
            trace_generation("slingshot-done");
            selected_ports[edge_index] = Some((route.source, route.target));
            total_score += route.cost;
            routes[edge_index] = route.points;
            if let Err(error) = interaction_index.add_generated_route_with_node_ids(
                graph,
                edge_index,
                &routes[edge_index],
                &route.node_ids,
                Some(&visibility),
                trace_flavor,
                budget,
            ) {
                // generateRoutes returns addRoute's error immediately. A
                // nonempty numerical path does not make failed publication a
                // successful worker response or permit selected-route commit.
                return (
                    routes,
                    router_edge_count - this_edge_order_index,
                    f64::INFINITY,
                    selected_ports,
                    Some(error.into_layout_error(graph, edge_index)),
                    Vec::new(),
                    Vec::new(),
                );
            }
            trace_route_cost("slingshot-s", total_score, routes[edge_index].len());
            continue;
        }
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("slingshot-done");
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("search-begin");
        let Some(mut prepared_search) = visibility.prepare_route_search(budget) else {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        };
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("search-arrays-done");
        if interaction_index
            .prepare_search_node_labels(graph, edge_index, budget)
            .is_none()
        {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        }
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("search-labels-done");
        // Go quickRoute uses Node.DistanceTo(..., true), which
        // is the pure rectangle-border distance.  The ordinary edge-scoring
        // distance additionally includes `distanceBetweenCenters`; that term
        // must not move an edge into or out of this open 20..42 window.
        let border_distance = graph.sized_distance_to(edge.from, edge.to);
        let quick_line = if !overlap
            && !orientation.is_diagonal()
            && border_distance < 42.0
            && border_distance > 20.0
        {
            interaction_index
                .materialize_quick_route_records(budget)
                .and_then(|materialized| {
                    route_line::route_line_with_route_records(
                        graph,
                        edge_index,
                        &materialized,
                        budget,
                    )
                })
        } else {
            None
        };
        if let Some(line) = quick_line {
            // Search allocates its temporary arrays and scans labels before
            // trying this shortcut, but performs ancestry checks only if it fails.
            #[cfg(feature = "diagnostic-traces")]
            trace_generation("search-done");
            selected_ports[edge_index] = Some((line.source, line.target));
            total_score += line.cost;
            routes[edge_index] = vec![line.source.point, line.target.point];
            let node_ids = super::slingshot::complete_route_node_ids(
                graph,
                &visibility,
                edge_index,
                &[usize::MAX, usize::MAX - 1],
            )
            .unwrap_or_default();
            if let Err(error) = interaction_index.add_generated_route_with_node_ids(
                graph,
                edge_index,
                &routes[edge_index],
                &node_ids,
                Some(&visibility),
                trace_flavor,
                budget,
            ) {
                // generateRoutes returns addRoute's error immediately. A
                // nonempty numerical path does not make failed publication a
                // successful worker response or permit selected-route commit.
                return (
                    routes,
                    router_edge_count - this_edge_order_index,
                    f64::INFINITY,
                    selected_ports,
                    Some(error.into_layout_error(graph, edge_index)),
                    Vec::new(),
                    Vec::new(),
                );
            }
            trace_route_cost("quick-line", total_score, routes[edge_index].len());
            continue;
        }
        if prepared_search
            .prepare_arrowhead_nodes(graph, edge_index, budget)
            .is_none()
        {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        }
        if prepared_search
            .prepare_ancestry(graph, edge.from, edge.to, budget)
            .is_none()
        {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        }
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("search-descendants-done");
        // Recovered search builds facing-port sets with
        // getPortsByOrientation(orientation.GetOpposite()). A diagonal
        // orientation admits both component sides; preferLaunchingVertically
        // belongs only to slingshot and must not collapse this to one side.
        let source_facing_orientation = orientation.opposite();
        let target_facing_orientation = reverse_orientation.opposite();
        let table_ports_for = |node: NodeId, index: usize| {
            [PortSide::Right, PortSide::Left]
                .into_iter()
                .filter_map(|side| {
                    nth_port_on_side(
                        boxes[&node],
                        graph.nodes[node.0 as usize].shape,
                        graph.nodes[node.0 as usize].table_column_count,
                        side,
                        index,
                    )
                })
                .collect::<Vec<_>>()
        };
        let allowed_source_ports = edge
            .source_table_column
            .map(|index| table_ports_for(edge.from, index));
        let allowed_target_ports = edge
            .target_table_column
            .map(|index| table_ports_for(edge.to, index));
        let allow_source_port_sharing = edge.source_table_column.is_some();
        let allow_target_port_sharing = edge.target_table_column.is_some();
        let source_cluster = graph.nodes[edge.from.0 as usize].cluster;
        let target_cluster = graph.nodes[edge.to.0 as usize].cluster;
        let Some((mut source_cluster_nodes, mut target_cluster_nodes)) =
            super::interactions::cluster_memberships_with_budget(graph, edge.from, edge.to, budget)
        else {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        };
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("search-clusters-done");

        // Cluster.AbductEdges routes an external edge through the temporary
        // vessel.  The vessel owns the complete member-port inventory, while
        // the recovered route keeps the original member geometry for the
        // emitted border points.  Cleanup restores the stable member
        // endpoints in ArenaGraph, so expose the vessel's combined inventory
        // only for this edge's candidate search.  Do not rewrite graph edges
        // or boxes: those remain the retained member frame used by addRoute.
        // Only temporary vessel inventories need a private port table. Ordinary
        // flavors retain the shared OVG view throughout generation.
        let may_replace_ports = !graph.clusters.is_empty() || !graph.sequences.is_empty();
        let saved_cluster_source_ports = may_replace_ports
            .then(|| node_ports.get(&edge.from).cloned())
            .flatten();
        let saved_cluster_target_ports = may_replace_ports
            .then(|| node_ports.get(&edge.to).cloned())
            .flatten();
        let mut ports_replaced = false;
        let cluster_port_inventory =
            |node: NodeId, ports: &BTreeMap<NodeId, Vec<Port>>| -> Option<Vec<Port>> {
                let cluster = graph
                    .clusters
                    .iter()
                    .find(|cluster| cluster.members.contains(&node))?;
                let mut seen = BTreeSet::new();
                let combined = cluster
                    .members
                    .iter()
                    .filter_map(|member| ports.get(member))
                    .flatten()
                    .copied()
                    .filter(|port| seen.insert(port_key(*port)))
                    .collect::<Vec<_>>();
                (!combined.is_empty()).then_some(combined)
            };
        let sequence_port_inventory = |node: NodeId,
                                       other: NodeId,
                                       ports: &BTreeMap<NodeId, Vec<Port>>|
         -> Option<Vec<Port>> {
            let sequence = graph
                .sequences
                .iter()
                .find(|sequence| sequence.members.contains(&node))?;
            if sequence.members.contains(&other) {
                return None;
            }
            let mut seen = BTreeSet::new();
            let combined = sequence
                .members
                .iter()
                .filter_map(|member| ports.get(member))
                .flatten()
                .copied()
                .filter(|port| seen.insert(port_key(*port)))
                .collect::<Vec<_>>();
            (!combined.is_empty()).then_some(combined)
        };
        if let Some(combined) = cluster_port_inventory(edge.from, &node_ports) {
            std::sync::Arc::make_mut(&mut node_ports).insert(edge.from, combined);
            ports_replaced = true;
        }
        if let Some(combined) = cluster_port_inventory(edge.to, &node_ports) {
            std::sync::Arc::make_mut(&mut node_ports).insert(edge.to, combined);
            ports_replaced = true;
        }
        if let Some(combined) = sequence_port_inventory(edge.from, edge.to, &node_ports) {
            std::sync::Arc::make_mut(&mut node_ports).insert(edge.from, combined);
            ports_replaced = true;
        }
        if let Some(combined) = sequence_port_inventory(edge.to, edge.from, &node_ports) {
            std::sync::Arc::make_mut(&mut node_ports).insert(edge.to, combined);
            ports_replaced = true;
        }
        let undesirable_cluster_arrangement = source_cluster.is_some_and(|cluster| {
            !cluster_has_desirable_arrangement_to(graph, cluster, edge.to, &boxes)
        }) || target_cluster.is_some_and(|cluster| {
            !cluster_has_desirable_arrangement_to(graph, cluster, edge.from, &boxes)
        });
        let prefer_facing_ports = (!source_cluster_nodes.is_empty()
            || !target_cluster_nodes.is_empty())
            && !undesirable_cluster_arrangement;
        if prepared_search
            .prepare_blocked_ports(
                graph,
                edge.from,
                edge.to,
                &boxes,
                node_ports.get(&edge.from).map_or(0, Vec::len),
                node_ports.get(&edge.to).map_or(0, Vec::len),
                edge.has_table_column(),
                budget,
            )
            .is_none()
        {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        }
        if edge.has_table_column() {
            let table_setup = || -> Option<()> {
                // tablePorts(false) derives each row index on right, then
                // left, reserving the incident-edge scan plus index lookup.
                for (node, allowed) in [
                    (edge.from, &allowed_source_ports),
                    (edge.to, &allowed_target_ports),
                ] {
                    if allowed.is_some() {
                        for _ in [PortSide::Right, PortSide::Left] {
                            if let Some(guard) = budget {
                                guard
                                    .reserve_sum(graph.nodes[node.0 as usize].edges.len() as u64, 1)
                                    .ok()?;
                            }
                        }
                    }
                }
                Some(())
            };
            if table_setup().is_none()
                || prepared_search
                    .prepare_table_blocked_ports(
                        node_ports.get(&edge.from).map(Vec::as_slice).unwrap_or(&[]),
                        node_ports.get(&edge.to).map(Vec::as_slice).unwrap_or(&[]),
                        allowed_source_ports.as_deref(),
                        allowed_target_ports.as_deref(),
                        budget,
                    )
                    .is_none()
            {
                unrouted += router_edge_count - this_edge_order_index;
                break;
            }
        }
        let mut neighbors_complete = true;
        // Between two columns Go walks source incidence before target
        // incidence. A one-column edge walks only the opposite endpoint.
        for (node, enabled, neighbors) in [
            (
                edge.from,
                edge.target_table_column.is_some(),
                &mut target_cluster_nodes,
            ),
            (
                edge.to,
                edge.source_table_column.is_some(),
                &mut source_cluster_nodes,
            ),
        ] {
            if enabled {
                for other_edge in &graph.nodes[node.0 as usize].edges.read() {
                    if !budget.is_none_or(|guard| guard.step().is_ok()) {
                        neighbors_complete = false;
                        break;
                    }
                    let other = &graph.edge_records[other_edge.0 as usize];
                    neighbors.insert(if other.from == node {
                        other.to
                    } else {
                        other.from
                    });
                }
                if !neighbors_complete {
                    break;
                }
            }
        }
        if !neighbors_complete {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        }
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("search-blocked-ports-done");
        let Some(used_ports) = interaction_index.used_ports(
            graph,
            edge_index,
            &visibility,
            &node_ports,
            &source_cluster_nodes,
            &target_cluster_nodes,
            undesirable_cluster_arrangement,
            budget,
        ) else {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        };
        let UsedPortState {
            source: used_source_ports,
            target: used_target_ports,
            duplicate_source: duplicate_source_ports,
            duplicate_target: duplicate_target_ports,
            shared_source: shared_cluster_source_ports,
            shared_target: shared_cluster_target_ports,
        } = used_ports;
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("search-usedports-done");
        if crate::internal::trace_env_value("TALA_TRACE_CENTER_PORTS")
            .is_some_and(|raw| raw == edge.tala_id.to_string())
        {
            let probe = Point {
                x: 2574.0,
                y: 1710.0,
            };
            for (other_index, route) in routes.iter().enumerate() {
                if route.contains(&probe) {
                    let matching = node_ports
                        .get(&edge.to)
                        .into_iter()
                        .flatten()
                        .filter(|port| port.point == probe)
                        .copied()
                        .collect::<Vec<_>>();
                    eprintln!(
                        "CENTER_PROBE_RUST edge={} other={} from={} to={} matching={:?} selected={:?}",
                        edge.tala_id,
                        graph.edge_records[other_index].tala_id,
                        graph.nodes[graph.edge_records[other_index].from.0 as usize].tala_id,
                        graph.nodes[graph.edge_records[other_index].to.0 as usize].tala_id,
                        matching,
                        selected_ports
                            .get(other_index)
                            .and_then(|selected| *selected),
                    );
                }
            }
        }
        if crate::internal::trace_env_value("TALA_TRACE_CENTER_PORTS")
            .is_some_and(|raw| raw == edge.tala_id.to_string())
        {
            let source_marked = node_ports
                .get(&edge.from)
                .into_iter()
                .flatten()
                .filter(|port| {
                    port.is_center
                        && mirrored_center_port_coordinate_is_used(
                            graph.nodes[edge.from.0 as usize].shape,
                            **port,
                            node_ports.get(&edge.from).map(Vec::as_slice).unwrap_or(&[]),
                            &used_source_ports,
                        )
                })
                .map(|port| port.point)
                .collect::<Vec<_>>();
            let target_marked = node_ports
                .get(&edge.to)
                .into_iter()
                .flatten()
                .filter(|port| {
                    port.is_center
                        && mirrored_center_port_coordinate_is_used(
                            graph.nodes[edge.to.0 as usize].shape,
                            **port,
                            node_ports.get(&edge.to).map(Vec::as_slice).unwrap_or(&[]),
                            &used_target_ports,
                        )
                })
                .map(|port| port.point)
                .collect::<Vec<_>>();
            eprintln!(
                "CENTER_PORTS_RUST edge={} source_used={:?} target_used={:?} source_marked={:?} target_marked={:?}",
                edge.tala_id, used_source_ports, used_target_ports, source_marked, target_marked,
            );
        }
        let source_raw_ports = node_ports.get(&edge.from).map(Vec::as_slice).unwrap_or(&[]);
        let target_raw_ports = node_ports.get(&edge.to).map(Vec::as_slice).unwrap_or(&[]);
        let symmetry_endpoints = [
            SymmetryEndpoint {
                node: edge.from,
                shape: graph.nodes[edge.from.0 as usize].shape,
                raw_ports: source_raw_ports,
                shape_ports: saved_cluster_source_ports
                    .as_deref()
                    .unwrap_or(source_raw_ports),
                used: &used_source_ports,
            },
            SymmetryEndpoint {
                node: edge.to,
                shape: graph.nodes[edge.to.0 as usize].shape,
                raw_ports: target_raw_ports,
                shape_ports: saved_cluster_target_ports
                    .as_deref()
                    .unwrap_or(target_raw_ports),
                used: &used_target_ports,
            },
        ];
        let Some(symmetrical_nodes) =
            center_symmetrical_ports(&visibility, &symmetry_endpoints, budget)
        else {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        };
        prepared_search.mark_symmetry_prepared();
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("search-symmetry-done");
        if fixed_overlaps_cache.is_none() {
            let Some(overlaps) =
                super::visibility::fixed_overlaps_for_route(graph, &graph.node_order, budget)
            else {
                unrouted += router_edge_count - this_edge_order_index;
                break;
            };
            fixed_overlaps_cache = Some(std::sync::Arc::new(overlaps));
        }
        prepared_search.set_fixed_overlaps(std::sync::Arc::clone(
            fixed_overlaps_cache.as_ref().unwrap(),
        ));
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("search-fixed-overlaps-done");
        let mirrored_center_is_used = |node: NodeId, port: Port| {
            visibility
                .port_index(node, port.point)
                .is_some_and(|identity| symmetrical_nodes.contains(&identity))
        };
        let mut best: Option<(f64, Port, Port, Vec<Point>, Vec<usize>)> = None;
        // Upstream uses the same multi-source OVG search for every routing
        // flavor, including TopDownLeftRight. Hierarchy membership changes
        // edge order, not the endpoint search or its port penalties. Publish
        // its selected ports directly, as ovg_edge_router.go:generateRoutes
        // does; re-searching direct tunnels changes equal-cost port choices.
        {
            let source_candidates = node_ports[&edge.from]
                .iter()
                .copied()
                .filter(|source| table_port_allowed(&allowed_source_ports, *source))
                .map(|source| {
                    let source_key = interaction_point_key(source.point);
                    // Match ovg_edge_router.go:search's connector accumulator.
                    // The prohibited-sharing cost replaces the unit base.
                    let mut connector_cost = if !allow_source_port_sharing
                        && !prefer_facing_ports
                        && duplicate_source_ports.contains(&source_key)
                    {
                        10_000_000.0
                    } else {
                        1.0
                    };
                    if !shared_cluster_source_ports.is_empty()
                        && !shared_cluster_source_ports.contains(&source_key)
                    {
                        connector_cost += 2.0 * graph.routing_costs.1;
                    }
                    if prefer_facing_ports
                        && !side_matches_orientation(source.side, source_facing_orientation)
                    {
                        connector_cost += 2.0 * graph.routing_costs.1;
                    }
                    if !source.is_center {
                        connector_cost += graph.routing_costs.2;
                    } else if !mirrored_center_is_used(edge.from, source) {
                        connector_cost += 1.0;
                    }
                    (source, connector_cost)
                })
                .collect::<Vec<_>>();
            let target_candidates = node_ports[&edge.to]
                .iter()
                .copied()
                .filter(|target| table_port_allowed(&allowed_target_ports, *target))
                .map(|target| {
                    let target_key = interaction_point_key(target.point);
                    // Match ovg_edge_router.go:search's connector accumulator.
                    // The prohibited-sharing cost replaces the unit base.
                    let mut connector_cost = if !allow_target_port_sharing
                        && !prefer_facing_ports
                        && duplicate_target_ports.contains(&target_key)
                    {
                        10_000_000.0
                    } else {
                        1.0
                    };
                    if !shared_cluster_target_ports.is_empty()
                        && !shared_cluster_target_ports.contains(&target_key)
                    {
                        connector_cost += 2.0 * graph.routing_costs.1;
                    }
                    if prefer_facing_ports
                        && !side_matches_orientation(target.side, target_facing_orientation)
                    {
                        connector_cost += 2.0 * graph.routing_costs.1;
                    }
                    if !target.is_center {
                        connector_cost += graph.routing_costs.2;
                    } else if !mirrored_center_is_used(edge.to, target) {
                        connector_cost += 1.0;
                    }
                    (target, connector_cost)
                })
                .collect::<Vec<_>>();
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("TALA_TRACE_ROUTE_ADDITIONAL")
                && fixed_routes.is_some()
            {
                let port_json = |(port, penalty): &(Port, f64)| {
                    serde_json::json!({
                        "x_bits": format!("0x{:016x}", port.point.x.to_bits()),
                        "y_bits": format!("0x{:016x}", port.point.y.to_bits()),
                        "side": format!("{:?}", port.side),
                        "direction": format!("{:?}", port.direction),
                        "center": port.is_center,
                        "tunnel": port.tunnel,
                        "connector_cost_bits": format!("0x{:016x}", penalty.to_bits()),
                    })
                };
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "route-additional-candidates",
                        "edge_index": edge_index,
                        "edge_id": edge.tala_id,
                        "from": graph.nodes[edge.from.0 as usize].tala_id,
                        "to": graph.nodes[edge.to.0 as usize].tala_id,
                        "source": source_candidates.iter().map(port_json).collect::<Vec<_>>(),
                        "target": target_candidates.iter().map(port_json).collect::<Vec<_>>(),
                    })
                );
            }
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_value("TALA_TRACE_CONNECTOR_DETAIL").as_deref()
                == Some("1")
            {
                for (port, penalty) in target_candidates.iter().copied() {
                    eprintln!(
                        "CONNECTOR_CANDIDATE_RUST edge={} target=({:.17e},{:.17e}) center={} side={:?} direction={:?} turn_bits=0x{:016x} non_center_bits=0x{:016x} cost_bits=0x{:016x} penalty_bits=0x{:016x}",
                        edge.tala_id,
                        port.point.x,
                        port.point.y,
                        port.is_center,
                        port.side,
                        port.direction,
                        graph.routing_costs.1.to_bits(),
                        graph.routing_costs.2.to_bits(),
                        if !allow_target_port_sharing
                            && !prefer_facing_ports
                            && duplicate_target_ports.contains(&interaction_point_key(port.point))
                        {
                            10_000_000.0_f64
                        } else {
                            1.0_f64
                        }
                        .to_bits(),
                        penalty.to_bits(),
                    );
                }
            }
            let route_result = visibility.route_between_connectors_with_prepared_state(
                graph,
                &source_candidates,
                &target_candidates,
                edge.from,
                edge.to,
                Some(edge_index),
                &interaction_index,
                &boxes,
                overlap,
                budget,
                prepared_search,
            );
            if let Some(route) = route_result
                && !mismatched_tunnel_endpoints(route.source, route.target)
            {
                best = Some((
                    route.cost,
                    route.source,
                    route.target,
                    route.points,
                    route.node_ids,
                ));
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        trace_generation("search-done");
        // Go's routeLineChecked propagates the failed work guard before
        // constructing a fallback. An absent search result must not turn
        // resource exhaustion or cancellation into a publishable line.
        if budget.is_some_and(|guard| guard.exhausted() || guard.check().is_err()) {
            unrouted += router_edge_count - this_edge_order_index;
            break;
        }
        if let Some((score, source, target, route, node_ids)) = best {
            selected_ports[edge_index] = Some((source, target));
            total_score += score;
            routes[edge_index] = route;
            if let Err(error) = interaction_index.add_generated_route_with_node_ids(
                graph,
                edge_index,
                &routes[edge_index],
                &node_ids,
                Some(&visibility),
                trace_flavor,
                budget,
            ) {
                // generateRoutes returns addRoute's error immediately. A
                // nonempty numerical path does not make failed publication a
                // successful worker response or permit selected-route commit.
                return (
                    routes,
                    router_edge_count - this_edge_order_index,
                    f64::INFINITY,
                    selected_ports,
                    Some(error.into_layout_error(graph, edge_index)),
                    Vec::new(),
                    Vec::new(),
                );
            }
        } else if let Some(line) = {
            // Go routeLoops commits loop points before router creation;
            // ordinary generated routes leave original GEdge.Points alone.
            let mut original_points = graph
                .edge_records
                .iter()
                .map(|edge| edge.route.read().clone())
                .collect::<Vec<_>>();
            for &prior in &preexisting_route_order {
                if graph.edge_records[prior].from == graph.edge_records[prior].to {
                    original_points[prior] = routes[prior].clone();
                }
            }
            route_line::route_line_with_published_records(
                graph,
                edge_index,
                interaction_index.accepted_edge_indices(),
                &original_points,
                &interaction_index.routes,
                budget,
            )
        } {
            // Recovered generateRoutes invokes routeLine only after slingshot
            // or OVG search returns an error. It is a genuine fallback, not a
            // competing direct-route candidate.
            selected_ports[edge_index] = Some((line.source, line.target));
            total_score += line.cost;
            routes[edge_index] = vec![line.source.point, line.target.point];
            let interior_ids = [
                visibility.index_of(line.source.point),
                visibility.index_of(line.target.point),
            ];
            let node_ids = if let [Some(source_id), Some(target_id)] = interior_ids {
                super::slingshot::complete_route_node_ids(
                    graph,
                    &visibility,
                    edge_index,
                    &[source_id, target_id],
                )
                .unwrap_or_default()
            } else {
                Vec::new()
            };
            if let Err(error) = interaction_index.add_generated_route_with_node_ids(
                graph,
                edge_index,
                &routes[edge_index],
                &node_ids,
                Some(&visibility),
                trace_flavor,
                budget,
            ) {
                // generateRoutes returns addRoute's error immediately. A
                // nonempty numerical path does not make failed publication a
                // successful worker response or permit selected-route commit.
                return (
                    routes,
                    router_edge_count - this_edge_order_index,
                    f64::INFINITY,
                    selected_ports,
                    Some(error.into_layout_error(graph, edge_index)),
                    Vec::new(),
                    Vec::new(),
                );
            }
        } else if budget.is_some_and(|guard| guard.exhausted() || guard.check().is_err()) {
            // A guarded routeLine failure is terminal; only an invalid
            // candidate may proceed to Go's clipped-center fallback.
            unrouted += router_edge_count - this_edge_order_index;
            break;
        } else if let Some((route, cost)) = route_line::clipped_center_fallback(graph, edge_index) {
            // Recovered ovgEdgeRouter.routeLine catches Graph.RouteLine's
            // IllegalMove and clips the center line to both boxes. These are
            // deliberately not ordinary occupied ports.
            total_score += cost;
            routes[edge_index] = route;
            let node_ids = super::slingshot::complete_route_node_ids(
                graph,
                &visibility,
                edge_index,
                &[usize::MAX, usize::MAX - 1],
            )
            .unwrap_or_default();
            if let Err(error) = interaction_index.add_generated_route_with_node_ids(
                graph,
                edge_index,
                &routes[edge_index],
                &node_ids,
                Some(&visibility),
                trace_flavor,
                budget,
            ) {
                // generateRoutes returns addRoute's error immediately. A
                // nonempty numerical path does not make failed publication a
                // successful worker response or permit selected-route commit.
                return (
                    routes,
                    router_edge_count - this_edge_order_index,
                    f64::INFINITY,
                    selected_ports,
                    Some(error.into_layout_error(graph, edge_index)),
                    Vec::new(),
                    Vec::new(),
                );
            }
        } else {
            unrouted += 1;
        }
        trace_route_cost(
            if selected_ports[edge_index].is_some() {
                "ovg-or-line"
            } else {
                "unrouted"
            },
            total_score,
            routes[edge_index].len(),
        );
        if ports_replaced {
            let node_ports = std::sync::Arc::make_mut(&mut node_ports);
            if let Some(ports) = saved_cluster_source_ports {
                node_ports.insert(edge.from, ports);
            }
            if let Some(ports) = saved_cluster_target_ports {
                node_ports.insert(edge.to, ports);
            }
        }
    }
    // generateRoutes returns the guard error from slingshot/search/routeLine
    // before testing the route length. An interrupted generation leaves empty
    // slots; those are not an independent route invariant. Preserve the cause
    // before the caller's deferred finish can observe a later cancellation.
    let failure = if let Some(work) = budget.filter(|work| work.has_failed()) {
        Some(super::finalize::failure(work, trace_flavor).into_layout_error())
    } else if fixed_routes.is_some() {
        let failed = ordered_edges.iter().copied().find(|edge_index| {
            let edge = &graph.edge_records[*edge_index];
            edge.from == edge.to || routes[*edge_index].is_empty()
        });
        unrouted = ordered_edges
            .iter()
            .filter(|edge_index| {
                let edge = &graph.edge_records[**edge_index];
                edge.from == edge.to || routes[**edge_index].is_empty()
            })
            .count();
        failed.map(|edge_index| {
            // routeAdditionalEdges does not run the ordinary loop prepass.
            // With identical source and target centers the generic router's
            // OVG route contains only that one node, which generateRoutes
            // rejects before addRoute.
            let route_size =
                if graph.edge_records[edge_index].from == graph.edge_records[edge_index].to {
                    1
                } else {
                    routes[edge_index].len()
                };
            crate::LayoutError::InvariantViolation(format!(
                "Route '{}' has size {}",
                route_edge_debug_id(graph, edge_index),
                route_size
            ))
        })
    } else {
        None
    };
    (
        routes,
        unrouted,
        total_score,
        selected_ports,
        failure,
        #[cfg(feature = "diagnostic-traces")]
        preexisting_route_order,
        #[cfg(not(feature = "diagnostic-traces"))]
        Vec::new(),
        interaction_index.routes,
    )
}

fn ovg_routing_nodes(scope_nodes: &[NodeId], nearby_nodes: &[NodeId]) -> Vec<NodeId> {
    let mut routing_nodes = nearby_nodes.to_vec();
    for node in scope_nodes.iter().copied() {
        if !routing_nodes.contains(&node) {
            routing_nodes.push(node);
        }
    }
    routing_nodes
}

fn ovg_port_node_order(
    routed_graph_node_order: &[NodeId],
    routing_nodes: &[NodeId],
    nearby_nodes: &[NodeId],
    port_owners: impl IntoIterator<Item = NodeId>,
) -> Vec<NodeId> {
    let port_owners = port_owners.into_iter().collect::<BTreeSet<_>>();
    let mut order = nearby_nodes
        .iter()
        .copied()
        .filter(|node| port_owners.contains(node))
        .collect::<Vec<_>>();
    for node in routed_graph_node_order.iter().chain(routing_nodes).copied() {
        if port_owners.contains(&node) && !order.contains(&node) {
            order.push(node);
        }
    }
    order
}

/// Reconstruct `OVG.NodesInsideBoundingBox` iteration order for route
/// collision checks. Go starts with nearby nodes, appends non-hierarchy nodes
/// in the routed graph's node order, then appends one hierarchy OVG per first
/// seen hierarchy. `newOVGForHierarchy` sorts those members by source EntityID.
fn ovg_obstacle_node_order(
    graph: &ArenaGraph,
    routed_graph_node_order: &[NodeId],
    nearby_nodes: &[NodeId],
    boxes: &BTreeMap<NodeId, Rect>,
    budget: Option<&super::route_search_guard::OvgBuildBudget<'_>>,
) -> Result<Vec<NodeId>, String> {
    let mut order = nearby_nodes
        .iter()
        .copied()
        .filter(|node| boxes.contains_key(node))
        .collect::<Vec<_>>();

    for node in routed_graph_node_order.iter().copied() {
        if let Some(budget) = budget {
            budget.reserve_work(1).map_err(|_| {
                "OVG node classification cancelled or exceeded its work limit".to_owned()
            })?;
        }
        if graph.nodes[node.0 as usize].hierarchy.is_none() && boxes.contains_key(&node) {
            order.push(node);
        }
    }

    let mut seen_hierarchies = BTreeSet::new();
    for node in routed_graph_node_order.iter().copied() {
        let Some(hierarchy) = graph.nodes[node.0 as usize].hierarchy.clone() else {
            continue;
        };
        if !seen_hierarchies.insert(hierarchy.id) {
            continue;
        }
        let mut members = routed_graph_node_order
            .iter()
            .copied()
            .filter(|member| {
                boxes.contains_key(member)
                    && graph.nodes[member.0 as usize]
                        .hierarchy
                        .clone()
                        .is_some_and(|membership| membership.id == hierarchy.id)
            })
            .collect::<Vec<_>>();
        members.stable_sort_by_key(|member| graph.nodes[member.0 as usize].tala_id);
        order.extend(members);
    }
    Ok(order)
}

#[cfg(feature = "diagnostic-traces")]
fn trace_route_flavor(
    graph: &ArenaGraph,
    flavor: &str,
    order: &[usize],
    routes: &[Vec<Point>],
    distance: f64,
    preexisting_order: &[usize],
) {
    if !crate::internal::trace_env_enabled("TALA_TRACE_ROUTING") {
        return;
    }
    // Go's router appends routes that were precomputed by the loop/tree
    // prepass before it appends the flavor's ordinary edge routes. Rust keeps
    // all routes indexed by edge ID, so replay the prepass insertion order for
    // the trace only; routing and public output continue to use the indexed
    // representation.
    let mut trace_order = Vec::with_capacity(order.len());
    for &index in preexisting_order {
        if !trace_order.contains(&index) && !routes[index].is_empty() {
            trace_order.push(index);
        }
    }
    for &index in order {
        if !trace_order.contains(&index) {
            trace_order.push(index);
        }
    }
    let routes=trace_order.iter().map(|&index| {
        let edge=&graph.edge_records[index];
        // Go's diagnostic observes Route.OVGNodes, whose synthetic first and
        // last entries are the endpoint centers; the Rust release route owns
        // only the segment endpoints. Reconstruct that diagnostic view here
        // without changing the route used by the engine or public output.
        let mut points = Vec::with_capacity(routes[index].len() + 2);
        let center = |node: NodeId| {
            let position = graph.position(node).unwrap_or(graph.nodes[node.0 as usize].rect.origin);
            let size = graph.nodes[node.0 as usize].rect.size;
            Point {
                x: position.x + size.width / 2.0,
                y: position.y + size.height / 2.0,
            }
        };
        points.push(center(edge.from));
        points.extend(routes[index].iter().copied());
        points.push(center(edge.to));
        serde_json::json!({"id":edge.tala_id,"from":graph.nodes[edge.from.0 as usize].tala_id,"to":graph.nodes[edge.to.0 as usize].tala_id,"points":points.iter().map(|p|serde_json::json!({"x_bits":format!("0x{:016x}",p.x.to_bits()),"y_bits":format!("0x{:016x}",p.y.to_bits())})).collect::<Vec<_>>()})
    }).collect::<Vec<_>>();
    eprintln!(
        "{}",
        serde_json::json!({"event":"route-flavor","flavor":flavor,"distance_bits":format!("0x{:016x}",distance.to_bits()),"routes":routes})
    );
}

/// A constructor's deferred finish records work, but cannot replace the
/// error already returned by its failing operation (ovg_edge_router.go).
fn finish_failed_route_constructor(
    work: &LocalRouteSearchWork,
    response: Option<crate::LayoutError>,
) -> crate::LayoutError {
    let failure = work.failure();
    let error = match failure {
        crate::LayoutError::Cancelled | crate::LayoutError::DeadlineExceeded => failure,
        _ => response.unwrap_or(failure),
    };
    let _ = work.finish();
    error
}

fn route_edge_subset(
    graph: &mut ArenaGraph,
    scope_nodes: &[NodeId],
    nearby_nodes: &[NodeId],
    declaration_order: &[usize],
    budget_limit: Option<u64>,
    ovg_budget: Option<&super::route_search_guard::OvgBuildBudget>,
    aggregate: Option<&std::sync::Arc<super::route_search_guard::RouteStageWork>>,
) -> Result<super::finalize::Updates, crate::LayoutError> {
    let active_cancellation = crate::internal::current_cancellation();
    let parent = super::route_stage_guard::RuntimeRouteContext;
    // coordinator.go constructs the shared OVG before any router constructor.
    let shared_context = build_route_ovg_context_mut(
        graph,
        scope_nodes,
        nearby_nodes,
        declaration_order,
        None,
        ovg_budget,
    )
    .map_err(|error| {
        crate::internal::cancellation_layout_error()
            .unwrap_or(crate::LayoutError::BadRouteState(error))
    })?;
    let stage_limit = || crate::LayoutError::WorkLimit {
        seed: 0,
        location: "EdgeRouting",
        limit: super::route_search_guard::MAX_EDGE_ROUTING_STAGE_WORK_UNITS,
    };
    if ovg_budget.is_some_and(|guard| guard.exhausted()) {
        return Err(stage_limit());
    }
    let hierarchy = scope_nodes
        .first()
        .is_some_and(|node| graph.nodes[node.0 as usize].hierarchy.is_some());
    let flavors = if hierarchy {
        vec!["TopDownLeftRight"]
    } else {
        vec!["ShortestToLongest", "LongestToShortest", "Default"]
    };
    let cluster_nodes = graph
        .clusters
        .iter()
        .flat_map(|cluster| cluster.members.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut routers = Vec::with_capacity(flavors.len());
    let flavor_count = flavors.len();
    let mut shared_context = Some(shared_context);
    for (flavor_index, flavor) in flavors.into_iter().enumerate() {
        let limit = budget_limit.unwrap_or(super::MAX_ROUTE_SEARCH_WORK_UNITS);
        let work = aggregate.map_or_else(
            || LocalRouteSearchWork::new(limit),
            |aggregate| LocalRouteSearchWork::with_deferred_aggregate(limit, aggregate.clone()),
        );
        work.bind(&parent)?;
        let mut order = if hierarchy {
            top_down_left_right_edge_order(graph, declaration_order)
        } else {
            declaration_order.to_vec()
        };
        let distance = |index: usize| {
            let edge = &graph.edge_records[index];
            graph.sized_distance(edge.from, edge.to)
        };
        if flavor == "ShortestToLongest" {
            order.stable_sort_by(|a, b| distance(*a).total_cmp(&distance(*b)));
        } else if flavor == "LongestToShortest" {
            order.stable_sort_by(|a, b| distance(*b).total_cmp(&distance(*a)));
        }
        order.stable_sort_by_key(|index| {
            let edge = &graph.edge_records[*index];
            !(cluster_nodes.contains(&edge.from) || cluster_nodes.contains(&edge.to))
        });
        // All constructors run under the caller context before child creation.
        // The context is not read after the last constructor.
        let flavor_context = if flavor_index + 1 == flavor_count {
            shared_context.take()
        } else {
            shared_context.clone()
        };
        let prepared = match prepare_route_flavor_owned(
            graph,
            scope_nodes,
            nearby_nodes,
            &order,
            declaration_order,
            flavor,
            None,
            flavor_context,
            None,
            Some(&work),
        ) {
            Ok(prepared) => prepared,
            Err(response) => return Err(finish_failed_route_constructor(&work, response.4)),
        };
        if work.check().is_err() {
            let error = work.failure();
            let _ = work.finish();
            return Err(error);
        }
        routers.push((flavor, (flavor, order, work.into_state(), prepared)));
    }
    let responses = super::workers::generate(
        active_cancellation.as_ref(),
        routers,
        |(flavor, order, state, prepared)| {
            let work = state.resume();
            if let Err(error) = work.bind_current_scope() {
                let _ = work.finish();
                return Err(error);
            }
            let generated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                generate_route_flavor(
                    graph,
                    scope_nodes,
                    &order,
                    flavor,
                    None,
                    Some(&work),
                    prepared,
                )
            }));
            // generateRoutes defers finish on both normal and panic exits.
            let finished = work.finish();
            let candidate = match generated {
                Ok(response) => response,
                Err(payload) => std::panic::resume_unwind(payload),
            };
            if let Some(error) = candidate.4.as_ref() {
                return Err(error.clone());
            }
            if finished.is_err() || work.exhausted() {
                // route_search_guard.go reports the declared flavor and its
                // limit; finalization uses the same guard error boundary.
                return Err(super::finalize::failure(&work, flavor).into_layout_error());
            }
            if candidate.1 != 0 {
                return Err(crate::LayoutError::InvariantViolation(
                    "could not route every requested edge".into(),
                ));
            }
            Ok((flavor, order, work.into_state(), candidate))
        },
    )?;
    let successes = super::workers::successful(
        responses.into_iter().map(|(_, response)| response),
        aggregate
            .filter(|guard| guard.exhausted())
            .map(|_| stage_limit()),
    )?;
    let mut best = None;
    for (_flavor, _order, state, candidate) in successes {
        #[cfg(feature = "diagnostic-traces")]
        trace_route_flavor(
            graph,
            _flavor,
            &_order,
            &candidate.0,
            candidate.2,
            &candidate.5,
        );
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_ROUTE_FLAVORS") {
            eprintln!(
                "ROUTE_FLAVOR_RUST flavor={_flavor} unrouted={} distance={:.17e}",
                candidate.1, candidate.2
            );
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_ROUTE_ALL_FLAVORS") {
            eprintln!(
                "ROUTE_FLAVOR_LENS flavor={_flavor} lens={:?}",
                candidate.0.iter().map(Vec::len).collect::<Vec<_>>()
            );
        }
        let least_distance = best.as_ref().map_or(
            f64::INFINITY,
            |(_, _, previous): &(
                &'static str,
                super::route_search_guard::RouteSearchWorkState,
                RouteFlavorResponse,
            )| previous.2,
        );
        if routing_precision_compare(candidate.2, least_distance).is_lt() {
            best = Some((_flavor, state, candidate));
        }
    }
    let Some((flavor, state, (_, _, _, _, _, _, mut records))) = best else {
        return Err(crate::LayoutError::InvariantViolation(
            "selected edge-routing flavor has no work guard".into(),
        ));
    };
    let work = state.resume();
    let order = (0..records.len()).map(Some).collect::<Vec<_>>();
    super::finalize::selected(
        Some(graph),
        &mut records,
        &order,
        None,
        Some(&work),
        &parent,
        flavor,
        true,
    )
    .map_err(super::finalize::Error::into_layout_error)
}

/// Recovered `Graph.routeAdditionalEdges`: route only `requested` on one
/// full-graph OVG while importing every other serialized route as fixed
/// topology and interaction state.
fn route_additional_edge_subset(
    graph: &mut ArenaGraph,
    scope_nodes: &[NodeId],
    requested: &[usize],
    fixed_routes: &[Vec<Point>],
    budget_limit: Option<u64>,
    seed: i64,
    ovg_budget: &super::route_search_guard::OvgBuildBudget<'_>,
) -> Result<super::finalize::Updates, crate::LayoutError> {
    let cluster_nodes = graph
        .clusters
        .iter()
        .flat_map(|cluster| cluster.members.iter().copied())
        .collect::<BTreeSet<_>>();
    let hierarchy = scope_nodes
        .first()
        .is_some_and(|node| graph.nodes[node.0 as usize].hierarchy.is_some());
    let flavors = if hierarchy {
        vec!["TopDownLeftRight"]
    } else {
        vec!["ShortestToLongest", "LongestToShortest", "Default"]
    };
    let shared_context = build_route_ovg_context_mut(
        graph,
        scope_nodes,
        &[],
        requested,
        Some(fixed_routes),
        Some(ovg_budget),
    )
    .map_err(|_| crate::LayoutError::WorkLimit {
        seed,
        location: "EdgeRouting",
        limit: budget_limit.unwrap_or(super::route_search_guard::MAX_OVG_WORK_UNITS),
    })?;
    if ovg_budget.exhausted() {
        return Err(crate::LayoutError::WorkLimit {
            seed,
            location: "EdgeRouting",
            limit: budget_limit.unwrap_or(super::route_search_guard::MAX_OVG_WORK_UNITS),
        });
    }
    let prioritize_cluster_edges = |edges: &mut Vec<usize>| {
        edges.stable_sort_by_key(|edge_index| {
            let edge = &graph.edge_records[*edge_index];
            !(cluster_nodes.contains(&edge.from) || cluster_nodes.contains(&edge.to))
        });
    };
    let runtime_context = super::route_stage_guard::RuntimeRouteContext;
    let parent = ovg_budget.context().unwrap_or(&runtime_context);
    // Source constructs every router with the caller context before deriving
    // the worker context or starting any generation.
    let mut prepared_flavors = Vec::with_capacity(flavors.len());
    for flavor in flavors {
        let work = LocalRouteSearchWork::with_context(
            budget_limit.unwrap_or(super::MAX_ROUTE_SEARCH_WORK_UNITS),
            parent,
        )?;
        let mut order = if hierarchy {
            top_down_left_right_edge_order(graph, requested)
        } else {
            requested.to_vec()
        };
        if flavor == "ShortestToLongest" {
            order.stable_sort_by(|left, right| {
                graph
                    .sized_distance(graph.edge_records[*left].from, graph.edge_records[*left].to)
                    .total_cmp(&graph.sized_distance(
                        graph.edge_records[*right].from,
                        graph.edge_records[*right].to,
                    ))
            });
        } else if flavor == "LongestToShortest" {
            order.stable_sort_by(|left, right| {
                graph
                    .sized_distance(
                        graph.edge_records[*right].from,
                        graph.edge_records[*right].to,
                    )
                    .total_cmp(&graph.sized_distance(
                        graph.edge_records[*left].from,
                        graph.edge_records[*left].to,
                    ))
            });
        }
        prioritize_cluster_edges(&mut order);
        let prepared = match prepare_route_flavor(
            graph,
            scope_nodes,
            &[],
            &order,
            requested,
            flavor,
            Some(fixed_routes),
            Some(&shared_context),
            None,
            Some(&work),
        ) {
            Ok(prepared) => prepared,
            Err(response) => {
                let _ = work.finish();
                if matches!(
                    work.failure(),
                    crate::LayoutError::Cancelled | crate::LayoutError::DeadlineExceeded
                ) {
                    return Err(work.failure());
                }
                return Err(response.4.unwrap_or_else(|| work.failure()));
            }
        };
        if work.check().is_err() {
            let error = work.failure();
            let _ = work.finish();
            return Err(error);
        }
        prepared_flavors.push((flavor, order, work, prepared));
    }
    let shared_graph: &ArenaGraph = graph;
    // generateRoutes on a bound guard: finish is deferred even when generation
    // unwinds, and a failed finish fails an otherwise complete response.
    let generate_bound = |order: &[usize],
                          flavor: &'static str,
                          work: &LocalRouteSearchWork<'_>,
                          prepared: PreparedRouteFlavor| {
        let generated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            generate_route_flavor(
                shared_graph,
                scope_nodes,
                order,
                flavor,
                Some(fixed_routes),
                Some(work),
                prepared,
            )
        }));
        let finished = work.finish();
        let mut response = match generated {
            Ok(response) => response,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        if finished.is_err() && response.4.is_none() {
            response.4 = Some(super::finalize::failure(work, flavor).into_layout_error());
            response.1 = requested.len();
        }
        response
    };
    let unbound = |error: crate::LayoutError| -> RouteFlavorResponse {
        (
            Vec::new(),
            requested.len(),
            f64::INFINITY,
            Vec::new(),
            Some(error),
            Vec::new(),
            Vec::new(),
        )
    };
    #[cfg(feature = "diagnostic-traces")]
    let trace_response = |flavor: &'static str,
                          order: &[usize],
                          work: &LocalRouteSearchWork<'_>,
                          response: &RouteFlavorResponse| {
        if std::env::var("D2_TRACE_ROUTER_RESPONSES").as_deref() == Ok("1") {
            eprintln!(
                "{}",
                serde_json::json!({"event":"route-response", "flavor":flavor,
                "scope_nodes":scope_nodes.len(), "scope_edges":shared_graph.edge_order.len(),
                "work_used":work.used(), "work_exhausted":work.exhausted(),
                "unrouted":response.1, "error":response.4.as_ref().map(ToString::to_string),
                "distance_bits":format!("0x{:016x}",response.2.to_bits())})
            );
        }
        let existing_order = shared_context
            .existing_routes
            .iter()
            .map(|route| route.edge.0 as usize)
            .collect::<Vec<_>>();
        trace_route_flavor(
            shared_graph,
            flavor,
            order,
            &response.0,
            response.2,
            &existing_order,
        );
    };
    let worker_context = super::worker_context::WorkerContext::new(parent);
    let mut responses = Vec::with_capacity(prepared_flavors.len());
    let mut first_panic = None;
    if parent.reads_cancellation_scope() {
        // coordinator.go runs every flavor worker concurrently under a child of
        // the route context. Workers read only the shared graph and their own
        // prepared state; responses keep declared order, so the selected
        // flavor is the same as a serial drain's.
        let routers = prepared_flavors
            .into_iter()
            .map(|(flavor, order, work, prepared)| {
                (flavor, (flavor, order, work.into_state(), prepared))
            })
            .collect::<Vec<_>>();
        let generated = super::workers::generate(
            crate::internal::current_cancellation().as_ref(),
            routers,
            |(flavor, order, state, prepared)| {
                let work = state.resume();
                let (bound, response) = match work.bind_current_scope() {
                    Ok(()) => (true, generate_bound(&order, flavor, &work, prepared)),
                    Err(error) => {
                        let _ = work.finish();
                        (false, unbound(error))
                    }
                };
                (flavor, order, work.into_state(), bound, response)
            },
        )?;
        for (_, (flavor, order, state, _bound, response)) in generated {
            let work = state.resume();
            #[cfg(feature = "diagnostic-traces")]
            if _bound {
                trace_response(flavor, &order, &work, &response);
            }
            responses.push((flavor, order, work, response));
        }
    } else {
        // Keep all responses in declared order and drain every flavor even
        // after a recovered panic.
        for (flavor, order, work, prepared) in prepared_flavors {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Err(error) = work.bind(&worker_context) {
                    let _ = work.finish();
                    return Err(error);
                }
                // A finish panic is caught by the outer per-flavor recovery.
                Ok::<_, crate::LayoutError>(generate_bound(&order, flavor, &work, prepared))
            }));
            match result {
                Ok(Ok(response)) => {
                    #[cfg(feature = "diagnostic-traces")]
                    trace_response(flavor, &order, &work, &response);
                    responses.push((flavor, order, work, response));
                }
                Ok(Err(error)) => responses.push((flavor, order, work, unbound(error))),
                Err(_) => {
                    first_panic.get_or_insert(flavor);
                    worker_context.cancel();
                }
            }
        }
    }
    if let Some(flavor) = first_panic {
        return Err(crate::LayoutError::InvariantViolation(format!(
            "edge routing flavor {flavor} violated an invariant"
        )));
    }
    // collectRouteFlavorResponses checks Err directly, including nil/open Done.
    if let Some(cause) = parent.error() {
        return Err(match cause {
            crate::internal::limits::ContextError::Cancelled => crate::LayoutError::Cancelled,
            crate::internal::limits::ContextError::DeadlineExceeded => {
                crate::LayoutError::DeadlineExceeded
            }
        });
    }
    worker_context.cancel();
    let mut best = None;
    let mut first_error = None;
    for (flavor, _order, work, candidate) in responses {
        if candidate.1 != 0 || candidate.4.is_some() || work.exhausted() {
            first_error.get_or_insert_with(|| candidate.4.unwrap_or_else(|| work.failure()));
            continue;
        }
        if best.as_ref().is_none_or(
            |(_, _, previous): &(&'static str, LocalRouteSearchWork, RouteFlavorResponse)| {
                routing_precision_compare(candidate.2, previous.2).is_lt()
            },
        ) {
            best = Some((flavor, work, candidate));
        }
    }
    let Some((flavor, work, (_routes, _, _, _, _, _, mut records))) = best else {
        return Err(first_error.unwrap_or_else(|| {
            crate::LayoutError::BadRouteState("could not route every requested edge".into())
        }));
    };
    let order = (0..records.len()).map(Some).collect::<Vec<_>>();
    let selected = requested.iter().copied().collect::<BTreeSet<_>>();
    let updates = super::finalize::selected(
        Some(graph),
        &mut records,
        &order,
        Some(&selected),
        Some(&work),
        parent,
        flavor,
        false,
    )
    .map_err(super::finalize::Error::into_layout_error)?;
    #[cfg(feature = "diagnostic-traces")]
    if crate::internal::trace_env_enabled("TALA_TRACE_ROUTE_ADDITIONAL") {
        for edge_index in requested {
            let edge = &graph.edge_records[*edge_index];
            let points = _routes[*edge_index]
                .iter()
                .map(|point| {
                    serde_json::json!({
                        "x_bits": format!("0x{:016x}", point.x.to_bits()),
                        "y_bits": format!("0x{:016x}", point.y.to_bits()),
                    })
                })
                .collect::<Vec<_>>();
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "route-additional-result",
                    "edge_index": edge_index,
                    "edge_id": edge.tala_id,
                    "from": graph.nodes[edge.from.0 as usize].tala_id,
                    "to": graph.nodes[edge.to.0 as usize].tala_id,
                    "points": points,
                })
            );
        }
    }
    Ok(updates)
}

pub(in crate::internal) fn initialize_standalone_routing_costs(graph: &mut ArenaGraph) {
    // Go's additional router warms these lazy costs on the caller's Graph;
    // subsequent straight fallback and labeling read the same cached values.
    let nodes = graph.graph_node_order();
    let edges = graph
        .edge_order
        .read()
        .iter()
        .map(|id| id.0 as usize)
        .collect::<Vec<_>>();
    initialize_scope_routing_costs(graph, &nodes, &edges);
}

fn initialize_scope_routing_costs(
    graph: &mut ArenaGraph,
    scope_nodes: &[NodeId],
    _edge_indices: &[usize],
) {
    if scope_nodes.is_empty() {
        return;
    }
    let min_width = scope_nodes
        .iter()
        .map(|node| graph.nodes[node.0 as usize].rect.size.width)
        .fold(f64::INFINITY, f64::min);
    let min_height = scope_nodes
        .iter()
        .map(|node| graph.nodes[node.0 as usize].rect.size.height)
        .fold(f64::INFINITY, f64::min);
    let max_width = scope_nodes
        .iter()
        .map(|node| graph.nodes[node.0 as usize].rect.size.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_height = scope_nodes
        .iter()
        .map(|node| graph.nodes[node.0 as usize].rect.size.height)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_length = min_width.min(min_height);
    let max_size = max_width.max(max_height);
    graph.cell_size = if 3.0 * min_length > max_size {
        max_size.ceil()
    } else {
        (3.0 * min_length * 0.5).ceil()
    }
    .max(10.0);

    // Coordinator warms the actual Graph in Crossing/Turn/NonCenter order.
    // Then each OVG router captures values without changing the graph owner.
    graph.refresh_node_order_membership();
    graph.crossing_cost_value();
    graph.turn_cost_value();
    graph.non_center_port_cost_value();
    // newOVGEdgeRouter captures semantic return values in Turn/Crossing/Port
    // order. A no-edge port return is +0 even when the raw cache is -0.
    let turn = graph.turn_cost_value();
    let crossing = graph.crossing_cost_value();
    let noncenter = graph.non_center_port_cost_value();
    graph.routing_costs = (crossing, turn, noncenter);
}

/// Recovered `Graph.getBoundingBox` for one `SplitSubgraphs` result.
///
/// EdgeRouting uses this complete bound before selecting nearby nodes. In
/// particular, routes retained from an earlier EdgeRouting pass can expand a
/// later pass's nearby-node inventory even when no node moved.
fn routing_subgraph_bounds(
    graph: &ArenaGraph,
    subgraph: &RoutingSubgraph,
) -> Option<(Point, Point)> {
    let (mut top_left, mut bottom_right) = graph.fixed_node_bounds(&subgraph.nodes)?;
    for edge_index in subgraph.edges.iter().copied() {
        let edge = &graph.edge_records[edge_index];
        if edge.route.is_empty() {
            continue;
        }

        let mut edge_top_left = Point {
            x: f64::INFINITY,
            y: f64::INFINITY,
        };
        let mut edge_bottom_right = Point {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
        };
        let mut include_rect = |rect: Rect| {
            edge_top_left.x = edge_top_left.x.min(rect.origin.x);
            edge_top_left.y = edge_top_left.y.min(rect.origin.y);
            edge_bottom_right.x = edge_bottom_right.x.max(rect.right());
            edge_bottom_right.y = edge_bottom_right.y.max(rect.bottom());
        };
        for point in edge.route.read().iter().copied() {
            include_rect(Rect {
                origin: point,
                size: crate::Size::default(),
            });
        }
        if let Some(label) = edge
            .label
            .as_ref()
            .filter(|label| label.position != crate::LabelPosition::Unset)
            && let Some(origin) = crate::internal::labels::edge_label_top_left(
                edge,
                label.position,
                label.percentage,
                label.size.width,
                label.size.height,
            )
        {
            include_rect(Rect {
                origin,
                size: label.size,
            });
        }
        for is_target in [false, true] {
            if let Some(rect) = crate::internal::labels::arrowhead_label_rect_for_route(
                edge,
                &edge.route.read(),
                is_target,
            ) {
                include_rect(rect);
            }
        }

        edge_top_left.x = edge_top_left.x.round();
        edge_top_left.y = edge_top_left.y.round();
        edge_bottom_right.x = edge_bottom_right.x.round();
        edge_bottom_right.y = edge_bottom_right.y.round();
        top_left.x = top_left.x.min(edge_top_left.x);
        top_left.y = top_left.y.min(edge_top_left.y);
        bottom_right.x = bottom_right.x.max(edge_bottom_right.x);
        bottom_right.y = bottom_right.y.max(edge_bottom_right.y);
    }
    top_left.x = top_left.x.round();
    top_left.y = top_left.y.round();
    bottom_right.x = bottom_right.x.round();
    bottom_right.y = bottom_right.y.round();
    Some((top_left, bottom_right))
}

fn routing_nearby_nodes(
    graph: &ArenaGraph,
    subgraphs: &[RoutingSubgraph],
    subgraph_index: usize,
) -> Vec<NodeId> {
    let subgraph = &subgraphs[subgraph_index];
    let member_rects = subgraph
        .nodes
        .iter()
        .filter_map(|node| node_rect(graph, *node).map(|rect| (*node, rect)))
        .collect::<Vec<_>>();
    if member_rects.is_empty() {
        return Vec::new();
    }
    let Some((top_left, bottom_right)) = routing_subgraph_bounds(graph, subgraph) else {
        return Vec::new();
    };
    let left = top_left.x - 100.0;
    let top = top_left.y - 100.0;
    let right = bottom_right.x + 100.0;
    let bottom = bottom_right.y + 100.0;

    subgraphs
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != subgraph_index)
        .flat_map(|(_, other)| other.nodes.iter().copied())
        .filter(|node| {
            let Some(rect) = node_rect(graph, *node) else {
                return false;
            };
            // Direct translation of recovered Node.isWithinBounds: keep any
            // node whose box intersects the subgraph's 100-unit expanded
            // bounds. TALA rejects only boxes wholly beyond an edge; it does
            // not require the complete nearby box to fit inside the bounds.
            let within_bounds = rect.origin.x <= right
                && rect.origin.y <= bottom
                && rect.right() >= left
                && rect.bottom() >= top;
            let overlaps_member = member_rects.iter().any(|(_, member)| {
                rect.origin.x < member.right()
                    && member.origin.x < rect.right()
                    && rect.origin.y < member.bottom()
                    && member.origin.y < rect.bottom()
            });
            within_bounds && !overlaps_member
        })
        .collect()
}

/// Translate TALA's `EdgeRoutingStage` split-subgraph boundary and the three
/// flat-graph OVG generation flavors. Each flavor gets an independent
/// occupied-port set; the lowest total route distance wins, with ties
/// retaining shortest-first, longest-first, then current Graph.Edges order.
pub(in crate::internal) fn route_edges(graph: &ArenaGraph) -> Vec<Vec<Point>> {
    route_edges_with_budget(graph, None).0
}

fn route_edges_with_budget(
    graph: &ArenaGraph,
    budget_limit: Option<u64>,
) -> (Vec<Vec<Point>>, bool) {
    match route_edges_with_stage_budget(graph, budget_limit, None) {
        Ok(routes) => (
            routes
                .into_iter()
                .map(|route| route.map_or_else(Vec::new, |route| route.read().to_vec()))
                .collect(),
            false,
        ),
        Err(_) => (vec![Vec::new(); graph.edge_records.len()], true),
    }
}

fn route_edges_with_stage_budget(
    graph: &ArenaGraph,
    budget_limit: Option<u64>,
    aggregate: Option<std::sync::Arc<super::route_search_guard::RouteStageWork>>,
) -> Result<Vec<Option<crate::internal::RoutePoints>>, crate::LayoutError> {
    let subgraphs = routing_subgraphs(graph);

    // TALA routes Graph.Edges in its current slice order. PreprocessTrees
    // removes fringe edges and putBackNonBranchingTrees appends them after the
    // cyclic cores, so this is intentionally not serialized declaration order.
    let declaration_order: Vec<_> = graph
        .edge_order
        .read()
        .iter()
        .map(|edge_id| edge_id.0 as usize)
        .collect();

    let mut routes = vec![None; graph.edge_records.len()];
    let ovg_budget =
        super::route_search_guard::OvgBuildBudget::for_current_scope(aggregate.clone());
    for (subgraph_index, subgraph) in subgraphs.iter().enumerate() {
        if subgraph.edges.is_empty() {
            continue;
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_SUBGRAPHS") {
            let node_ids = subgraph
                .nodes
                .iter()
                .map(|node| graph.nodes[node.0 as usize].tala_id)
                .collect::<Vec<_>>();
            let edge_ids = subgraph
                .edges
                .iter()
                .map(|edge| graph.edge_records[*edge].tala_id)
                .collect::<Vec<_>>();
            eprintln!(
                "TALA_TRACE_SUBGRAPH index={subgraph_index} nodes={node_ids:?} edges={edge_ids:?}"
            );
        }
        let edge_members = subgraph.edges.iter().copied().collect::<BTreeSet<_>>();
        let subgraph_order = declaration_order
            .iter()
            .copied()
            .filter(|edge| edge_members.contains(edge))
            .collect::<Vec<_>>();
        let nearby_nodes = routing_nearby_nodes(graph, &subgraphs, subgraph_index);
        // SplitSubgraphs constructs a fresh Graph and CopyEntitiesFrom
        // deliberately does not copy CellSize or any lazy route-cost cache.
        let mut routing_graph = graph.clone();
        // Mirror the temporary Graph.Nodes/Edges membership, without
        // renumbering arena storage used by ownership references.
        routing_graph.node_order = subgraph.nodes.clone();
        routing_graph.edge_order.replace_values(
            &subgraph_order
                .iter()
                .map(|&index| crate::EdgeId(index as u32))
                .collect::<Vec<_>>(),
        );
        routing_graph.cell_size = 0.0;
        routing_graph.fresh_cost_owner();
        initialize_scope_routing_costs(&mut routing_graph, &subgraph.nodes, &subgraph_order);
        let subgraph_routes = route_edge_subset(
            &mut routing_graph,
            &subgraph.nodes,
            &nearby_nodes,
            &subgraph_order,
            budget_limit,
            Some(&ovg_budget),
            aggregate.as_ref(),
        )?;
        for (edge, points) in subgraph_routes {
            routes[edge] = Some(points);
        }
    }
    Ok(routes)
}

/// Run the complete edge-routing search and preserve the v0.9.0 cancellation
/// boundary owned by the engine stage. The historical infallible helper stays
/// available to focused geometry tests and diagnostic callers; production
/// pipeline code uses this checked entry point so a cancellation that arrives
/// during OVG search cannot be published as a partially routed graph.
pub(in crate::internal) fn route_edges_with_cancellation(
    graph: &ArenaGraph,
    seed: i64,
) -> Result<Vec<Vec<Point>>, crate::LayoutError> {
    let aggregate = super::route_search_guard::RouteStageWork::new(
        super::route_search_guard::MAX_EDGE_ROUTING_STAGE_WORK_UNITS,
    );
    route_edges_with_cancellation_and_aggregate(graph, seed, aggregate).map(|routes| {
        routes
            .into_iter()
            .map(|route| route.map_or_else(Vec::new, |route| route.read().to_vec()))
            .collect()
    })
}

/// Checked route entry point used by the pipeline so the same aggregate guard
/// can continue through post-routing stages. The ordinary wrapper above keeps
/// the historical API for direct callers while allocating a fresh stage
/// envelope for each standalone invocation.
pub(in crate::internal) fn route_edges_with_cancellation_and_aggregate(
    graph: &ArenaGraph,
    seed: i64,
    aggregate: std::sync::Arc<super::route_search_guard::RouteStageWork>,
) -> Result<Vec<Option<crate::internal::RoutePoints>>, crate::LayoutError> {
    if crate::internal::cancellation_requested() {
        return Err(
            crate::internal::cancellation_layout_error().unwrap_or(crate::LayoutError::Cancelled)
        );
    }
    // Account the stage inventory before any OVG or flavor mutation. This is
    // the small graph-discovery portion of the upstream route-stage envelope;
    // the OVG and flavor guards charge their exact inner work to the same
    // aggregate counter below.
    let inventory = graph.nodes.len() as u64 + graph.edge_order.len() as u64;
    if aggregate.reserve(inventory).is_err() {
        return Err(crate::LayoutError::WorkLimit {
            seed,
            location: "EdgeRouting",
            limit: super::route_search_guard::MAX_EDGE_ROUTING_STAGE_WORK_UNITS,
        });
    }
    let routes = route_edges_with_stage_budget(
        graph,
        Some(super::MAX_ROUTE_SEARCH_WORK_UNITS),
        Some(aggregate.clone()),
    )
    .map_err(|error| match error {
        crate::LayoutError::WorkLimit {
            location, limit, ..
        } => crate::LayoutError::WorkLimit {
            seed,
            location,
            limit,
        },
        error => error,
    })?;
    if crate::internal::cancellation_requested() {
        return Err(
            crate::internal::cancellation_layout_error().unwrap_or(crate::LayoutError::Cancelled)
        );
    }
    if aggregate.exhausted() {
        return Err(crate::LayoutError::WorkLimit {
            seed,
            location: "EdgeRouting",
            limit: super::route_search_guard::MAX_EDGE_ROUTING_STAGE_WORK_UNITS,
        });
    }
    Ok(routes)
}

pub(in crate::internal) fn route_additional_edges(
    graph: &ArenaGraph,
    requested: &[crate::EdgeId],
) -> Result<Vec<Vec<Point>>, crate::LayoutError> {
    route_additional_edges_with_budget(graph, requested, None, 0)
}

pub(in crate::internal) fn route_additional_edges_with_budget(
    graph: &ArenaGraph,
    requested: &[crate::EdgeId],
    budget_limit: Option<u64>,
    seed: i64,
) -> Result<Vec<Vec<Point>>, crate::LayoutError> {
    let ovg_budget = super::route_search_guard::OvgBuildBudget::new();
    route_additional_edges_with_ovg_budget(graph, requested, budget_limit, seed, &ovg_budget, false)
        .map(|updates| {
            let mut routes = graph
                .edge_records
                .iter()
                .map(|edge| edge.route.read().to_vec())
                .collect::<Vec<_>>();
            for (edge, points) in updates {
                routes[edge] = points.read().to_vec();
            }
            routes
        })
}

pub(super) fn route_additional_edges_with_context(
    graph: &mut ArenaGraph,
    requested: &[crate::EdgeId],
    budget_limit: Option<u64>,
    seed: i64,
    context: &dyn crate::internal::limits::Context,
) -> Result<super::finalize::Updates, super::route_stage_guard::RouteStageError> {
    let ovg_budget = super::route_search_guard::OvgBuildBudget::with_context(context);
    // newOVGBuildGuard checks before construction or derived allocations.
    if ovg_budget.check().is_err() {
        return Err(ovg_budget
            .take_context_error()
            .expect("OVG context error")
            .into());
    }
    let result =
        route_additional_edges_from_live_graph(graph, requested, budget_limit, seed, &ovg_budget);
    if let Some(error) = ovg_budget.take_context_error() {
        return Err(error.into());
    }
    result.map_err(|error| {
        let cause = match error {
            crate::LayoutError::Cancelled => crate::internal::limits::ContextError::Cancelled,
            crate::LayoutError::DeadlineExceeded => {
                crate::internal::limits::ContextError::DeadlineExceeded
            }
            error => return super::route_stage_guard::RouteStageError::Kernel(error),
        };
        crate::internal::limits::Error::Context {
            location: "EdgeRouting".into(),
            cause,
        }
        .into()
    })
}

fn route_additional_edges_with_ovg_budget(
    graph: &ArenaGraph,
    requested: &[crate::EdgeId],
    budget_limit: Option<u64>,
    seed: i64,
    ovg_budget: &super::route_search_guard::OvgBuildBudget<'_>,
    _use_live_owner: bool,
) -> Result<super::finalize::Updates, crate::LayoutError> {
    let mut working = graph.clone();
    working.cell_size = 0.0;
    let scope_nodes = working.graph_node_order();
    let edges = working
        .edge_order
        .read()
        .iter()
        .map(|e| e.0 as usize)
        .collect::<Vec<_>>();
    initialize_scope_routing_costs(&mut working, &scope_nodes, &edges);
    route_additional_edges_from_live_graph(&mut working, requested, budget_limit, seed, ovg_budget)
}

fn route_additional_edges_from_live_graph(
    graph: &mut ArenaGraph,
    requested: &[crate::EdgeId],
    budget_limit: Option<u64>,
    seed: i64,
    ovg_budget: &super::route_search_guard::OvgBuildBudget<'_>,
) -> Result<super::finalize::Updates, crate::LayoutError> {
    let requested = requested
        .iter()
        .map(|edge| edge.0 as usize)
        .collect::<Vec<_>>();
    let requested_set = requested.iter().copied().collect::<BTreeSet<_>>();
    let fixed_routes = graph
        .member_edge_entries()
        .map(|(index, edge)| {
            if !requested_set.contains(&index) {
                edge.route.read().clone()
            } else {
                Default::default()
            }
        })
        .collect::<Vec<_>>();
    let scope_nodes = graph.graph_node_order();
    #[cfg(feature = "diagnostic-traces")]
    trace_additional_routing_costs(graph, &scope_nodes);
    route_additional_edge_subset(
        graph,
        &scope_nodes,
        &requested,
        &fixed_routes,
        budget_limit,
        seed,
        ovg_budget,
    )
}

#[cfg(feature = "diagnostic-traces")]
fn trace_additional_routing_costs(graph: &ArenaGraph, nodes: &[NodeId]) {
    use std::fmt::Write as _;
    if std::env::var("D2_TRACE_ROUTER_COSTS").as_deref() != Ok("1") {
        return;
    }
    let mut snapshot = String::new();
    writeln!(
        &mut snapshot,
        "ROUTER_COSTS begin nodes={} edges={}",
        nodes.len(),
        graph.edge_order.len()
    )
    .unwrap();
    writeln!(
        &mut snapshot,
        "ROUTER_COST_VALUES cell={:016x} crossing={:016x} turn={:016x} noncenter={:016x} root={}",
        graph.cell_size.to_bits(),
        graph.raw_crossing_cost().to_bits(),
        graph.raw_turn_cost().to_bits(),
        graph.raw_non_center_port_cost().to_bits(),
        graph.root_hierarchy
    )
    .unwrap();
    for id in nodes {
        let node = &graph.nodes[id.0 as usize];
        let edges = node
            .edges
            .read()
            .iter()
            .map(|edge| graph.edge_records[edge.0 as usize].tala_id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            &mut snapshot,
            "ROUTER_COST_NODE node={} container={} hierarchy={} edges={}",
            node.tala_id,
            node.is_container,
            node.hierarchy.is_some(),
            edges
        )
        .unwrap();
    }
    writeln!(&mut snapshot, "ROUTER_COSTS end").unwrap();
    eprint!("{snapshot}");
}

#[cfg(feature = "diagnostic-traces")]
fn trace_ovg_import_stage(
    graph: &ArenaGraph,
    visibility: &super::visibility::VisibilityGraph,
    ports: &BTreeMap<NodeId, Vec<Port>>,
    stage: &str,
) {
    if std::env::var("D2_TRACE_OVG_STAGE_SCOPE").ok().as_deref()
        != Some(format!("{}:{}", graph.nodes.len(), graph.edge_records.len()).as_str())
    {
        return;
    }
    if stage == "base" {
        let inventory = graph.node_order.iter().map(|id| {
   let node = &graph.nodes[id.0 as usize]; let rect = node_rect(graph, *id).unwrap();
   let points = ports.get(id).into_iter().flatten().map(|port| [format!("{:016x}",port.point.x.to_bits()),format!("{:016x}",port.point.y.to_bits())]).collect::<Vec<_>>();
   serde_json::json!({"cluster":node.cluster.is_some(),"tree":graph.tree_routing_nodes.contains_key(id),"node":node.tala_id,"parent":node.container.map_or(0,|parent|graph.nodes[parent.0 as usize].tala_id),"fixed":node.fixed_top_left.is_some(),"rect":[format!("{:016x}",rect.origin.x.to_bits()),format!("{:016x}",rect.origin.y.to_bits()),format!("{:016x}",rect.size.width.to_bits()),format!("{:016x}",rect.size.height.to_bits())],"ports":points})
  }).collect::<Vec<_>>();
        eprintln!(
            "{}",
            serde_json::json!({"event":"ovg-port-inventory","scope_nodes":graph.node_order.len(),"scope_edges":graph.edge_order.len(),"nodes":inventory})
        );
    }
    let edges = visibility
        .segments()
        .iter()
        .map(|segment| {
            [
                if segment[0].x == segment[1].x {
                    "vertical"
                } else if segment[0].y == segment[1].y {
                    "horizontal"
                } else {
                    "other"
                }
                .to_string(),
                format!("{:016x}", segment[0].x.to_bits()),
                format!("{:016x}", segment[0].y.to_bits()),
                format!("{:016x}", segment[1].x.to_bits()),
                format!("{:016x}", segment[1].y.to_bits()),
            ]
        })
        .collect::<Vec<_>>();
    eprintln!(
        "{}",
        serde_json::json!({"event":"ovg-import-stage","stage":stage,"scope_nodes":graph.node_order.len(),"scope_edges":graph.edge_order.len(),"edges":edges})
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    fn label_collision_guard_observations(
        candidate_route: &[Point],
        prior_route: &[Point],
    ) -> Vec<(Option<f64>, u64, bool)> {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        let edge = input.add_edge(crate::Edge { source, target });
        input.add_edge(crate::Edge { source, target });
        input.set_edge_arrowhead_labels(
            edge,
            crate::EdgeArrowheadLabels {
                source: Some(crate::ArrowheadLabel {
                    text: "endpoint".into(),
                    size: crate::Size {
                        width: 20.0,
                        height: 10.0,
                    },
                    color: None,
                }),
                target: None,
            },
        );
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 100.0, y: 0.0 });
        // The focused pristine Go graph has cell size and TurnCost both 20.
        graph.set_turn_cost(20.0);
        graph.routing_costs = graph.raw_routing_costs();
        let boxes = graph
            .nodes
            .iter()
            .map(|node| (node.input_id, node_rect(&graph, node.input_id).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let visibility = super::super::visibility::VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.node_order,
            &graph.node_order,
            std::iter::empty(),
        );
        let mut interactions = RouteInteractionIndex::new(&visibility, &graph);
        // Match the Go witness's literal accepted Route inventory. These
        // three OVG nodes give exactly two segments; publication is a separate
        // contract and is not charged by this guarded cost helper.
        interactions.accepted_edge_order.push(1);
        interactions
            .routes
            .push(super::super::published_route::PublishedRoute::new(
                1,
                prior_route.to_vec(),
                None,
                None,
                Vec::new(),
            ));
        (2..=3)
            .map(|limit| {
                let guard = LocalRouteSearchWork::new(limit);
                let cost = interactions.candidate_arrowhead_label_cost_for_end_guarded(
                    &graph,
                    0,
                    candidate_route,
                    false,
                    &[],
                    Some(&guard),
                );
                (cost, guard.used(), guard.exhausted())
            })
            .collect()
    }

    #[test]
    fn label_collision_finite_overflow_preserves_guard_errors_and_work() {
        let observations = label_collision_guard_observations(
            &[Point { x: -5.0, y: 16.0 }, Point { x: 95.0, y: 16.0 }],
            &[
                Point { x: 1e308, y: 1e308 },
                Point {
                    x: -1e308,
                    y: 5e307,
                },
                Point {
                    x: -5e307,
                    y: 1e308,
                },
            ],
        );
        assert_eq!(observations, vec![(None, 2, true), (Some(0.0), 3, false)]);
    }

    #[test]
    fn label_collision_nan_box_preserves_guard_errors_and_work() {
        let observations = label_collision_guard_observations(
            &[Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 0.0 }],
            &[
                Point { x: 0.0, y: 0.0 },
                Point { x: 1.0, y: 0.0 },
                Point { x: 2.0, y: 0.0 },
            ],
        );
        assert_eq!(observations, vec![(None, 2, true), (Some(0.0), 3, false)]);
    }

    #[test]
    fn label_collision_finite_controls_preserve_guard_work() {
        let candidate = [Point { x: -5.0, y: 16.0 }, Point { x: 95.0, y: 16.0 }];
        assert_eq!(
            label_collision_guard_observations(
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
            ),
            vec![(None, 2, true), (Some(0.0), 3, false)],
        );
        assert_eq!(
            label_collision_guard_observations(
                &candidate,
                &[
                    Point { x: 10.0, y: 30.0 },
                    Point { x: 10.0, y: -20.0 },
                    Point { x: 30.0, y: -20.0 },
                ],
            ),
            vec![(Some(20.0), 2, false), (Some(20.0), 2, false)],
        );
    }

    #[test]
    fn coincident_arrowhead_label_scoring_keeps_guard_visits_and_errors() {
        let (mut graph, visibility, _, _) = route_interaction_fixture();
        graph.edge_records[0].source_arrowhead_label = Some(crate::ArrowheadLabel {
            text: "port".into(),
            size: crate::Size {
                width: 20.0,
                height: 10.0,
            },
            color: None,
        });
        let interactions = RouteInteractionIndex::new(&visibility, &graph);
        for points in [
            vec![Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 0.0 }],
            vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 0.0, y: 0.0 },
                Point { x: 100.0, y: 0.0 },
            ],
        ] {
            let limited = LocalRouteSearchWork::new(0);
            assert!(
                interactions
                    .candidate_arrowhead_label_cost_for_end_guarded(
                        &graph,
                        0,
                        &points,
                        false,
                        &graph.node_order,
                        Some(&limited),
                    )
                    .is_none()
            );
            assert!(limited.exhausted());
            let full = LocalRouteSearchWork::new(2);
            assert_eq!(
                interactions.candidate_arrowhead_label_cost_for_end_guarded(
                    &graph,
                    0,
                    &points,
                    false,
                    &graph.node_order,
                    Some(&full),
                ),
                Some(0.0)
            );
            assert_eq!(full.used(), 2);
        }
    }

    #[test]
    fn candidate_arrowhead_scoring_charges_nodes_and_propagates_limits() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        let edge = input.add_edge(crate::Edge { source, target });
        input.set_edge_arrowhead_labels(
            edge,
            crate::EdgeArrowheadLabels {
                source: Some(crate::ArrowheadLabel {
                    text: "port".into(),
                    size: crate::Size {
                        width: 20.0,
                        height: 10.0,
                    },
                    color: None,
                }),
                target: None,
            },
        );
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 100.0, y: 0.0 });
        let boxes = graph
            .nodes
            .iter()
            .map(|node| (node.input_id, node_rect(&graph, node.input_id).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let visibility = super::super::visibility::VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.node_order,
            &graph.node_order,
            std::iter::empty(),
        );
        let interactions = RouteInteractionIndex::new(&visibility, &graph);
        let points = [Point { x: 20.0, y: 10.0 }, Point { x: 100.0, y: 10.0 }];
        let full = LocalRouteSearchWork::new(2);
        let scored = interactions
            .candidate_arrowhead_label_cost_for_end_guarded(
                &graph,
                0,
                &points,
                false,
                &graph.node_order,
                Some(&full),
            )
            .unwrap();
        assert_eq!(full.used(), 2);
        assert_eq!(
            scored,
            interactions.candidate_arrowhead_label_cost_for_end(&graph, 0, &points, false)
        );
        for limit in 0..2 {
            let guard = LocalRouteSearchWork::new(limit);
            assert!(
                interactions
                    .candidate_arrowhead_label_cost_for_end_guarded(
                        &graph,
                        0,
                        &points,
                        false,
                        &graph.node_order,
                        Some(&guard)
                    )
                    .is_none()
            );
            assert!(guard.exhausted());
        }
        let no_target_label = LocalRouteSearchWork::new(0);
        assert_eq!(
            interactions.candidate_arrowhead_label_cost_for_end_guarded(
                &graph,
                0,
                &points,
                true,
                &graph.node_order,
                Some(&no_target_label)
            ),
            Some(0.0)
        );
        assert!(!no_target_label.exhausted());
        // Slingshot extracts both interior points, filters both endpoint
        // nodes, then scores the one present arrowhead against both nodes.
        let proposal = LocalRouteSearchWork::new(6);
        assert_eq!(
            interactions.candidate_arrowhead_label_cost_for_route_guarded(
                &graph,
                0,
                &points,
                Some(&proposal)
            ),
            Some(scored)
        );
        assert_eq!(proposal.used(), 6);
        for limit in 0..6 {
            let guard = LocalRouteSearchWork::new(limit);
            assert!(
                interactions
                    .candidate_arrowhead_label_cost_for_route_guarded(
                        &graph,
                        0,
                        &points,
                        Some(&guard)
                    )
                    .is_none()
            );
            assert!(guard.exhausted());
        }
    }

    #[test]
    fn source_connector_scores_node_labels_before_queue_publication() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        let labelled = input.add_node(routing_test_node("labelled", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        for (node, x, y) in [
            (source, 0.0, 0.0),
            (target, 100.0, 0.0),
            (labelled, 500.0, 500.0),
        ] {
            graph.set_position(node, Point { x, y });
        }
        graph.set_turn_cost(23.0);
        graph.routing_costs = graph.raw_routing_costs();
        let boxes = graph
            .nodes
            .iter()
            .map(|node| (node.input_id, node_rect(&graph, node.input_id).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let all_ports = graph
            .nodes
            .iter()
            .flat_map(|node| {
                super::super::ports(boxes[&node.input_id], node.shape)
                    .into_iter()
                    .map(move |port| (node.input_id, port))
            })
            .collect::<Vec<_>>();
        let source_port = all_ports
            .iter()
            .find(|(node, port)| *node == source && port.point == Point { x: 20.0, y: 10.0 })
            .unwrap()
            .1;
        let target_port = all_ports
            .iter()
            .find(|(node, port)| *node == target && port.point == Point { x: 100.0, y: 10.0 })
            .unwrap()
            .1;
        let visibility = super::super::visibility::VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.node_order,
            &graph.node_order,
            all_ports.into_iter(),
        );
        let mut interactions = RouteInteractionIndex::new(&visibility, &graph);
        let route = |interactions: &RouteInteractionIndex,
                     budget: Option<&LocalRouteSearchWork>| {
            visibility.route_between_connectors_with_budget(
                &graph,
                &[(source_port, 0.0)],
                &[(target_port, 0.0)],
                source,
                target,
                Some(0),
                interactions,
                &boxes,
                false,
                budget,
            )
        };
        let baseline = route(&interactions, None).unwrap();
        // This box intersects only the source center-to-port connector, not
        // the ordinary segment between the two endpoint boundaries.
        interactions.node_label_rects.push((
            labelled,
            Rect {
                origin: Point { x: 14.0, y: 9.0 },
                size: crate::Size {
                    width: 2.0,
                    height: 2.0,
                },
            },
        ));
        let work = LocalRouteSearchWork::new(100_000);
        let labelled_route = route(&interactions, Some(&work)).unwrap();
        assert_eq!(labelled_route.points, baseline.points);
        assert_eq!(labelled_route.cost, baseline.cost + graph.raw_turn_cost());
        for limit in 0..work.used() {
            let guard = LocalRouteSearchWork::new(limit);
            assert!(route(&interactions, Some(&guard)).is_none());
            assert!(guard.exhausted());
        }
    }

    #[test]
    fn target_connector_scores_node_labels_before_queue_publication() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        let labelled = input.add_node(routing_test_node("labelled", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        for (node, x, y) in [
            (source, 0.0, 0.0),
            (target, 100.0, 0.0),
            (labelled, 500.0, 500.0),
        ] {
            graph.set_position(node, Point { x, y });
        }
        graph.set_turn_cost(23.0);
        graph.routing_costs = graph.raw_routing_costs();
        let boxes = graph
            .nodes
            .iter()
            .map(|node| (node.input_id, node_rect(&graph, node.input_id).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let all_ports = graph
            .nodes
            .iter()
            .flat_map(|node| {
                super::super::ports(boxes[&node.input_id], node.shape)
                    .into_iter()
                    .map(move |port| (node.input_id, port))
            })
            .collect::<Vec<_>>();
        let source_port = all_ports
            .iter()
            .find(|(node, port)| *node == source && port.point == Point { x: 20.0, y: 10.0 })
            .unwrap()
            .1;
        let target_port = all_ports
            .iter()
            .find(|(node, port)| *node == target && port.point == Point { x: 100.0, y: 10.0 })
            .unwrap()
            .1;
        let visibility = super::super::visibility::VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.node_order,
            &graph.node_order,
            all_ports.into_iter(),
        );
        let mut interactions = RouteInteractionIndex::new(&visibility, &graph);
        let route = |interactions: &RouteInteractionIndex,
                     budget: Option<&LocalRouteSearchWork>| {
            visibility.route_between_connectors_with_budget(
                &graph,
                &[(source_port, 0.0)],
                &[(target_port, 0.0)],
                source,
                target,
                Some(0),
                interactions,
                &boxes,
                false,
                budget,
            )
        };
        let baseline = route(&interactions, None).unwrap();
        // This box intersects only the target port-to-center connector, not
        // the ordinary segment between the two endpoint boundaries.
        interactions.node_label_rects.push((
            labelled,
            Rect {
                origin: Point { x: 104.0, y: 9.0 },
                size: crate::Size {
                    width: 2.0,
                    height: 2.0,
                },
            },
        ));
        let work = LocalRouteSearchWork::new(100_000);
        let labelled_route = route(&interactions, Some(&work)).unwrap();
        assert_eq!(labelled_route.points, baseline.points);
        assert_eq!(labelled_route.cost, baseline.cost + graph.raw_turn_cost());
        for limit in 0..work.used() {
            let guard = LocalRouteSearchWork::new(limit);
            assert!(route(&interactions, Some(&guard)).is_none());
            assert!(guard.exhausted());
        }
    }

    #[test]
    fn content_shapes_do_not_become_ordinary_node_label_obstacles() {
        for shape in [
            crate::ShapeKind::Text,
            crate::ShapeKind::Code,
            crate::ShapeKind::Class,
            crate::ShapeKind::SqlTable,
        ] {
            let mut input = crate::Graph::default();
            let mut content = routing_test_node("content", 20.0, 20.0);
            content.shape = shape;
            content.label_size = Some(crate::Size {
                width: 30.0,
                height: 10.0,
            });
            let node = input.add_node(content);
            input.set_node_label(node, "nonempty content");
            let graph = ArenaGraph::from_input(&input);
            assert!(
                !graph.nodes[node.0 as usize].label_text_present,
                "{shape:?}"
            );
            // Source adapter.HasLabel leaves the layout label nil, while
            // original graph dimensions remain available to the renderer.
            assert!(graph.nodes[node.0 as usize].label_size.is_none());
            assert!(input.node(node).unwrap().label_size.is_some());
        }
    }

    #[test]
    fn explicitly_empty_node_labels_do_not_become_routing_obstacles() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        let mut blank = routing_test_node("blank", 20.0, 20.0);
        blank.label_size = Some(crate::Size {
            width: 30.0,
            height: 10.0,
        });
        blank.label_position = crate::LabelPosition::InsideMiddleCenter;
        let blank = input.add_node(blank);
        input.set_node_label(blank, "");
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        for node in &mut graph.nodes {
            node.position = Some(Point { x: 0.0, y: 0.0 });
        }
        let boxes = graph
            .nodes
            .iter()
            .map(|node| (node.input_id, node_rect(&graph, node.input_id).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let visibility = super::super::visibility::VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.node_order,
            &graph.node_order,
            std::iter::empty(),
        );
        assert!(!graph.nodes[blank.0 as usize].label_text_present);
        assert!(graph.nodes[blank.0 as usize].label_size.is_none());
        assert!(input.node(blank).unwrap().label_size.is_some());
        let mut index = RouteInteractionIndex::new_with_node_labels(&visibility, &graph, true);
        assert!(index.node_label_rects.is_empty());
        let work = LocalRouteSearchWork::new(3);
        index
            .prepare_search_node_labels(&graph, 0, Some(&work))
            .unwrap();
        assert_eq!(work.used(), 3); // all graph nodes visited before filtering
        assert!(index.node_label_rects.is_empty());
        let work = LocalRouteSearchWork::new(0);
        assert_eq!(
            index.apply_label_obstacle_cost_guarded(
                &graph,
                0,
                [Point { x: 0.0, y: 0.0 }, Point { x: 20.0, y: 0.0 }],
                7.0,
                Some(&work)
            ),
            Some(7.0)
        );
        assert_eq!(work.used(), 0);
    }

    #[test]
    fn compact_search_labels_preserve_storage_order_and_charged_visits() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        input.add_node(routing_test_node("unlabelled", 20.0, 20.0));
        let first = input.add_node(routing_test_node("first", 20.0, 20.0));
        input.add_node(routing_test_node("unrelated", 20.0, 20.0));
        let second = input.add_node(routing_test_node("second", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        for node in [source, first, second, target] {
            input.set_node_label(node, "label");
        }
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        for node in [source, first, second, target] {
            graph.nodes[node.0 as usize].label_size = Some(crate::Size {
                width: 10.0,
                height: 10.0,
            });
            graph.set_position(node, Point { x: 0.0, y: 0.0 });
        }
        graph.set_turn_cost(0.7);
        graph.routing_costs = graph.raw_routing_costs();
        // Split components retain storage IDs independently of member order.
        graph.node_order = vec![target, second, first, source];
        let visibility = super::super::visibility::VisibilityGraph::from_ports(
            &graph,
            &BTreeMap::new(),
            &[],
            &[],
            std::iter::empty(),
        );
        let mut index = RouteInteractionIndex::new_with_node_labels(&visibility, &graph, true);
        let before = index.node_label_rects.clone();
        let short = LocalRouteSearchWork::new(3);
        assert!(
            index
                .prepare_search_node_labels(&graph, 0, Some(&short))
                .is_none()
        );
        assert_eq!(index.node_label_rects, before);
        let preparation = LocalRouteSearchWork::new(4);
        index
            .prepare_search_node_labels(&graph, 0, Some(&preparation))
            .unwrap();
        assert_eq!(preparation.used(), 4);
        assert_eq!(
            index
                .node_label_rects
                .iter()
                .map(|&(id, _)| id)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
        let rect = index.node_label_rects[0].1;
        let segment = [
            rect.origin,
            Point {
                x: rect.right(),
                y: rect.origin.y,
            },
        ];
        let exact = LocalRouteSearchWork::new(2);
        let cost = index
            .apply_label_obstacle_cost_guarded(&graph, 0, segment, 7.0, Some(&exact))
            .unwrap();
        assert_eq!(
            cost.to_bits(),
            ((7.0 + graph.routing_costs.1) + graph.routing_costs.1).to_bits()
        );
        assert_eq!(exact.used(), 2);
        for limit in 0..2 {
            let short = LocalRouteSearchWork::new(limit);
            assert!(
                index
                    .apply_label_obstacle_cost_guarded(&graph, 0, segment, 7.0, Some(&short))
                    .is_none()
            );
            assert_eq!(short.used(), limit);
            assert!(short.exhausted());
        }
        let disabled = RouteInteractionIndex::new(&visibility, &graph);
        assert!(disabled.node_label_rects.is_empty());
        let zero = LocalRouteSearchWork::new(0);
        assert_eq!(
            disabled.apply_label_obstacle_cost_guarded(&graph, 0, segment, 7.0, Some(&zero)),
            Some(7.0)
        );
        assert_eq!(zero.used(), 0);
    }

    #[test]
    fn routing_costs_without_edges_have_no_center_port_floor() {
        let mut input = crate::Graph::default();
        input.add_node(routing_test_node("isolated", 120.0, 90.0));
        let mut graph = ArenaGraph::from_input(&input);
        initialize_standalone_routing_costs(&mut graph);
        assert_eq!(
            graph.raw_non_center_port_cost().to_bits(),
            0.0_f64.to_bits()
        );
    }

    #[test]
    fn router_captures_semantic_zero_while_the_raw_port_sentinel_is_negative() {
        let mut input = crate::Graph::default();
        input.add_node(routing_test_node("isolated", 120.0, 90.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.restore_routing_costs((-0.0, -0.0, -0.0));
        initialize_standalone_routing_costs(&mut graph);
        assert_eq!(
            graph.raw_non_center_port_cost().to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(graph.routing_costs.2.to_bits(), 0);
        assert_eq!(graph.routing_costs.0.to_bits(), 0);
        assert_eq!(graph.routing_costs.1.to_bits(), 0);
    }

    #[test]
    fn routing_costs_with_only_containers_keep_a_finite_center_port_penalty() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 100.0, y: 0.0 });
        graph.nodes[source.0 as usize].is_container = true;
        graph.nodes[target.0 as usize].is_container = true;
        let length = graph.sized_distance_to(source, target).max(60.0);
        initialize_standalone_routing_costs(&mut graph);
        assert_eq!(
            graph.raw_non_center_port_cost().to_bits(),
            (0.04287499999999999_f64 * length).to_bits()
        );
        assert_eq!(
            graph.raw_crossing_cost().to_bits(),
            (0.110592_f64 * length).to_bits()
        );
    }

    #[test]
    fn go_ovg_map_keys_coalesce_signed_zero() {
        let negative_zero = Point { x: -0.0, y: 12.0 };
        let positive_zero = Point { x: 0.0, y: 12.0 };

        assert_eq!(
            interaction_point_key(negative_zero),
            interaction_point_key(positive_zero)
        );
        assert_eq!(
            super::super::go_float_map_key(-0.0),
            super::super::go_float_map_key(0.0)
        );
    }

    fn routing_test_node(name: &str, width: f64, height: f64) -> crate::Node {
        crate::Node {
            d2_near_key: None,
            external_id: name.into(),
            size: crate::Size { width, height },
            declared_size: None,
            d2_authored_geometry: None,
            label_size: None,
            font_size: None,
            label_position: crate::LabelPosition::Unset,
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
            content_insets: crate::Insets::uniform(60.0),
            layout_margins: crate::Insets::uniform(0.0),
            external_label: None,
            icon_position: None,
            has_icon: false,
            label_aware_grid: false,
            packed_grid: false,
            content_alignment: crate::ContentAlignment::Padding,
            port_spread: 0.0,
            person: false,
            is_3d: false,
            is_multiple: false,
            shape: ShapeKind::Rectangle,
        }
    }

    fn route_interaction_fixture() -> (
        ArenaGraph,
        super::super::visibility::VisibilityGraph,
        NodeId,
        NodeId,
    ) {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 100.0, y: 0.0 });
        let boxes = graph
            .nodes
            .iter()
            .map(|node| {
                (
                    node.input_id,
                    Rect {
                        origin: node.position.unwrap(),
                        size: node.rect.size,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let nodes = graph.graph_node_order();
        let ports = graph
            .nodes
            .iter()
            .flat_map(|node| {
                crate::internal::routing::ports(boxes[&node.input_id], node.shape)
                    .into_iter()
                    .map(move |port| (node.input_id, port))
            })
            .collect::<Vec<_>>();
        let visibility = super::super::visibility::VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &nodes,
            &nodes,
            ports.into_iter(),
        );
        (graph, visibility, source, target)
    }

    #[test]
    fn static_edge_cost_uses_object_identity_with_equal_current_geometry() {
        use crate::internal::owner_points::SharedOwnerPoint;
        let (graph, visibility, _, _) = route_interaction_fixture();
        let mut build = super::super::ovg_ports::PortBuild::default();
        let a = SharedOwnerPoint::new(Point { x: 100.0, y: 0.0 });
        let b = SharedOwnerPoint::new(Point { x: 110.0, y: 0.0 });
        for point in [&a, &b] {
            let node = build.allocate_node(point.get());
            build.nodes[node].write().point_ref = Some(point.clone());
        }
        build.connect(0, 1, None).unwrap();
        a.set(Point { x: 100.0, y: 20.0 });
        b.set(Point { x: 110.0, y: 20.0 });
        build.connect(0, 1, None).unwrap();
        let mut index = RouteInteractionIndex::new(&visibility, &graph);
        index.static_index = super::super::static_edge_index::StaticEdgeIndex::from_build(&build);
        let accepted = [Point { x: 100.0, y: 21.0 }, Point { x: 110.0, y: 21.0 }];
        assert!(index.add_route_with_budget(&graph, 0, &accepted, "test", None));
        let query = [a.get(), b.get()];
        assert_eq!(
            index.static_index.edge_points(0),
            index.static_index.edge_points(1)
        );
        assert_eq!(
            index.static_index.nearby.iter().collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(
            index.apply_route_interaction_cost_for_edge(&graph, 0, query, 7.0, Some(0), None),
            Some(7.0)
        );
        assert_eq!(
            index.apply_route_interaction_cost_for_edge(&graph, 0, query, 7.0, Some(1), None),
            Some(10_000_000.0)
        );
    }

    #[test]
    fn routed_crossing_query_reads_shared_imported_points_after_publication() {
        let (graph, mut visibility, _, _) = route_interaction_fixture();
        let points = [Point { x: 350.0, y: 300.0 }, Point { x: 350.0, y: 500.0 }];
        graph.edge_records[0]
            .route
            .replace(RoutePoints::literal(&points));
        let first = graph.edge_records[0].route.slot(0).unwrap();
        let second = graph.edge_records[0].route.slot(1).unwrap();
        let route = visibility
            .add_existing_edge_route(&graph, crate::EdgeId(0), None)
            .unwrap()
            .unwrap();
        let mut index = RouteInteractionIndex::new(&visibility, &graph);
        index
            .add_imported_route(&graph, &visibility, &route, "test", None)
            .unwrap();
        let query = [Point { x: 301.0, y: 401.0 }, Point { x: 501.0, y: 401.0 }];
        assert!(index.crosses_routed_segment(query));
        first.set(Point { x: 700.0, y: 600.0 });
        second.set(Point { x: 700.0, y: 800.0 });
        assert!(!index.crosses_routed_segment(query));
        assert_eq!(index.routed_set.vertical_axes, vec![350.0]);
    }

    #[test]
    fn published_route_occurrences_keep_independent_endpoints_and_live_points() {
        let (graph, mut visibility, source, target) = route_interaction_fixture();
        let a = [Point { x: 301.0, y: 401.0 }, Point { x: 501.0, y: 401.0 }];
        let b = [Point { x: 321.0, y: 601.0 }, Point { x: 521.0, y: 601.0 }];
        graph.edge_records[0]
            .route
            .replace(RoutePoints::literal(&a));
        let shared = graph.edge_records[0].route.slot(0).unwrap();
        let first = visibility
            .add_existing_edge_route(&graph, crate::EdgeId(0), None)
            .unwrap()
            .unwrap();
        graph.edge_records[0]
            .route
            .replace(RoutePoints::literal(&b));
        let second = visibility
            .add_existing_edge_route(&graph, crate::EdgeId(0), None)
            .unwrap()
            .unwrap();
        let mut index = RouteInteractionIndex::new(&visibility, &graph);
        index
            .add_imported_route(&graph, &visibility, &first, "test", None)
            .unwrap();
        index
            .add_imported_route(&graph, &visibility, &second, "test", None)
            .unwrap();
        index
            .add_generated_route_with_node_ids(&graph, 0, &a, &first.nodes, None, "test", None)
            .unwrap();
        let changed = Point { x: 333.0, y: 444.0 };
        shared.set(changed);
        assert_eq!(index.routes.len(), 3);
        assert_eq!(index.routes[0].point(1), changed);
        assert_eq!(index.routes[1].point(1), b[0]);
        assert_eq!(index.routes[2].point(1), changed);
        assert_eq!(index.routes[0].endpoints, Some(a));
        assert_eq!(index.routes[1].endpoints, Some(b));
        assert_eq!(index.routes[2].endpoints, Some(a));
        assert_eq!(
            index.point_to_routes[&interaction_point_key(a[0])],
            vec![0, 2]
        );
        assert!(
            !index
                .point_to_routes
                .contains_key(&interaction_point_key(changed))
        );
        let ports = BTreeMap::from([(source, Vec::new()), (target, Vec::new())]);
        let used = index
            .used_ports(
                &graph,
                0,
                &visibility,
                &ports,
                &BTreeSet::new(),
                &BTreeSet::new(),
                false,
                None,
            )
            .unwrap();
        assert_eq!(
            used.duplicate_source,
            BTreeSet::from([interaction_point_key(a[0]), interaction_point_key(b[0])])
        );
        let work = LocalRouteSearchWork::new(15);
        let materialized = index.materialize_quick_route_records(Some(&work)).unwrap();
        assert_eq!(work.used(), 15);
        assert_eq!(
            materialized,
            vec![
                (0, vec![changed, a[1]]),
                (0, b.to_vec()),
                (0, vec![changed, a[1]])
            ]
        );
        for limit in 0..15 {
            let work = LocalRouteSearchWork::new(limit);
            assert!(index.materialize_quick_route_records(Some(&work)).is_none());
            assert_eq!(work.used(), limit / 5 * 5);
        }
    }

    #[test]
    fn overlapping_route_set_deduplicates_route_identity_instead_of_edge_identity() {
        let (graph, mut visibility, _, _) = route_interaction_fixture();
        let points = [Point { x: 301.0, y: 401.0 }, Point { x: 501.0, y: 401.0 }];
        graph.edge_records[0]
            .route
            .replace(RoutePoints::literal(&points));
        let first = visibility
            .add_existing_edge_route(&graph, crate::EdgeId(0), None)
            .unwrap()
            .unwrap();
        let second = visibility
            .add_existing_edge_route(&graph, crate::EdgeId(0), None)
            .unwrap()
            .unwrap();
        let mut index = RouteInteractionIndex::new(&visibility, &graph);
        for route in [&first, &second] {
            index
                .add_imported_route(&graph, &visibility, route, "test", None)
                .unwrap();
        }
        assert_eq!(
            index.overlapping_route_indices(points[0], points[1], None),
            Some(vec![0, 1])
        );
        assert_eq!(
            index
                .routes
                .iter()
                .map(|route| route.edge)
                .collect::<Vec<_>>(),
            vec![0, 0]
        );
        let work = LocalRouteSearchWork::new(1000);
        assert_eq!(
            index.overlapping_route_indices(points[0], points[1], Some(&work)),
            Some(vec![0, 1])
        );
        assert_eq!(work.used(), 8); // one bucket entry, one overlap, 2 outer and 4 inner route references
        for limit in 0..8 {
            let work = LocalRouteSearchWork::new(limit);
            assert!(
                index
                    .overlapping_route_indices(points[0], points[1], Some(&work))
                    .is_none()
            );
            assert_eq!(work.used(), limit);
        }
    }

    #[test]
    fn quick_line_scores_each_materialized_route_occurrence() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 50.0, y: 0.0 });
        let crossing = |x| vec![Point { x, y: -100.0 }, Point { x, y: 100.0 }];
        let records = vec![(1, crossing(30.0)), (1, crossing(40.0))];
        let both = route_line::route_line_with_route_records(&graph, 0, &records, None).unwrap();
        let one =
            route_line::route_line_with_route_records(&graph, 0, &records[1..], None).unwrap();
        assert_eq!(both.source.point, one.source.point);
        assert_eq!(both.target.point, one.target.point);
        assert!((both.cost - one.cost - 0.110592).abs() < 1e-12);
        let work = LocalRouteSearchWork::new(100000);
        let guarded =
            route_line::route_line_with_route_records(&graph, 0, &records, Some(&work)).unwrap();
        assert_eq!(guarded.cost, both.cost);
    }

    #[test]
    fn imported_route_handoff_retains_endpoint_copies_and_reads_current_objects() {
        let (mut graph, mut visibility, source, target) = route_interaction_fixture();
        let original = [Point { x: 301.0, y: 401.0 }, Point { x: 501.0, y: 401.0 }];
        graph.edge_records[0]
            .route
            .replace(RoutePoints::literal(&original));
        let imported = visibility
            .add_existing_edge_route(&graph, crate::EdgeId(0), None)
            .unwrap()
            .unwrap();
        let first = graph.edge_records[0].route.slot(0).unwrap();
        let changed = Point { x: 321.0, y: 421.0 };
        first.set(changed);
        // Moving graph centers after import cannot replace the retained nodes.
        graph.set_position(source, Point { x: 900.0, y: 900.0 });
        assert_eq!(imported.endpoints, original);
        assert_eq!(visibility.point(imported.nodes[1]), changed);
        let mut interactions = RouteInteractionIndex::new(&visibility, &graph);
        interactions
            .add_imported_route(&graph, &visibility, &imported, "test", None)
            .unwrap();
        let published = &interactions.routes[0];
        let points = published.points();
        let identities = &published.identities;
        assert_eq!(
            &*points,
            &[
                Point { x: 10.0, y: 10.0 },
                changed,
                original[1],
                Point { x: 110.0, y: 10.0 }
            ]
        );
        assert_eq!(identities.as_ref().unwrap(), &imported.nodes);
        assert_eq!(interactions.routes[0].endpoints, Some(original));
        assert!(
            interactions
                .point_to_routes
                .contains_key(&interaction_point_key(changed))
        );
        assert!(
            !interactions
                .point_to_routes
                .contains_key(&interaction_point_key(original[0]))
        );
        let ports = BTreeMap::from([(source, Vec::new()), (target, Vec::new())]);
        let used = interactions
            .used_ports(
                &graph,
                0,
                &visibility,
                &ports,
                &BTreeSet::new(),
                &BTreeSet::new(),
                false,
                None,
            )
            .unwrap();
        assert!(
            used.duplicate_source
                .contains(&interaction_point_key(original[0]))
        );
        assert!(
            !used
                .duplicate_source
                .contains(&interaction_point_key(changed))
        );
    }

    #[test]
    fn route_flavors_share_obstacles_and_keep_vessel_ports_local() {
        for clustered in [false, true] {
            let mut input = crate::Graph::default();
            let source = input.add_node(routing_test_node("source", 20.0, 20.0));
            let member = input.add_node(routing_test_node("member", 20.0, 20.0));
            let target = input.add_node(routing_test_node("target", 20.0, 20.0));
            input.add_edge(crate::Edge { source, target });
            let mut graph = ArenaGraph::from_input(&input);
            for (node, x, y) in [
                (source, 0.0, 0.0),
                (member, 0.0, 40.0),
                (target, 100.0, 0.0),
            ] {
                graph.set_position(node, Point { x, y });
            }
            if clustered {
                graph.clusters.push(crate::internal::ClusterState {
                    members: vec![source, member],
                    arrangement: crate::internal::ClusterArrangement::Column,
                    desired_arrangement: crate::internal::ClusterArrangement::Column,
                    padding: 0.0,
                    vessel_tala_id: 0,
                    fixed_size: false,
                });
                for node in [source, member] {
                    graph.nodes[node.0 as usize].cluster = Some(0);
                }
            }
            let nodes = [source, member, target];
            let context = build_route_ovg_context(&graph, &nodes, &[], &[0], None);
            let original_ports = || {
                context
                    .node_ports
                    .iter()
                    .map(|(&node, ports)| {
                        (
                            node,
                            ports.iter().copied().map(port_key).collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let before = original_ports();
            let prepare = |shared| {
                prepare_route_flavor(
                    &graph,
                    &nodes,
                    &[],
                    &[0],
                    &[0],
                    "Default",
                    None,
                    shared,
                    None,
                    None,
                )
                .unwrap_or_else(|response| panic!("router construction: {:?}", response.4))
            };
            // coordinator.go creates every router before generating any route.
            let first = prepare(Some(&context));
            let second = prepare(Some(&context));
            assert!(std::sync::Arc::ptr_eq(&first.boxes, &second.boxes));
            assert!(std::sync::Arc::ptr_eq(
                &first.obstacle_node_order,
                &second.obstacle_node_order
            ));
            assert!(std::sync::Arc::ptr_eq(
                &first.node_ports,
                &second.node_ports
            ));
            let independent =
                generate_route_flavor(&graph, &nodes, &[0], "Default", None, None, prepare(None));
            assert!(independent.4.is_none(), "{:?}", independent.4);
            assert!(!independent.0[0].is_empty());
            for prepared in [first, second] {
                let response =
                    generate_route_flavor(&graph, &nodes, &[0], "Default", None, None, prepared);
                assert!(response.4.is_none(), "{:?}", response.4);
                assert_eq!(response.0, independent.0);
                assert_eq!(response.1, independent.1);
                assert_eq!(response.2.to_bits(), independent.2.to_bits());
                assert_eq!(original_ports(), before);
            }
        }
    }

    #[test]
    fn fixed_route_records_follow_edge_occurrences_and_preserve_node_capacity() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 100.0, y: 0.0 });
        for edge in &mut graph.edge_records {
            edge.route.replace(RoutePoints::literal(&[
                Point { x: 301.0, y: 401.0 },
                Point { x: 501.0, y: 401.0 },
                Point { x: 601.0, y: 501.0 },
            ]));
        }
        graph
            .edge_order
            .replace_values(&[crate::EdgeId(1), crate::EdgeId(0), crate::EdgeId(1)]);
        let context = build_route_ovg_context(
            &graph,
            &[source, target],
            &[],
            &[],
            Some(&[Vec::new(), Vec::new()]),
        );
        assert_eq!(
            context
                .existing_routes
                .iter()
                .map(|r| r.edge.0)
                .collect::<Vec<_>>(),
            vec![1, 0, 1]
        );
        assert!(
            context
                .existing_routes
                .iter()
                .all(|r| r.nodes.len() == 5 && r.nodes.capacity() == 8)
        );
        assert_eq!(
            context.existing_routes[0].nodes,
            context.existing_routes[2].nodes
        );
        let mut interactions = RouteInteractionIndex::new(&context.visibility, &graph);
        for route in context.existing_routes.iter() {
            interactions
                .add_imported_route(&graph, &context.visibility, route, "test", None)
                .unwrap();
        }
        assert_eq!(interactions.accepted_edge_indices(), &[1, 0, 1]);
        let first = context
            .visibility
            .point(context.existing_routes[0].nodes[0]);
        assert_eq!(
            interactions.point_to_routes[&interaction_point_key(first)],
            vec![0, 1, 2]
        );
        assert_eq!(
            interactions
                .routes
                .iter()
                .map(|r| r.edge)
                .collect::<Vec<_>>(),
            vec![1, 0, 1]
        );
        let selected = build_route_ovg_context(
            &graph,
            &[source, target],
            &[],
            &[1],
            Some(&[Vec::new(), Vec::new()]),
        );
        assert_eq!(
            selected
                .existing_routes
                .iter()
                .map(|r| r.edge.0)
                .collect::<Vec<_>>(),
            vec![0]
        );
    }

    #[test]
    fn coincident_single_node_import_does_not_create_a_router_route() {
        let mut input = crate::Graph::default();
        let node = input.add_node(routing_test_node("loop", 0.0, 0.0));
        input.add_edge(crate::Edge {
            source: node,
            target: node,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(node, Point { x: 0.0, y: 0.0 });
        graph.edge_records[0]
            .route
            .replace(RoutePoints::literal(&[Point { x: 0.0, y: 0.0 }]));
        let context = build_route_ovg_context(
            &graph,
            &[node],
            &[],
            &[],
            Some(&[vec![Point { x: 9.0, y: 9.0 }]]),
        );
        assert!(context.existing_routes.is_empty());
    }

    #[test]
    fn route_publication_checks_ovg_identity_not_coordinate_equality() {
        let (graph, mut visibility, source, target) = route_interaction_fixture();
        let first = Point { x: 300.0, y: 400.0 };
        let second = Point { x: 500.0, y: 400.0 };

        // Import uses AddNode's coordinate interning, so the repeated point
        // carries the same OVG identity even though the route stores only
        // coordinates.
        let repeated_route = [first, second, first];
        let repeated_ids = visibility.add_existing_route(source, target, &repeated_route);
        assert_eq!(repeated_ids.len(), 5);
        assert_eq!(repeated_ids[1], repeated_ids[3]);
        let mut interactions = RouteInteractionIndex::new(&visibility, &graph);
        assert_eq!(
            interactions.add_route_with_node_ids(
                &graph,
                0,
                &repeated_route,
                Some(&repeated_ids),
                "test",
                None,
            ),
            Err(RoutePublicationError::DuplicateOvgNode {
                index: 3,
                point: first
            })
        );
        assert!(interactions.routes.is_empty());

        // Pointer-distinct OVG nodes may occupy the same coordinate, so the
        // corresponding IDs must remain legal even with repeated points.
        let coincident_route = [first, first, first];
        let distinct_ids = [20, 21, 22, 23, 24];
        assert_eq!(
            interactions.add_route_with_node_ids(
                &graph,
                0,
                &coincident_route,
                Some(&distinct_ids),
                "test",
                None,
            ),
            Ok(())
        );
    }

    #[test]
    fn overlap_work_uses_published_identity_count_including_coincident_centers() {
        let (graph, visibility, _, _) = route_interaction_fixture();
        let mut index = RouteInteractionIndex::new(&visibility, &graph);
        let (source, target) = index.route_centers[0].unwrap();
        let route = [source, target];
        index
            .add_generated_route_with_node_ids(
                &graph,
                0,
                &route,
                &[100, 101, 102, 103],
                None,
                "test",
                None,
            )
            .unwrap();
        assert_eq!(index.routes[0].len(), 4);
        let key = index
            .static_index
            .edge_at(route)
            .unwrap_or_else(|| index.static_index.append_value(route));
        index.static_index.overlapping.insert(key, vec![0]);
        index.point_to_routes.clear();
        index.static_index.nearby.insert(key);
        let guard = LocalRouteSearchWork::new(20);
        assert_eq!(
            index.apply_route_interaction_cost_guarded(&graph, 0, route, 1.0, Some(&guard)),
            Some(10_000_000.0)
        );
        assert_eq!(guard.used(), 20); // (4 published nodes + 1)*2, then ten passes
        let guard = LocalRouteSearchWork::new(19);
        assert_eq!(
            index.apply_route_interaction_cost_guarded(&graph, 0, route, 1.0, Some(&guard)),
            None
        );
        assert!(guard.exhausted());
    }

    #[test]
    fn occupied_route_work_preserves_multiplicity_and_once_per_pop_reservations() {
        let (graph, visibility, _, _) = route_interaction_fixture();
        let mut index = RouteInteractionIndex::new(&visibility, &graph);
        let point = Point { x: 12.0, y: 34.0 };
        index
            .point_to_routes
            .insert(interaction_point_key(point), vec![0, 0, 0]);
        let guard = LocalRouteSearchWork::new(33);
        assert_eq!(index.charge_occupied_routes(Some(point), &guard), Some(()));
        assert_eq!(guard.used(), 33);
        let guard = LocalRouteSearchWork::new(32);
        assert_eq!(index.charge_occupied_routes(Some(point), &guard), None);
        assert!(guard.exhausted());
        let guard = LocalRouteSearchWork::new(0);
        assert_eq!(index.charge_occupied_routes(None, &guard), Some(()));
        assert_eq!(guard.used(), 0);
    }

    #[test]
    fn intersection_scan_charges_skipped_axes_and_stops_at_first_crossing() {
        let (graph, visibility, _, _) = route_interaction_fixture();
        for transpose in [false, true] {
            let point = |x, y| {
                if transpose {
                    Point { x: y, y: x }
                } else {
                    Point { x, y }
                }
            };
            let mut index = RouteInteractionIndex::new(&visibility, &graph);
            for axis in [-10.0, 5.0, 20.0] {
                assert!(
                    index.charge_routed_edge_insert([point(axis, 0.0), point(axis, 10.0)], None)
                );
            }
            let candidate = [point(0.0, 5.0), point(10.0, 5.0)];
            let guard = LocalRouteSearchWork::new(3);
            assert_eq!(
                index.intersects_routed_edges_guarded(candidate, &guard),
                Some(true)
            );
            assert_eq!(guard.used(), 3); // skipped axis, crossing axis, first segment
            let guard = LocalRouteSearchWork::new(2);
            assert_eq!(
                index.intersects_routed_edges_guarded(candidate, &guard),
                None
            );
            assert!(guard.exhausted());
            let miss = [point(0.0, 20.0), point(10.0, 20.0)];
            let guard = LocalRouteSearchWork::new(4);
            assert_eq!(
                index.intersects_routed_edges_guarded(miss, &guard),
                Some(false)
            );
            assert_eq!(guard.used(), 4); // includes the first out-of-range axis
        }
    }

    #[test]
    fn generated_route_publication_rejects_repeated_ovg_identity() {
        let (graph, visibility, source, target) = route_interaction_fixture();
        let source_center = visibility.center_index(source).unwrap();
        let target_center = visibility.center_index(target).unwrap();
        let first = Point { x: 300.0, y: 400.0 };
        let second = Point { x: 500.0, y: 400.0 };
        let route = [first, second, first];
        let mut interactions = RouteInteractionIndex::new(&visibility, &graph);

        assert_eq!(
            interactions.add_generated_route_with_node_ids(
                &graph,
                0,
                &route,
                &[
                    source_center,
                    usize::MAX,
                    usize::MAX - 1,
                    usize::MAX,
                    target_center,
                ],
                None,
                "test",
                None,
            ),
            Err(RoutePublicationError::DuplicateOvgNode {
                index: 3,
                point: first
            })
        );

        // Equal coordinates remain legal when the corresponding OVG nodes
        // are pointer-distinct, matching Go's identity-based duplicate guard.
        assert_eq!(
            interactions.add_generated_route_with_node_ids(
                &graph,
                0,
                &[first, first, first],
                &[
                    source_center,
                    usize::MAX,
                    usize::MAX - 1,
                    usize::MAX - 2,
                    target_center,
                ],
                None,
                "test",
                None,
            ),
            Ok(())
        );
    }

    #[test]
    fn generated_route_publication_keeps_coincident_center_port_occurrences() {
        let (graph, visibility, _, _) = route_interaction_fixture();
        let source_center = Point { x: 10.0, y: 10.0 };
        let target_port = Point { x: 100.0, y: 10.0 };
        let route = [source_center, target_port];
        let mut interactions = RouteInteractionIndex::new(&visibility, &graph);

        // The route's first point represents a center port. Its coordinate
        // matches the source center, but upstream keeps both OVG nodes in
        // Route.OVGNodes and appends the edge twice to pointToRoute.
        assert!(interactions.add_route_with_budget(&graph, 0, &route, "test", None));
        assert_eq!(
            interactions
                .point_to_routes
                .get(&interaction_point_key(source_center)),
            Some(&vec![0, 0])
        );
        assert_eq!(
            interactions.routes[0].segment_endpoints(),
            Some(route.to_vec())
        );
    }

    #[test]
    fn additional_routing_retains_ports_coincident_with_zero_sized_centers() {
        // Go OVG.connectPortsToCenter adds distinct centers after its ports,
        // replacing OccupiedPoints at these coordinates without replacing Ports.
        for separation in [0.125, 10.0, 200.0] {
            let mut input = crate::Graph::default();
            let source = input.add_node(routing_test_node("source", 0.0, 0.0));
            let target = input.add_node(routing_test_node("target", 0.0, 0.0));
            input.add_edge(crate::Edge { source, target });
            let mut graph = ArenaGraph::from_input(&input);
            graph.set_position(source, Point { x: 0.0, y: 0.0 });
            graph.set_position(
                target,
                Point {
                    x: separation,
                    y: 0.0,
                },
            );
            initialize_standalone_routing_costs(&mut graph);
            let routes = route_additional_edges_with_budget(&graph, &[crate::EdgeId(0)], None, 0)
                .expect("distinct retained ports form a publishable route");
            assert_eq!(
                routes[0],
                vec![
                    Point { x: 0.0, y: 0.0 },
                    Point {
                        x: separation,
                        y: 0.0
                    }
                ]
            );
        }
    }

    #[test]
    fn additional_routing_preserves_generation_work_errors_before_empty_routes() {
        // Pinned ovg_edge_router.go generateRoutes returns a failed work
        // operation before testing the route length; its deferred finish
        // preserves that error. No selected route may be published on failure.
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 100.0, y: 0.0 });
        let before = graph.edge_records[0].route.read().to_vec();
        for limit in [8, 16, 32] {
            let error =
                route_additional_edges_with_budget(&graph, &[crate::EdgeId(0)], Some(limit), 0)
                    .unwrap_err();
            assert!(
                matches!(error, crate::LayoutError::RouteSearchWorkLimit {
                    flavor: "ShortestToLongest", limit: actual,
                } if actual == limit),
                "limit {limit}: {error}"
            );
            assert_eq!(&*graph.edge_records[0].route.read(), before.as_slice());
        }

        struct Context(std::cell::Cell<usize>);
        impl crate::internal::limits::Context for Context {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<crate::internal::limits::ContextError> {
                use crate::internal::limits::ContextError;
                let polls = self.0.get() + 1;
                self.0.set(polls);
                match polls {
                    1 => None,
                    2 => Some(ContextError::Cancelled),
                    _ => Some(ContextError::DeadlineExceeded),
                }
            }
        }
        let nodes = graph.graph_node_order();
        let fixed_routes = vec![Vec::new()];
        let prepared = prepare_route_flavor(
            &graph,
            &nodes,
            &[],
            &[0],
            &[0],
            "ShortestToLongest",
            Some(&fixed_routes),
            None,
            None,
            None,
        )
        .unwrap_or_else(|_| panic!("prepare an unlimited router"));
        let context = Context(std::cell::Cell::new(0));
        let work = LocalRouteSearchWork::with_context(100, &context).unwrap();
        let response = generate_route_flavor(
            &graph,
            &nodes,
            &[0],
            "ShortestToLongest",
            Some(&fixed_routes),
            Some(&work),
            prepared,
        );
        assert!(matches!(response.4, Some(crate::LayoutError::Cancelled)));
        assert_eq!(context.0.get(), 2); // response construction must not poll
        assert_eq!(work.used(), 0);
        assert!(work.finish().is_err());
        assert!(matches!(
            work.failure(),
            crate::LayoutError::DeadlineExceeded
        ));
        assert!(matches!(response.4, Some(crate::LayoutError::Cancelled)));
        assert_eq!(&*graph.edge_records[0].route.read(), before.as_slice());
    }

    #[test]
    fn additional_edge_routing_rejects_a_fixed_route_that_revisits_an_ovg_node() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let middle = input.add_node(routing_test_node("middle", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        input.add_edge(crate::Edge {
            source,
            target: middle,
        });
        input.add_edge(crate::Edge {
            source: middle,
            target,
        });
        let mut graph = ArenaGraph::from_input(&input);
        for (node, x) in [(source, 0.0), (middle, 100.0), (target, 200.0)] {
            graph.set_position(node, Point { x, y: 0.0 });
        }
        graph.edge_records[0].route.replace(RoutePoints::literal(&[
            Point { x: 300.0, y: 400.0 },
            Point { x: 500.0, y: 400.0 },
            Point { x: 300.0, y: 400.0 },
        ]));

        let error = route_additional_edges_with_budget(&graph, &[crate::EdgeId(1)], None, 1)
            .expect_err("fixed route repeats an imported OVG node");
        assert!(
            matches!(&error, crate::LayoutError::DuplicateRouteOvgNode { point, index: 3, .. } if *point == Point { x: 300.0, y: 400.0 }),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn router_construction_work_matches_v090_inventory_boundaries() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.clusters.push(crate::internal::ClusterState {
            members: vec![source, target],
            arrangement: crate::internal::ClusterArrangement::Row,
            desired_arrangement: crate::internal::ClusterArrangement::Row,
            padding: 0.0,
            vessel_tala_id: 0,
            fixed_size: false,
        });

        // add(edge count) + cluster/member steps (3) + edge step + both
        // one-edge endpoint adjacency lists + two len=1 sort steps = 9.
        let exact = LocalRouteSearchWork::new(9);
        charge_route_router_construction(&graph, &[0], &exact).unwrap();
        assert_eq!(exact.used(), 9);

        let one_short = LocalRouteSearchWork::new(8);
        assert!(charge_route_router_construction(&graph, &[0], &one_short).is_err());
        assert_eq!(one_short.used(), 8);
        assert!(one_short.exhausted());
    }

    #[test]
    fn constructor_finish_preserves_the_error_before_later_cancellation() {
        struct Context(std::cell::Cell<usize>);
        impl crate::internal::limits::Context for Context {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<crate::internal::limits::ContextError> {
                let polls = self.0.get() + 1;
                self.0.set(polls);
                (polls >= 2).then_some(crate::internal::limits::ContextError::Cancelled)
            }
        }
        let context = Context(std::cell::Cell::new(0));
        let work = LocalRouteSearchWork::with_context(0, &context).unwrap();
        let error = finish_failed_route_constructor(
            &work,
            Some(crate::LayoutError::BadRouteState(
                "constructor failed".into(),
            )),
        );
        assert!(
            matches!(error, crate::LayoutError::BadRouteState(message) if message == "constructor failed")
        );
        assert_eq!(context.0.get(), 2);
        assert!(matches!(work.failure(), crate::LayoutError::Cancelled));
        // A cause observed by the failing operation itself is preserved too.
        let error = finish_failed_route_constructor(
            &work,
            Some(crate::LayoutError::BadRouteState(
                "generic guard failure".into(),
            )),
        );
        assert!(matches!(error, crate::LayoutError::Cancelled));
    }

    #[test]
    fn ordinary_constructor_failure_rejects_without_publishing() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 100.0, y: 0.0 });
        let before = graph.edge_records[0].route.read().to_vec();
        let aggregate = super::super::route_search_guard::RouteStageWork::new(20);
        let error = route_edge_subset(
            &mut graph,
            &[source, target],
            &[],
            &[0],
            Some(0),
            None,
            Some(&aggregate),
        )
        .unwrap_err();
        assert!(matches!(error, crate::LayoutError::BadRouteState(message)
            if message == "route-search work limit exceeded while preparing router"));
        assert_eq!(aggregate.used(), 0);
        assert_eq!(&*graph.edge_records[0].route.read(), before.as_slice());
        for deadline in [false, true] {
            let token = d2_rs_core::CancellationToken::new();
            if deadline {
                token.expire();
            } else {
                token.cancel();
            }
            let result = crate::internal::with_cancellation(Some(&token), || {
                route_edge_subset(&mut graph, &[source, target], &[], &[0], None, None, None)
            });
            if deadline {
                assert!(matches!(result, Err(crate::LayoutError::DeadlineExceeded)));
            } else {
                assert!(matches!(result, Err(crate::LayoutError::Cancelled)));
            }
            assert_eq!(&*graph.edge_records[0].route.read(), before.as_slice());
        }
    }

    #[test]
    fn routing_precision_compare_uses_the_recovered_open_boundary() {
        assert_eq!(
            routing_precision_compare(0.0001_f64.next_down(), 0.0),
            Ordering::Equal
        );
        assert_eq!(routing_precision_compare(0.0001, 0.0), Ordering::Greater);
        assert_eq!(routing_precision_compare(-0.0001, 0.0), Ordering::Less);
    }

    #[test]
    fn table_candidate_filter_accepts_only_the_exact_base_port() {
        let base = Port {
            point: Point { x: 20.0, y: 54.0 },
            direction: PortSide::Left,
            side: PortSide::Left,
            index: 0,
            tunnel: None,
            is_center: false,
        };
        let allowed = Some(vec![base]);
        assert!(table_port_allowed(&allowed, base));
        assert!(!table_port_allowed(
            &allowed,
            Port {
                tunnel: Some(0),
                ..base
            }
        ));
        assert!(!table_port_allowed(
            &allowed,
            Port {
                direction: PortSide::Right,
                ..base
            }
        ));
        assert!(table_port_allowed(&None, base));
    }

    #[test]
    fn source_center_contact_precedes_opposing_route_segment() {
        let route = [
            Point { x: 0.0, y: 80.0 },
            Point { x: 0.0, y: 130.0 },
            Point { x: -40.0, y: 130.0 },
        ];
        assert!(!route_is_opposing_collinear(
            &route,
            Some((Point { x: 0.0, y: 0.0 }, Point { x: -40.0, y: 50.0 })),
            route[1],
            route[0],
        ));
        // Strict interior overlap still opposes the route direction.
        assert!(route_is_opposing_collinear(
            &route,
            Some((Point { x: 0.0, y: 0.0 }, Point { x: -40.0, y: 50.0 })),
            Point { x: 0.0, y: 120.0 },
            Point { x: 0.0, y: 90.0 },
        ));
    }

    #[test]
    fn used_tunnel_alias_waives_the_mirrored_center_surcharge() {
        let rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: crate::Size {
                width: 100.0,
                height: 100.0,
            },
        };
        let mut owner_ports = super::super::ports::ports(rect, ShapeKind::Rectangle);
        let top_center = owner_ports[1];
        let bottom_center = owner_ports[7];
        owner_ports.push(Port {
            tunnel: Some(9),
            ..top_center
        });

        let used = BTreeSet::from([port_key(Port {
            tunnel: Some(9),
            ..top_center
        })]);
        assert!(mirrored_center_port_coordinate_is_used(
            ShapeKind::Rectangle,
            bottom_center,
            &owner_ports,
            &used,
        ));
    }

    #[test]
    fn static_route_crossings_use_the_add_route_index_with_external_fallback() {
        let static_candidate = [Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 }];
        let routed_segment = [Point { x: 5.0, y: -5.0 }, Point { x: 5.0, y: 5.0 }];
        let mut index = RouteInteractionIndex {
            #[cfg(feature = "diagnostic-traces")]
            trace_flavor: None,
            static_index: super::super::static_edge_index::StaticEdgeIndex::from_segments([
                static_candidate,
            ]),
            route_centers: Vec::new(),
            tunnel_points: Default::default(),
            point_to_routes: Default::default(),

            routes: Vec::new(),
            shared_points: Vec::new().into(),
            canonical_nodes: Vec::new().into(),
            accepted_edge_order: Vec::new(),

            routed_segments: vec![routed_segment],
            routed_set: super::routed_edge_set::EdgeSet::default(),
            consider_node_labels: false,
            node_label_rects: Vec::new(),
            positioned_arrowhead_labels: Vec::new(),
            search_clusters: None,
        };

        index
            .routed_set
            .add(super::super::routed_edge_set::Edge::fixture(routed_segment));
        assert!(index.crosses_routed_segment(static_candidate));
        assert!(
            index.crosses_routed_segment([Point { x: 0.0, y: 1.0 }, Point { x: 10.0, y: 1.0 },])
        );
        assert!(!index.crosses_routed_segment([routed_segment[0], Point { x: 10.0, y: -5.0 },]));
    }

    #[test]
    fn add_route_preserves_go_point_and_overlap_multiplicity() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        let graph = ArenaGraph::from_input(&input);

        let first = Point { x: 0.0, y: 0.0 };
        let second = Point { x: 10.0, y: 0.0 };
        // Each distinct static edge object retains its own repeated Route list.
        let static_index =
            super::super::static_edge_index::StaticEdgeIndex::from_segments([[first, second]; 2]);
        let mut index = RouteInteractionIndex {
            #[cfg(feature = "diagnostic-traces")]
            trace_flavor: None,
            static_index,
            route_centers: vec![Some((
                Point { x: -10.0, y: -10.0 },
                Point { x: 20.0, y: 10.0 },
            ))],
            tunnel_points: Default::default(),
            point_to_routes: Default::default(),

            routes: Vec::new(),
            shared_points: Vec::new().into(),
            canonical_nodes: Vec::new().into(),
            accepted_edge_order: Vec::new(),

            routed_segments: Vec::new(),
            routed_set: super::routed_edge_set::EdgeSet::default(),
            consider_node_labels: false,
            node_label_rects: Vec::new(),
            positioned_arrowhead_labels: Vec::new(),
            search_clusters: None,
        };

        // The reverse segment is the same OVG edge geometrically, but Go's
        // addRoute still visits both route-node intervals. Its point map and
        // overlappingRoutes map therefore retain repeated route pointers.
        let route = vec![first, second, first];
        assert!(index.add_route_with_budget(&graph, 0, &route, "test", None));
        assert_eq!(
            index.point_to_routes.get(&interaction_point_key(first)),
            Some(&vec![0, 0])
        );
        assert_eq!(
            index.point_to_routes.get(&interaction_point_key(second)),
            Some(&vec![0])
        );
        assert_eq!(index.static_index.overlapping.get(&0), Some(&vec![0, 0]));
        assert_eq!(index.static_index.overlapping.get(&1), Some(&vec![0, 0]));
        assert_eq!(
            index.overlapping_route_indices(first, second, None),
            Some(vec![0])
        );
        assert!(index.charge_routed_edge_insert([first, first], None));
        assert!(index.routed_set.vertical_axes.contains(&first.x));
    }

    #[test]
    fn guarded_search_ancestry_preserves_nil_and_identity_work_boundaries() {
        let (mut graph, _, source, target) = route_interaction_fixture();
        graph.nodes[source.0 as usize].container = Some(target);
        let cases = [
            (Some(source), Some(source), true, 1),
            (Some(source), Some(target), true, 2),
            (Some(target), Some(source), false, 1),
            (Some(source), None, true, 2),
            (None, None, true, 1),
            (None, Some(target), false, 1),
        ];
        for (descendant, ancestor, result, units) in cases {
            let work = LocalRouteSearchWork::new(units);
            assert_eq!(
                super::super::route_search_guard::is_descendant_of(
                    &graph,
                    descendant,
                    ancestor,
                    Some(&work)
                ),
                Some(result)
            );
            assert_eq!(work.used(), units);
        }
        let limited = LocalRouteSearchWork::new(1);
        assert_eq!(
            super::super::route_search_guard::is_descendant_of(
                &graph,
                Some(source),
                Some(target),
                Some(&limited)
            ),
            None
        );
        assert!(limited.exhausted());
        assert_eq!(limited.used(), 1);
    }

    #[test]
    fn search_label_scan_charges_endpoints_and_commits_only_when_complete() {
        let (graph, visibility, _, _) = route_interaction_fixture();
        let mut index = RouteInteractionIndex::new_with_node_labels(&visibility, &graph, true);
        let before = index.node_label_rects.clone();
        let limited = LocalRouteSearchWork::new(graph.nodes.len() as u64 - 1);
        assert!(
            index
                .prepare_search_node_labels(&graph, 0, Some(&limited))
                .is_none()
        );
        assert_eq!(index.node_label_rects, before);
        let exact = LocalRouteSearchWork::new(graph.nodes.len() as u64);
        assert!(
            index
                .prepare_search_node_labels(&graph, 0, Some(&exact))
                .is_some()
        );
        assert_eq!(exact.used(), graph.nodes.len() as u64);
        assert!(index.node_label_rects.is_empty());
        let mut disabled = RouteInteractionIndex::new(&visibility, &graph);
        let zero = LocalRouteSearchWork::new(0);
        assert!(
            disabled
                .prepare_search_node_labels(&graph, 0, Some(&zero))
                .is_some()
        );
        assert_eq!(zero.used(), 0);
    }

    #[test]
    fn component_search_scans_only_logical_members_with_stable_storage_ids() {
        let mut input = crate::Graph::default();
        let unrelated = input.add_node(routing_test_node("unrelated", 20.0, 20.0));
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        for (node, x) in [(unrelated, 500.0), (source, 0.0), (target, 100.0)] {
            graph.set_position(node, Point { x, y: 0.0 });
        }
        let boxes = graph
            .nodes
            .iter()
            .map(|node| (node.input_id, node_rect(&graph, node.input_id).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let visibility = super::super::visibility::VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.node_order,
            &graph.node_order,
            std::iter::empty(),
        );
        graph.node_order = vec![target, source];
        let mut index = RouteInteractionIndex::new_with_node_labels(&visibility, &graph, true);
        let work = LocalRouteSearchWork::new(2);
        assert!(
            index
                .prepare_search_node_labels(&graph, 0, Some(&work))
                .is_some()
        );
        assert_eq!(work.used(), 2);
        assert!(index.node_label_rects.is_empty());
        let ancestors = LocalRouteSearchWork::new(2);
        assert_eq!(
            super::super::route_search_guard::filter_edge_ancestors(&graph, 0, Some(&ancestors)),
            Some(vec![target, source])
        );
        assert_eq!(ancestors.used(), 2);
    }

    #[test]
    fn publication_finishes_edge_index_before_nearby_scan_exhaustion() {
        let (graph, visibility, _, _) = route_interaction_fixture();
        let mut base = RouteInteractionIndex::new(&visibility, &graph);
        let mut crowded = RouteInteractionIndex::new(&visibility, &graph);
        for index in [&mut base, &mut crowded] {
            index.static_index.horizontal.clear();
            index.static_index.vertical.clear();
        }
        let route = [
            Point { x: 10.0, y: 0.0 },
            Point { x: 40.0, y: 0.0 },
            Point { x: 40.0, y: 30.0 },
            Point { x: 90.0, y: 30.0 },
        ];
        let base_work = LocalRouteSearchWork::new(100_000);
        assert!(base.add_route_with_budget(&graph, 0, &route, "test", Some(&base_work)));
        // Every axis is charged before its proximity filter. Exhaust that
        // scan after index publication, even though none is nearby.
        for axis in 100..200 {
            crowded.static_index.horizontal.buckets.insert(
                (axis as f64).to_bits(),
                super::super::static_edge_index::Bucket {
                    axis: axis as f64,
                    edges: Vec::new(),
                },
            );
        }
        let limited_work = LocalRouteSearchWork::new(base_work.used());
        assert!(!crowded.add_route_with_budget(&graph, 0, &route, "test", Some(&limited_work)));
        assert_eq!(crowded.routed_segments, base.routed_segments);
        assert_eq!(crowded.point_to_routes, base.point_to_routes);
    }

    #[test]
    fn invisible_publication_does_not_charge_nearby_axis_inventory() {
        let (mut graph, visibility, _, _) = route_interaction_fixture();
        graph.edge_records[0].invisible = true;
        let mut base = RouteInteractionIndex::new(&visibility, &graph);
        let mut extra = RouteInteractionIndex::new(&visibility, &graph);
        for index in [&mut base, &mut extra] {
            index.static_index.horizontal.clear();
            index.static_index.vertical.clear();
        }
        let segment = [Point { x: 10.0, y: 1.0 }, Point { x: 90.0, y: 1.0 }];
        extra.static_index.append_value(segment);
        let route = [Point { x: 10.0, y: 0.0 }, Point { x: 90.0, y: 0.0 }];
        let base_work = LocalRouteSearchWork::new(100_000);
        let extra_work = LocalRouteSearchWork::new(100_000);
        assert!(base.add_route_with_budget(&graph, 0, &route, "test", Some(&base_work)));
        assert!(extra.add_route_with_budget(&graph, 0, &route, "test", Some(&extra_work)));
        assert_eq!(base_work.used(), extra_work.used());
        assert!(extra.static_index.nearby.is_empty());
        assert!(extra.static_index.overlapping.is_empty());
        assert!(!extra.routed_segments.is_empty());
    }

    #[test]
    fn symmetry_preparation_scans_aliases_and_reserves_before_marks() {
        let (graph, visibility, source, target) = route_interaction_fixture();
        let source_shape =
            super::super::ports(node_rect(&graph, source).unwrap(), ShapeKind::Rectangle);
        let target_shape =
            super::super::ports(node_rect(&graph, target).unwrap(), ShapeKind::Rectangle);
        let mut source_raw = source_shape.clone();
        let mut alias = source_shape[1];
        alias.index = usize::MAX;
        alias.tunnel = Some(7);
        source_raw.push(alias);
        let used = BTreeSet::from([port_key(source_shape[7])]);
        let empty = BTreeSet::new();
        let endpoints = [
            SymmetryEndpoint {
                node: source,
                shape: ShapeKind::Rectangle,
                raw_ports: &source_raw,
                shape_ports: &source_shape,
                used: &used,
            },
            SymmetryEndpoint {
                node: target,
                shape: ShapeKind::Rectangle,
                raw_ports: &target_shape,
                shape_ports: &target_shape,
                used: &empty,
            },
        ];
        let expected = 2 * (source_raw.len() + target_shape.len()) as u64;
        let work = LocalRouteSearchWork::new(expected);
        let marked = center_symmetrical_ports(&visibility, &endpoints, Some(&work)).unwrap();
        assert_eq!(
            marked,
            BTreeSet::from([visibility
                .port_index(source, source_shape[1].point)
                .unwrap()])
        );
        assert_eq!(work.used(), expected);
        for limit in 0..expected {
            let work = LocalRouteSearchWork::new(limit);
            assert!(center_symmetrical_ports(&visibility, &endpoints, Some(&work)).is_none());
            let reservation = (source_raw.len() + target_shape.len()) as u64;
            assert_eq!(work.used(), if limit < reservation { 0 } else { limit });
        }
    }

    #[test]
    fn used_ports_keeps_repeated_routes_and_rejects_coordinate_only_identity() {
        let (graph, visibility, source, target) = route_interaction_fixture();
        let ports = [source, target]
            .into_iter()
            .map(|node| {
                (
                    node,
                    super::super::ports(
                        node_rect(&graph, node).unwrap(),
                        graph.nodes[node.0 as usize].shape,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let from = ports[&source][0].point;
        let to = ports[&target][0].point;
        let ids = [
            visibility.center_index(source).unwrap(),
            usize::MAX,
            usize::MAX - 1,
            visibility.center_index(target).unwrap(),
        ];
        let mut index = RouteInteractionIndex::new(&visibility, &graph);
        index
            .add_generated_route_with_node_ids(&graph, 0, &[from, to], &ids, None, "test", None)
            .unwrap();
        let source_center = index.route_centers[0].unwrap().0;
        index
            .point_to_routes
            .get_mut(&interaction_point_key(source_center))
            .unwrap()
            .push(0);
        let expected = 2 + 3 + 3 * 4 * (1 + ports[&source].len() + ports[&target].len());
        let work = LocalRouteSearchWork::new(expected as u64);
        let state = index
            .used_ports(
                &graph,
                0,
                &visibility,
                &ports,
                &BTreeSet::new(),
                &BTreeSet::new(),
                false,
                Some(&work),
            )
            .unwrap();
        assert!(state.source.is_empty() && state.target.is_empty());
        assert!(
            state
                .duplicate_source
                .contains(&interaction_point_key(from))
        );
        assert!(state.duplicate_target.contains(&interaction_point_key(to)));
        assert_eq!(work.used(), expected as u64);
        let canonical_ids = [
            ids[0],
            visibility.port_index(source, from).unwrap(),
            visibility.port_index(target, to).unwrap(),
            ids[3],
        ];
        index.routes[0].identities = Some(canonical_ids.to_vec());
        let canonical_work = LocalRouteSearchWork::new(expected as u64);
        let canonical = index
            .used_ports(
                &graph,
                0,
                &visibility,
                &ports,
                &BTreeSet::new(),
                &BTreeSet::new(),
                false,
                Some(&canonical_work),
            )
            .unwrap();
        assert!(canonical.source.contains(&port_key(ports[&source][0])));
        assert!(canonical.target.contains(&port_key(ports[&target][0])));
        assert_eq!(
            canonical_work.used(),
            (expected - 3 * (ports[&source].len() - 1 + ports[&target].len() - 1)) as u64
        );
        index.routes[0].identities = Some(ids.to_vec());
        for limit in [0, 1, 2, expected as u64 - 1] {
            let work = LocalRouteSearchWork::new(limit);
            assert!(
                index
                    .used_ports(
                        &graph,
                        0,
                        &visibility,
                        &ports,
                        &BTreeSet::new(),
                        &BTreeSet::new(),
                        false,
                        Some(&work)
                    )
                    .is_none()
            );
            assert_eq!(work.used(), limit);
        }
    }

    #[test]
    fn imported_center_endpoint_labels_use_retained_ovg_occurrences() {
        use crate::Size;
        for is_target in [false, true] {
            let (mut graph, mut visibility, source, target) = route_interaction_fixture();
            let label = crate::ArrowheadLabel {
                text: "endpoint".into(),
                size: Size {
                    width: 20.0,
                    height: 10.0,
                },
                color: None,
            };
            if is_target {
                graph.edge_records[0].target_arrowhead_label = Some(label);
            } else {
                graph.edge_records[0].source_arrowhead_label = Some(label);
            }
            graph.edge_records[0].source_arrow = false;
            graph.edge_records[0].target_arrow = false;
            let a = Point { x: 20.0, y: 10.0 };
            let b = Point { x: 100.0, y: 10.0 };
            let route = if is_target {
                vec![a, b, Point { x: 110.0, y: 10.0 }]
            } else {
                vec![Point { x: 10.0, y: 10.0 }, a, b]
            };
            let identities = visibility.add_existing_route(source, target, &route);
            assert_eq!(identities.len(), 4);
            assert_eq!(identities.iter().copied().collect::<BTreeSet<_>>().len(), 4);
            let mut interactions = RouteInteractionIndex::new(&visibility, &graph);
            let work = LocalRouteSearchWork::new(100_000);
            interactions
                .add_route_with_node_ids(&graph, 0, &route, Some(&identities), "test", Some(&work))
                .unwrap();
            let label = &interactions.positioned_arrowhead_labels[0];
            assert_eq!(label.is_target, is_target);
            assert_eq!(
                label.rect,
                Rect {
                    origin: Point {
                        x: if is_target { 75.0 } else { 25.0 },
                        y: -6.0
                    },
                    size: Size {
                        width: 20.0,
                        height: 10.0
                    }
                }
            );
            assert_eq!(
                interactions.materialize_quick_route_records(None).unwrap(),
                vec![(0, vec![a, b])]
            );
            let exact = LocalRouteSearchWork::new(work.used());
            let mut repeated = RouteInteractionIndex::new(&visibility, &graph);
            repeated
                .add_route_with_node_ids(&graph, 0, &route, Some(&identities), "test", Some(&exact))
                .unwrap();
            for limit in 0..work.used() {
                let limited = LocalRouteSearchWork::new(limit);
                let mut rejected = RouteInteractionIndex::new(&visibility, &graph);
                assert!(
                    rejected
                        .add_route_with_node_ids(
                            &graph,
                            0,
                            &route,
                            Some(&identities),
                            "test",
                            Some(&limited)
                        )
                        .is_err()
                );
                assert!(limited.exhausted());
            }
        }
    }

    #[test]
    fn publication_retains_synthetic_node_identity_at_existing_coordinates() {
        let (graph, visibility, _, _) = route_interaction_fixture();
        let mut index = RouteInteractionIndex::new(&visibility, &graph);
        let source = index.route_centers[0].unwrap().0;
        let target = index.route_centers[0].unwrap().1;
        let ids = [
            visibility.center_index(graph.edge_records[0].from).unwrap(),
            usize::MAX,
            usize::MAX - 1,
            visibility.center_index(graph.edge_records[0].to).unwrap(),
        ];
        index
            .add_generated_route_with_node_ids(
                &graph,
                0,
                &[source, target],
                &ids,
                None,
                "test",
                None,
            )
            .unwrap();
        let points = index.routes[0].points();
        let identities = &index.routes[0].identities;
        assert_eq!(&*points, &[source, source, target, target]);
        assert_eq!(identities.as_deref(), Some(ids.as_slice()));
        assert_ne!(ids[0], ids[1]);
        assert_eq!(
            index.materialize_quick_route_records(None).unwrap(),
            vec![(0, vec![source, target])]
        );
    }

    #[test]
    fn publication_does_not_charge_perpendicular_edges() {
        let (graph, visibility, _, _) = route_interaction_fixture();
        let mut base = RouteInteractionIndex::new(&visibility, &graph);
        let mut crossing = RouteInteractionIndex::new(&visibility, &graph);
        for index in [&mut base, &mut crossing] {
            index.static_index.horizontal.clear();
            index.static_index.vertical.clear();
        }
        let segment = [Point { x: 50.0, y: -10.0 }, Point { x: 50.0, y: 10.0 }];
        crossing.static_index.append_value(segment);
        let route = [Point { x: 10.0, y: 0.0 }, Point { x: 90.0, y: 0.0 }];
        let base_work = LocalRouteSearchWork::new(100_000);
        let crossing_work = LocalRouteSearchWork::new(100_000);
        assert!(base.add_route_with_budget(&graph, 0, &route, "test", Some(&base_work)));
        assert!(crossing.add_route_with_budget(&graph, 0, &route, "test", Some(&crossing_work)));
        assert_eq!(base_work.used(), crossing_work.used());
        assert!(crossing.crosses_routed_segment(segment));
    }

    #[test]
    fn routed_edge_insert_polls_after_mutating_the_source_edge_set() {
        let (graph, visibility, _, _) = route_interaction_fixture();
        let mut index = RouteInteractionIndex::new(&visibility, &graph);
        let segment = [Point { x: 10.0, y: 20.0 }, Point { x: 30.0, y: 20.0 }];
        let budget = LocalRouteSearchWork::new(100);
        let token = crate::CancellationToken::new();
        token.cancel();

        let inserted = crate::internal::with_cancellation(Some(&token), || {
            index.charge_routed_edge_insert(segment, Some(&budget))
        });

        // addGuarded reserves and mutates its maps before its unconditional
        // guard.check, so cancellation is reported with the three accepted
        // work units and the inserted edge still visible in the local router.
        assert!(!inserted);
        assert_eq!(budget.used(), 3);
        assert!(
            index
                .routed_set
                .contains(&super::super::routed_edge_set::Edge::fixture(segment))
        );
        assert_eq!(
            index
                .routed_set
                .horizontal
                .get(&super::super::go_float_map_key(segment[0].y))
                .map(|(_, edges)| edges.len()),
            Some(1)
        );
    }

    #[test]
    fn add_route_checks_cancellation_after_publishing_without_inner_edges() {
        let (graph, visibility, _, _) = route_interaction_fixture();
        let mut index = RouteInteractionIndex::new(&visibility, &graph);
        let route = [Point { x: 50.0, y: 40.0 }];
        let budget = LocalRouteSearchWork::new(100);
        let token = crate::CancellationToken::new();
        token.cancel();

        let published = crate::internal::with_cancellation(Some(&token), || {
            index.add_route_with_budget(&graph, 0, &route, "test", Some(&budget))
        });

        // The route has three center-inclusive OVG nodes, no arrowhead labels,
        // and no interior OVG edge. Go's terminal work.check still detects
        // cancellation after publishing the route and its point index.
        assert!(!published);
        assert_eq!(budget.used(), 6);
        assert_eq!(index.routes.len(), 1);
        assert_eq!(index.accepted_edge_order, vec![0]);
    }

    #[test]
    fn add_route_reserves_each_present_arrowhead_label_separately() {
        let (mut graph, visibility, _, _) = route_interaction_fixture();
        let label = crate::ArrowheadLabel {
            text: "endpoint".into(),
            size: crate::Size {
                width: 20.0,
                height: 10.0,
            },
            color: None,
        };
        graph.edge_records[0].source_arrowhead_label = Some(label.clone());
        graph.edge_records[0].target_arrowhead_label = Some(label);
        let route = [Point { x: 50.0, y: 40.0 }];
        // Three OVG-node checks precede separate three-unit source/target
        // reservations. createSegmentEndpoints retains the middle point twice,
        // so each present label is a NaN box. A later guard rejection retains
        // earlier label/route mutations in this local attempt.
        for limit in (0..=12).chain([100]) {
            let mut index = RouteInteractionIndex::new(&visibility, &graph);
            let budget = LocalRouteSearchWork::new(limit);
            let published = index.add_route_with_budget(&graph, 0, &route, "test", Some(&budget));
            let expected_work = match limit {
                0..=2 => limit,
                3..=5 => 3,
                6..=8 => 6,
                _ => limit.min(12),
            };
            let label_count = match limit {
                0..=5 => 0,
                6..=8 => 1,
                _ => 2,
            };
            let point_count = limit.saturating_sub(9).min(3) as usize;
            assert_eq!(published, limit >= 12, "limit {limit}");
            assert_eq!(budget.exhausted(), limit < 12, "limit {limit}");
            assert_eq!(budget.used(), expected_work, "limit {limit}");
            assert_eq!(index.positioned_arrowhead_labels.len(), label_count);
            for (endpoint, positioned) in index.positioned_arrowhead_labels.iter().enumerate() {
                assert_eq!(positioned.edge_index, 0);
                assert_eq!(positioned.is_target, endpoint == 1);
                assert_eq!(positioned.text, "endpoint");
                assert_eq!(
                    positioned.rect.size,
                    graph.edge_records[0]
                        .source_arrowhead_label
                        .as_ref()
                        .unwrap()
                        .size
                );
                assert!(positioned.rect.origin.x.is_nan());
                assert!(positioned.rect.origin.y.is_nan());
                #[cfg(target_arch = "aarch64")]
                {
                    assert_eq!(positioned.rect.origin.x.to_bits(), 0x7ff8000000000000);
                    assert_eq!(positioned.rect.origin.y.to_bits(), 0x7ff8000000000000);
                }
            }
            if limit < 9 {
                assert!(index.routes.is_empty());
                assert!(index.accepted_edge_order.is_empty());
            } else {
                assert_eq!(index.routes[0].point(1), route[0]);
                assert_eq!(index.accepted_edge_order, vec![0]);
                assert_eq!(index.routes[0].len(), 3);
            }
            assert_eq!(index.point_to_routes.len(), point_count);
            let route_nodes = [
                Point { x: 10.0, y: 10.0 },
                route[0],
                Point { x: 110.0, y: 10.0 },
            ];
            for point in route_nodes.into_iter().take(point_count) {
                assert_eq!(
                    index
                        .point_to_routes
                        .get(&interaction_point_key(point))
                        .map(Vec::as_slice),
                    Some([0].as_slice())
                );
            }
        }
    }

    #[test]
    fn slingshot_outer_charge_reserves_adjacency_only_for_eligible_edges() {
        let (mut diagonal_graph, _, source, target) = route_interaction_fixture();
        let diagonal_origin = Point { x: 100.0, y: 100.0 };
        diagonal_graph.nodes[target.0 as usize].position = Some(diagonal_origin);
        diagonal_graph.nodes[target.0 as usize].rect.origin = diagonal_origin;
        let expected = 1
            + diagonal_graph.nodes[source.0 as usize].edges.len() as u64
            + diagonal_graph.nodes[target.0 as usize].edges.len() as u64;
        let eligible_budget = LocalRouteSearchWork::new(100);
        assert!(super::slingshot::charge_outer_call(
            &diagonal_graph,
            0,
            Orientation::BottomRight,
            Some(&eligible_budget)
        ));
        assert_eq!(eligible_budget.used(), expected);

        let (cardinal_graph, _, _, _) = route_interaction_fixture();
        let cardinal_budget = LocalRouteSearchWork::new(100);
        assert!(super::slingshot::charge_outer_call(
            &cardinal_graph,
            0,
            Orientation::Right,
            Some(&cardinal_budget)
        ));
        assert_eq!(cardinal_budget.used(), 1);

        let (mut clustered_graph, _, source, target) = route_interaction_fixture();
        clustered_graph.nodes[source.0 as usize].cluster = Some(0);
        clustered_graph.nodes[target.0 as usize].position = Some(diagonal_origin);
        clustered_graph.nodes[target.0 as usize].rect.origin = diagonal_origin;
        let cluster_budget = LocalRouteSearchWork::new(100);
        assert!(super::slingshot::charge_outer_call(
            &clustered_graph,
            0,
            Orientation::BottomRight,
            Some(&cluster_budget)
        ));
        assert_eq!(cluster_budget.used(), 1);

        let (mut table_graph, _, _, target) = route_interaction_fixture();
        table_graph.nodes[target.0 as usize].position = Some(diagonal_origin);
        table_graph.nodes[target.0 as usize].rect.origin = diagonal_origin;
        table_graph.edge_records[0].source_table_column = Some(0);
        let table_budget = LocalRouteSearchWork::new(100);
        assert!(super::slingshot::charge_outer_call(
            &table_graph,
            0,
            Orientation::BottomRight,
            Some(&table_budget)
        ));
        assert_eq!(table_budget.used(), 1);
    }

    #[test]
    fn retained_route_expands_the_next_pass_nearby_node_bounds() {
        let mut input = crate::Graph::default();
        let source = input.add_node(routing_test_node("source", 20.0, 20.0));
        let target = input.add_node(routing_test_node("target", 20.0, 20.0));
        let nearby = input.add_node(routing_test_node("nearby", 20.0, 20.0));
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        for (node, point) in [
            (source, Point { x: 0.0, y: 0.0 }),
            (target, Point { x: 80.0, y: 0.0 }),
            (nearby, Point { x: 250.0, y: 0.0 }),
        ] {
            graph.nodes[node.0 as usize].position = Some(point);
            graph.nodes[node.0 as usize].rect.origin = point;
        }
        let subgraphs = vec![
            RoutingSubgraph {
                nodes: vec![source, target],
                edges: vec![0],
            },
            RoutingSubgraph {
                nodes: vec![nearby],
                edges: Vec::new(),
            },
        ];

        assert!(routing_nearby_nodes(&graph, &subgraphs, 0).is_empty());
        graph.edge_records[0].route.replace(RoutePoints::literal(&[
            Point { x: 20.0, y: 10.0 },
            Point { x: 160.0, y: 10.0 },
            Point { x: 80.0, y: 10.0 },
        ]));
        assert_eq!(routing_nearby_nodes(&graph, &subgraphs, 0), vec![nearby]);
    }

    #[test]
    fn nearby_nodes_own_ports_before_the_routed_subgraph() {
        let nearby = NodeId(3);
        let first = NodeId(0);
        let second = NodeId(1);

        let routing_nodes = ovg_routing_nodes(&[first, second], &[nearby]);
        assert_eq!(routing_nodes, vec![nearby, first, second]);
        assert_eq!(
            ovg_port_node_order(
                &[first, second, nearby],
                &routing_nodes,
                &[nearby],
                [first, second, nearby],
            ),
            vec![nearby, first, second],
        );
    }

    #[test]
    fn routed_subgraph_order_controls_ports_after_nearby_nodes() {
        let nearby = NodeId(3);
        let first = NodeId(0);
        let second = NodeId(1);
        let routed_order = [second, first];
        let routing_nodes = ovg_routing_nodes(&routed_order, &[nearby]);

        assert_eq!(
            ovg_port_node_order(
                &routed_order,
                &routing_nodes,
                &[nearby],
                [first, second, nearby],
            ),
            vec![nearby, second, first],
        );
    }

    #[test]
    fn slingshot_obstacles_follow_merged_ovg_node_order() {
        let mut input = crate::Graph::default();
        let h1_late = input.add_node(routing_test_node("h1-late", 20.0, 20.0));
        let ordinary_first = input.add_node(routing_test_node("ordinary-first", 20.0, 20.0));
        let h2_first = input.add_node(routing_test_node("h2-first", 20.0, 20.0));
        let ordinary_second = input.add_node(routing_test_node("ordinary-second", 20.0, 20.0));
        let h1_early = input.add_node(routing_test_node("h1-early", 20.0, 20.0));
        let nearby = input.add_node(routing_test_node("nearby", 20.0, 20.0));
        let mut graph = ArenaGraph::from_input(&input);
        graph.assign_hierarchy_member(h1_late, 1, None, 0, 1);
        graph.assign_hierarchy_member(h2_first, 2, None, 0, 1);
        graph.assign_hierarchy_member(h1_early, 1, None, 0, 1);
        graph.nodes[h1_late.0 as usize].tala_id = 40;
        graph.nodes[h2_first.0 as usize].tala_id = 30;
        graph.nodes[h1_early.0 as usize].tala_id = 20;
        let boxes = [
            h1_late,
            ordinary_first,
            h2_first,
            ordinary_second,
            h1_early,
            nearby,
        ]
        .into_iter()
        .map(|node| (node, Rect::default()))
        .collect();

        assert_eq!(
            ovg_obstacle_node_order(
                &graph,
                &[h1_late, ordinary_first, h2_first, ordinary_second, h1_early],
                &[nearby],
                &boxes,
                None,
            )
            .unwrap(),
            vec![
                nearby,
                ordinary_first,
                ordinary_second,
                h1_early,
                h1_late,
                h2_first,
            ]
        );
    }
}
