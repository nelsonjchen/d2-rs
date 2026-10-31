// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Slingshot candidates that leave crowded endpoints before turning toward a target.
//!
//! Orientation chooses a launch side; visibility search and interaction costs
//! decide whether the longer initial escape produces the better legal route.

use super::{
    ArenaGraph, NodeId, Orientation, Point, Port, PortSide, Rect,
    interactions::edges_can_overlap_all, visibility::VisibilityGraph,
};

use super::route_search_guard::LocalRouteSearchWork;

/// Charge a source-shaped slingshot operation without making the ordinary
/// route representation depend on the diagnostic guard. Go returns the
/// resource error immediately; the Rust caller keeps the guard's exhausted
/// bit and lets the enclosing flavor classify the attempt as a work-limit
/// failure after the worker joins.
fn charge_step(budget: Option<&LocalRouteSearchWork>) -> bool {
    budget.is_none_or(|budget| budget.step().is_ok())
}

fn charge_add(budget: Option<&LocalRouteSearchWork>, units: u64) -> bool {
    budget.is_none_or(|budget| budget.add(units).is_ok())
}

// Go findLaunchings/findLandings visit each canonical OVG port once. Tunnel
// entries can alias an ordinary port, while the raw Ports slice still counts
// both entries for resource accounting. Preserve that raw charge at callers.
fn directional_ports(ports: &[Port], direction: PortSide) -> Vec<Port> {
    let mut selected = Vec::<Port>::new();
    for port in ports
        .iter()
        .copied()
        .filter(|port| port.direction == direction)
    {
        if !selected.iter().any(|prior| prior.point == port.point) {
            selected.push(port);
        }
    }
    selected
}

fn charge_sort(budget: Option<&LocalRouteSearchWork>, length: usize) -> bool {
    budget.is_none_or(|budget| budget.reserve_sort(length).is_ok())
}

fn add_fixed_turn_cost_after_selection(
    best: Option<SlingshotRoute>,
    fixed_turn_cost: f64,
) -> Option<SlingshotRoute> {
    // Keep this as one post-selection addition. Go adds turn cost after
    // comparing each candidate's distance and label penalty; folding it into
    // every candidate can alter IEEE-754 ties.
    best.map(|mut route| {
        route.cost += fixed_turn_cost;
        route
    })
}

/// Charge the outer `ovgEdgeRouter.slingshot` call. Go always charges its
/// entry step, but only reserves endpoint adjacency when the edge passes the
/// cluster, orientation, and table-column eligibility checks.
pub(super) fn charge_outer_call(
    graph: &ArenaGraph,
    edge_index: usize,
    orientation: Orientation,
    budget: Option<&LocalRouteSearchWork>,
) -> bool {
    if !charge_step(budget) {
        return false;
    }
    let edge = &graph.edge_records[edge_index];
    if edge.has_table_column()
        || graph.nodes[edge.from.0 as usize].cluster.is_some()
        || graph.nodes[edge.to.0 as usize].cluster.is_some()
        || !orientation.is_diagonal()
    {
        return true;
    }
    budget.is_none_or(|budget| {
        budget
            .reserve_sum(
                graph.nodes[edge.from.0 as usize].edges.len() as u64,
                graph.nodes[edge.to.0 as usize].edges.len() as u64,
            )
            .is_ok()
    })
}

pub(super) struct SlingshotRoute {
    pub(super) points: Vec<Point>,
    pub(super) node_ids: Vec<usize>,
    pub(super) source: Port,
    pub(super) target: Port,
    pub(super) cost: f64,
}

pub(super) fn complete_route_node_ids(
    graph: &ArenaGraph,
    visibility: &VisibilityGraph,
    edge_index: usize,
    interior: &[usize],
) -> Option<Vec<usize>> {
    let edge = &graph.edge_records[edge_index];
    let mut node_ids = Vec::with_capacity(interior.len() + 2);
    node_ids.push(visibility.center_index(edge.from)?);
    node_ids.extend_from_slice(interior);
    node_ids.push(visibility.center_index(edge.to)?);
    Some(node_ids)
}

fn launch_side(orientation: Orientation, vertical: bool) -> PortSide {
    match (orientation, vertical) {
        (Orientation::TopLeft | Orientation::TopRight, true) => PortSide::Bottom,
        (Orientation::BottomLeft | Orientation::BottomRight, true) => PortSide::Top,
        (Orientation::TopLeft | Orientation::BottomLeft, false) => PortSide::Right,
        (Orientation::TopRight | Orientation::BottomRight, false) => PortSide::Left,
        _ => unreachable!(),
    }
}

fn landing_side(orientation: Orientation, vertical: bool) -> PortSide {
    match (orientation, vertical) {
        (Orientation::TopLeft | Orientation::BottomLeft, true) => PortSide::Left,
        (Orientation::TopRight | Orientation::BottomRight, true) => PortSide::Right,
        (Orientation::TopLeft | Orientation::TopRight, false) => PortSide::Top,
        (Orientation::BottomLeft | Orientation::BottomRight, false) => PortSide::Bottom,
        _ => unreachable!(),
    }
}

fn follows_flight(previous: Point, next: Point, orientation: Orientation, vertical: bool) -> bool {
    match (orientation, vertical) {
        (Orientation::TopLeft | Orientation::TopRight, true) => {
            next.x == previous.x && next.y > previous.y
        }
        (Orientation::BottomLeft | Orientation::BottomRight, true) => {
            next.x == previous.x && next.y < previous.y
        }
        (Orientation::TopLeft | Orientation::BottomLeft, false) => {
            next.y == previous.y && next.x > previous.x
        }
        (Orientation::TopRight | Orientation::BottomRight, false) => {
            next.y == previous.y && next.x < previous.x
        }
        _ => false,
    }
}

fn overshot(
    point: Point,
    target: Rect,
    orientation: Orientation,
    vertical: bool,
    l_route: bool,
) -> bool {
    match (orientation, vertical) {
        (Orientation::TopLeft | Orientation::TopRight, true) => {
            point.y
                > if l_route {
                    target.bottom()
                } else {
                    target.origin.y
                }
        }
        (Orientation::BottomLeft | Orientation::BottomRight, true) => {
            point.y
                < if l_route {
                    target.origin.y
                } else {
                    target.bottom()
                }
        }
        (Orientation::TopLeft | Orientation::BottomLeft, false) => {
            point.x
                > if l_route {
                    target.right()
                } else {
                    target.origin.x
                }
        }
        (Orientation::TopRight | Orientation::BottomRight, false) => {
            point.x
                < if l_route {
                    target.origin.x
                } else {
                    target.right()
                }
        }
        _ => false,
    }
}

fn undershot(point: Point, target: Rect, orientation: Orientation, vertical: bool) -> bool {
    match (orientation, vertical) {
        (Orientation::TopLeft | Orientation::TopRight, true) => point.y < target.origin.y,
        (Orientation::BottomLeft | Orientation::BottomRight, true) => point.y > target.bottom(),
        (Orientation::TopLeft | Orientation::BottomLeft, false) => point.x < target.origin.x,
        (Orientation::TopRight | Orientation::BottomRight, false) => point.x > target.right(),
        _ => false,
    }
}

fn positive_collinear_overlap(a: Point, b: Point, c: Point, d: Point) -> bool {
    if a.x == b.x && c.x == d.x && a.x == c.x {
        // TALA's `isVerticalOrHorizontalOverlap` delegates to the inclusive
        // segment intersection predicate: merely touching at an OVG endpoint
        // still makes the route an occupied overlap for edge sharing.
        a.y.min(b.y) <= c.y.max(d.y) && c.y.min(d.y) <= a.y.max(b.y)
    } else if a.y == b.y && c.y == d.y && a.y == c.y {
        a.x.min(b.x) <= c.x.max(d.x) && c.x.min(d.x) <= a.x.max(b.x)
    } else {
        false
    }
}

fn edge_is_directed(graph: &ArenaGraph, edge_index: usize) -> bool {
    let edge = &graph.edge_records[edge_index];
    edge.source_arrow != edge.target_arrow
}

/// Translation of TALA's `Route.isOpposingColinear`.
///
/// Rust stores accepted routes from source port to target port, whereas Go's
/// `Route.OVGNodes` also contains the source and target centers.  Consequently
/// every segment in this representation is an occupied OVG segment; scanning
/// only `len - 2` entries would incorrectly skip the terminal segment.
/// Endpoint-entry/exit cases still return false before a later collinear
/// segment looks opposing, matching Go's center-inclusive route scan.
fn route_is_opposing_collinear(
    route: &[Point],
    from: Point,
    to: Point,
    route_directed: bool,
) -> bool {
    if !route_directed {
        return false;
    }

    if from.y == to.y {
        let (min_x, max_x) = (from.x.min(to.x), from.x.max(to.x));
        let mut i = 0usize;
        while i + 1 < route.len() {
            if route[i + 1].y != from.y {
                i += 2;
                continue;
            }
            if route[i].y != from.y {
                i += 1;
                continue;
            }
            if route[i + 1].x == route[i].x {
                if route[i + 1].y > route[i].y && to.y < from.y {
                    return true;
                }
                if route[i].y > route[i + 1].y && from.y < to.y {
                    return true;
                }
            } else if route[i + 1].y == route[i].y {
                let (r_min_x, r_max_x) = (
                    route[i].x.min(route[i + 1].x),
                    route[i].x.max(route[i + 1].x),
                );
                if max_x < r_min_x || r_max_x < min_x {
                    i += 1;
                    continue;
                }
                if r_min_x <= from.x
                    && from.x <= r_max_x
                    && (to.x < r_min_x || r_max_x < to.x)
                    && (route[i] == from || route[i + 1] == from)
                {
                    return false;
                }
                if r_min_x <= to.x
                    && to.x <= r_max_x
                    && (from.x < r_min_x || r_max_x < from.x)
                    && (route[i] == to || route[i + 1] == to)
                {
                    return false;
                }
                if route[i + 1].x > route[i].x && to.x < from.x {
                    return true;
                }
                if route[i].x > route[i + 1].x && from.x < to.x {
                    return true;
                }
            }
            i += 1;
        }
    } else if from.x == to.x {
        let (min_y, max_y) = (from.y.min(to.y), from.y.max(to.y));
        let mut i = 0usize;
        while i + 1 < route.len() {
            if route[i + 1].x != from.x {
                i += 2;
                continue;
            }
            if route[i].x != from.x {
                i += 1;
                continue;
            }
            if route[i + 1].y == route[i].y {
                if route[i + 1].x > route[i].x && to.x < from.x {
                    return true;
                }
                if route[i].x > route[i + 1].x && from.x < to.x {
                    return true;
                }
            } else if route[i + 1].x == route[i].x {
                let (r_min_y, r_max_y) = (
                    route[i].y.min(route[i + 1].y),
                    route[i].y.max(route[i + 1].y),
                );
                if max_y < r_min_y || r_max_y < min_y {
                    i += 1;
                    continue;
                }
                if r_min_y <= from.y
                    && from.y <= r_max_y
                    && (to.y < r_min_y || r_max_y < to.y)
                    && (route[i] == from || route[i + 1] == from)
                {
                    return false;
                }
                if r_min_y <= to.y
                    && to.y <= r_max_y
                    && (from.y < r_min_y || r_max_y < from.y)
                    && (route[i] == to || route[i + 1] == to)
                {
                    return false;
                }
                if route[i + 1].y > route[i].y && to.y < from.y {
                    return true;
                }
                if route[i].y > route[i + 1].y && from.y < to.y {
                    return true;
                }
            }
            i += 1;
        }
    }
    false
}

#[derive(Default)]
pub(super) struct SlingshotCheckers {
    occupied: CheckerCache,
    crossed: CheckerCache,
}

#[derive(Default)]
struct CheckerCache {
    values: std::cell::RefCell<std::collections::BTreeMap<(usize, usize), bool>>,
    #[cfg(feature = "diagnostic-traces")]
    trace_flavor: String,
}

impl SlingshotCheckers {
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn with_trace_flavor(mut self, flavor: &str, edge_id: i64) -> Self {
        if std::env::var("D2_TRACE_SLINGSHOT_CHECK_EDGE")
            .ok()
            .as_deref()
            == Some(edge_id.to_string().as_str())
        {
            self.occupied.trace_flavor = flavor.to_owned();
            self.crossed.trace_flavor = flavor.to_owned();
        }
        self
    }
}

impl CheckerCache {
    fn check(
        &self,
        from: usize,
        to: usize,
        budget: Option<&LocalRouteSearchWork>,
        compute: impl FnOnce() -> bool,
    ) -> (bool, bool) {
        if let Some(result) = self.values.borrow().get(&(from, to)).copied() {
            return (result, true);
        }
        let result = compute();
        // Go propagates the compute error without using it as a cached result.
        if budget.is_none_or(|guard| !guard.exhausted())
            && !crate::internal::cancellation_requested()
        {
            self.values.borrow_mut().insert((from, to), result);
        }
        (result, false)
    }
}

fn crossed_cached(
    checkers: &SlingshotCheckers,
    visibility: &VisibilityGraph,
    segments: &RoutedEdgeSet,
    edge_index: usize,
    from: Point,
    to: Point,
    budget: Option<&LocalRouteSearchWork>,
    #[cfg(feature = "diagnostic-traces")] graph: &ArenaGraph,
) -> bool {
    let indices = (visibility.index_of(from), visibility.index_of(to));
    let (result, cache_hit) = match indices {
        (Some(first), Some(second)) => checkers.crossed.check(first, second, budget, || {
            segments.crosses(edge_index, from, to, budget)
        }),
        _ => (segments.crosses(edge_index, from, to, budget), false),
    };
    #[cfg(not(feature = "diagnostic-traces"))]
    let _ = cache_hit;
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var("D2_TRACE_SLINGSHOT_CHECK_EDGE")
        .ok()
        .as_deref()
        == Some(graph.edge_records[edge_index].tala_id.to_string().as_str())
    {
        eprintln!(
            "{}",
            serde_json::json!({"event":"slingshot-check","scope_nodes":graph.nodes.len(),"scope_edges":graph.edge_order.len(),"edge":graph.edge_records[edge_index].tala_id,"kind":"crossed","flavor":checkers.crossed.trace_flavor,"from_index":indices.0,"to_index":indices.1,"from":[format!("{:016x}",from.x.to_bits()),format!("{:016x}",from.y.to_bits())],"to":[format!("{:016x}",to.x.to_bits()),format!("{:016x}",to.y.to_bits())],"hit":cache_hit,"result":result,"work":budget.map(LocalRouteSearchWork::used)})
        );
    }
    result
}

fn occupied_unshareably(
    graph: &ArenaGraph,
    edge_index: usize,
    from: Point,
    to: Point,
    routes: &[Vec<Point>],
    interaction_index: Option<&super::search::RouteInteractionIndex>,
    budget: Option<&LocalRouteSearchWork>,
    cache: Option<(&CheckerCache, usize, usize)>,
) -> bool {
    let compute = || {
        occupied_unshareably_uncached(
            graph,
            edge_index,
            from,
            to,
            routes,
            interaction_index,
            budget,
            #[cfg(feature = "diagnostic-traces")]
            cache
                .map(|(checker, _, _)| checker.trace_flavor.as_str())
                .unwrap_or(""),
        )
    };
    let (result, cache_hit) = match cache {
        Some((checker, first, second)) => checker.check(first, second, budget, compute),
        None => (compute(), false),
    };
    #[cfg(not(feature = "diagnostic-traces"))]
    let _ = cache_hit;
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var("D2_TRACE_SLINGSHOT_CHECK_EDGE")
        .ok()
        .as_deref()
        == Some(graph.edge_records[edge_index].tala_id.to_string().as_str())
    {
        eprintln!(
            "{}",
            serde_json::json!({"event":"slingshot-check","scope_nodes":graph.nodes.len(),"scope_edges":graph.edge_order.len(),"edge":graph.edge_records[edge_index].tala_id,"kind":"occupied","flavor":cache.map(|(checker,_,_)| checker.trace_flavor.as_str()),"from_index":cache.map(|(_,first,_)| first),"to_index":cache.map(|(_,_,second)| second),"from":[format!("{:016x}",from.x.to_bits()),format!("{:016x}",from.y.to_bits())],"to":[format!("{:016x}",to.x.to_bits()),format!("{:016x}",to.y.to_bits())],"hit":cache_hit,"result":result,"work":budget.map(LocalRouteSearchWork::used)})
        );
    }
    result
}

fn occupied_unshareably_uncached(
    graph: &ArenaGraph,
    edge_index: usize,
    from: Point,
    to: Point,
    routes: &[Vec<Point>],
    interaction_index: Option<&super::search::RouteInteractionIndex>,
    budget: Option<&LocalRouteSearchWork>,
    #[cfg(feature = "diagnostic-traces")] trace_flavor: &str,
) -> bool {
    if let Some(interaction_index) = interaction_index {
        let Some(overlapping_routes) =
            interaction_index.overlapping_route_indices(from, to, budget)
        else {
            return true;
        };
        let mut overlapping = Vec::new();
        for route_id in overlapping_routes {
            let route = interaction_index.published_route(route_id);
            let other_index = route.edge;
            if !charge_add(
                budget,
                route.len().saturating_add(1).saturating_mul(2) as u64,
            ) {
                return true;
            }
            overlapping.push(other_index);
            let points = route.points();
            let opposing = edge_is_directed(graph, edge_index)
                && route_is_opposing_collinear(
                    &points[..points.len().saturating_sub(1)],
                    from,
                    to,
                    edge_is_directed(graph, other_index),
                );
            #[cfg(feature = "diagnostic-traces")]
            if std::env::var("D2_TRACE_SLINGSHOT_CHECK_EDGE")
                .ok()
                .as_deref()
                == Some(graph.edge_records[edge_index].tala_id.to_string().as_str())
            {
                eprintln!(
                    "{}",
                    serde_json::json!({"event":"slingshot-occupied-route","scope_nodes":graph.nodes.len(),"scope_edges":graph.edge_order.len(),"edge":graph.edge_records[edge_index].tala_id,"flavor":trace_flavor,"other":graph.edge_records[other_index].tala_id,"nodes":route.len(),"opposing":opposing,"from":[format!("{:016x}",from.x.to_bits()),format!("{:016x}",from.y.to_bits())],"to":[format!("{:016x}",to.x.to_bits()),format!("{:016x}",to.y.to_bits())],"work":budget.map(LocalRouteSearchWork::used)})
                );
            }
            if opposing {
                return true;
            }
        }
        // Source edgeCanOverlapEdgesGuarded reserves ten full passes and
        // polls cancellation before the shareability decision.
        for _ in 0..10 {
            if !charge_add(budget, overlapping.len() as u64) {
                return true;
            }
        }
        if !budget.is_none_or(|guard| guard.check().is_ok()) {
            return true;
        }
        return !overlapping.is_empty() && !edges_can_overlap_all(graph, edge_index, &overlapping);
    }

    // Keep the standalone helper useful for focused geometry tests that do
    // not construct the publication index. Production slingshot calls pass
    // the source-shaped index above.
    let mut overlapping = Vec::new();
    for (other_index, route) in routes.iter().enumerate() {
        if other_index == edge_index {
            continue;
        }
        if route
            .windows(2)
            .any(|segment| positive_collinear_overlap(from, to, segment[0], segment[1]))
        {
            if !charge_add(budget, (route.len() as u64 + 1).saturating_mul(2)) {
                return true;
            }
            overlapping.push(other_index);
            if edge_is_directed(graph, edge_index)
                && route_is_opposing_collinear(
                    route,
                    from,
                    to,
                    edge_is_directed(graph, other_index),
                )
            {
                return true;
            }
        }
    }
    !overlapping.is_empty() && !edges_can_overlap_all(graph, edge_index, &overlapping)
}

/// The Go router keeps accepted OVG segments in an indexed edge set. Preserve
/// that shape instead of scanning every route point: same-axis overlaps are
/// stored by coordinate, while crossing axes are sorted so the skipped prefix
/// can be charged exactly after binary search.
#[derive(Clone, Copy, Debug)]
struct IndexedRouteSegment {
    edge_index: usize,
    from: Point,
    to: Point,
}

#[derive(Default, Debug)]
struct RoutedEdgeSet {
    horizontal_edges: std::collections::BTreeMap<u64, Vec<IndexedRouteSegment>>,
    vertical_edges: std::collections::BTreeMap<u64, Vec<IndexedRouteSegment>>,
    horizontals: Vec<f64>,
    verticals: Vec<f64>,
    edges: std::collections::BTreeSet<(u64, u64, u64, u64)>,
}

impl RoutedEdgeSet {
    fn from_routes(edge_index: usize, routes: &[Vec<Point>]) -> Self {
        let mut set = Self::default();
        for (other_index, route) in routes.iter().enumerate() {
            if other_index == edge_index {
                continue;
            }
            for segment in route.windows(2) {
                set.add(IndexedRouteSegment {
                    edge_index: other_index,
                    from: segment[0],
                    to: segment[1],
                });
            }
        }
        set
    }

    fn add(&mut self, segment: IndexedRouteSegment) {
        let key = (
            super::go_float_map_key(segment.from.x),
            super::go_float_map_key(segment.from.y),
            super::go_float_map_key(segment.to.x),
            super::go_float_map_key(segment.to.y),
        );
        if !self.edges.insert(key) {
            return;
        }
        // `ovgEdgeSet.add` classifies vertical edges before horizontal ones;
        // a zero-length segment therefore follows the vertical path even
        // though it satisfies both predicates.
        if segment.from.x == segment.to.x {
            let axis = segment.from.x;
            let axis_key = super::go_float_map_key(axis);
            if !self.vertical_edges.contains_key(&axis_key) {
                self.verticals.push(axis);
                self.verticals.sort_by(f64::total_cmp);
            }
            let edges = self.vertical_edges.entry(axis_key).or_default();
            let min = segment.from.y.min(segment.to.y);
            let insertion = edges.partition_point(|current| current.from.y.min(current.to.y) < min);
            edges.insert(insertion, segment);
        } else if segment.from.y == segment.to.y {
            let axis = segment.from.y;
            let axis_key = super::go_float_map_key(axis);
            if !self.horizontal_edges.contains_key(&axis_key) {
                self.horizontals.push(axis);
                self.horizontals.sort_by(f64::total_cmp);
            }
            let edges = self.horizontal_edges.entry(axis_key).or_default();
            let min = segment.from.x.min(segment.to.x);
            let insertion = edges.partition_point(|current| current.from.x.min(current.to.x) < min);
            edges.insert(insertion, segment);
        }
    }

    fn crosses(
        &self,
        edge_index: usize,
        from: Point,
        to: Point,
        budget: Option<&LocalRouteSearchWork>,
    ) -> bool {
        let horizontal = from.y == to.y;
        let vertical = from.x == to.x;
        if !horizontal && !vertical {
            return false;
        }
        if horizontal {
            let (min_x, max_x) = (from.x.min(to.x), from.x.max(to.x));
            if let Some(edges) = self.horizontal_edges.get(&super::go_float_map_key(from.y)) {
                for other in edges {
                    if !charge_step(budget) {
                        return true;
                    }
                    let (other_min, other_max) =
                        (other.from.x.min(other.to.x), other.from.x.max(other.to.x));
                    if max_x <= other_min {
                        break;
                    }
                    if other_max <= min_x {
                        continue;
                    }
                    if !shares_endpoint(from, to, other.from, other.to) {
                        return self.trace_cross(edge_index, other, from, to);
                    }
                }
            }
            let start = self.verticals.partition_point(|axis| *axis < min_x);
            if !charge_add(budget, start as u64) {
                return true;
            }
            for axis in self.verticals.iter().skip(start) {
                if !charge_step(budget) {
                    return true;
                }
                if *axis > max_x {
                    break;
                }
                if let Some(edges) = self.vertical_edges.get(&super::go_float_map_key(*axis)) {
                    for other in edges {
                        if !charge_step(budget) {
                            return true;
                        }
                        let (other_min, other_max) =
                            (other.from.y.min(other.to.y), other.from.y.max(other.to.y));
                        if from.y < other_min {
                            break;
                        }
                        if other_max < from.y {
                            continue;
                        }
                        if !shares_endpoint(from, to, other.from, other.to) {
                            return self.trace_cross(edge_index, other, from, to);
                        }
                    }
                }
            }
        } else {
            let (min_y, max_y) = (from.y.min(to.y), from.y.max(to.y));
            if let Some(edges) = self.vertical_edges.get(&super::go_float_map_key(from.x)) {
                for other in edges {
                    if !charge_step(budget) {
                        return true;
                    }
                    let (other_min, other_max) =
                        (other.from.y.min(other.to.y), other.from.y.max(other.to.y));
                    if max_y <= other_min {
                        break;
                    }
                    if other_max <= min_y {
                        continue;
                    }
                    if !shares_endpoint(from, to, other.from, other.to) {
                        return self.trace_cross(edge_index, other, from, to);
                    }
                }
            }
            let start = self.horizontals.partition_point(|axis| *axis < min_y);
            if !charge_add(budget, start as u64) {
                return true;
            }
            for axis in self.horizontals.iter().skip(start) {
                if !charge_step(budget) {
                    return true;
                }
                if *axis > max_y {
                    break;
                }
                if let Some(edges) = self.horizontal_edges.get(&super::go_float_map_key(*axis)) {
                    for other in edges {
                        if !charge_step(budget) {
                            return true;
                        }
                        let (other_min, other_max) =
                            (other.from.x.min(other.to.x), other.from.x.max(other.to.x));
                        if from.x < other_min {
                            break;
                        }
                        if other_max < from.x {
                            continue;
                        }
                        if !shares_endpoint(from, to, other.from, other.to) {
                            return self.trace_cross(edge_index, other, from, to);
                        }
                    }
                }
            }
        }
        false
    }

    fn trace_cross(
        &self,
        edge_index: usize,
        other: &IndexedRouteSegment,
        from: Point,
        to: Point,
    ) -> bool {
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SLINGSHOT_ALL") {
            eprintln!(
                "CROSS_RUST edge={} other={} candidate={:?}->{:?} other_segment={:?}->{:?}",
                edge_index, other.edge_index, from, to, other.from, other.to
            );
        }
        true
    }
}

fn shares_endpoint(
    first_from: Point,
    first_to: Point,
    second_from: Point,
    second_to: Point,
) -> bool {
    first_from == second_from
        || first_from == second_to
        || first_to == second_from
        || first_to == second_to
}

fn fill_path(
    visibility: &VisibilityGraph,
    from: usize,
    to: usize,
    budget: Option<&LocalRouteSearchWork>,
) -> Option<Vec<usize>> {
    let from_point = visibility.point(from);
    let to_point = visibility.point(to);
    let mut path = vec![from];
    let mut current = from;
    while current != to {
        if !charge_step(budget) {
            return None;
        }
        let current_point = visibility.point(current);
        let mut next = None;
        for candidate in visibility.neighbors(current) {
            if !charge_step(budget) {
                return None;
            }
            let point = visibility.point(candidate);
            let on_path = if from_point.x == to_point.x && point.x == current_point.x {
                (from_point.y < to_point.y && current_point.y < point.y)
                    || (from_point.y > to_point.y && current_point.y > point.y)
            } else if from_point.y == to_point.y && point.y == current_point.y {
                (from_point.x < to_point.x && current_point.x < point.x)
                    || (from_point.x > to_point.x && current_point.x > point.x)
            } else {
                false
            };
            if on_path {
                next = Some(candidate);
                break;
            }
        }
        let next = next?;
        current = next;
        if current != to {
            path.push(current);
        }
    }
    Some(path)
}

fn path_is_occupied_unshareably(
    graph: &ArenaGraph,
    visibility: &VisibilityGraph,
    edge_index: usize,
    path: &[usize],
    routes: &[Vec<Point>],
    interaction_index: Option<&super::search::RouteInteractionIndex>,
    budget: Option<&LocalRouteSearchWork>,
    checkers: Option<&SlingshotCheckers>,
) -> bool {
    path.windows(2).any(|pair| {
        if !charge_step(budget) {
            return true;
        }
        let occupied = occupied_unshareably(
            graph,
            edge_index,
            visibility.point(pair[0]),
            visibility.point(pair[1]),
            routes,
            interaction_index,
            budget,
            checkers.map(|cache| (&cache.occupied, pair[0], pair[1])),
        );
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SLINGSHOT_ALL")
            && graph.nodes[graph.edge_records[edge_index].from.0 as usize].tala_id == 1070434817
            && graph.nodes[graph.edge_records[edge_index].to.0 as usize].tala_id == 3340145390
        {
            eprintln!(
                "SLINGSHOT_PATH_RUST edge={} from={:?} to={:?} occupied={}",
                edge_index,
                visibility.point(pair[0]),
                visibility.point(pair[1]),
                occupied
            );
        }
        occupied
    })
}

/// Match the `findLShapedRoute` duplicate-route guard. Go compares the
/// complete center-inclusive OVG path, while Rust stores only the source-port
/// through target-port points. The endpoint and length tests therefore map
/// directly; the interior comparison skips both ports in the same way as the
/// upstream `i := 2; i < len(path)-2` loop. The upstream S-shaped path has no
/// duplicate-route check, so its translation deliberately does not call this
/// helper.
fn duplicate_route_exists(
    routes: &[Vec<Point>],
    points: &[Point],
    source: Point,
    target: Point,
    budget: Option<&LocalRouteSearchWork>,
) -> Option<bool> {
    for route in routes.iter().filter(|route| !route.is_empty()) {
        if !charge_step(budget) {
            return None;
        }
        if route.first() != Some(&source)
            || route.last() != Some(&target)
            || route.len() != points.len()
        {
            continue;
        }
        let mut duplicate = true;
        for (existing, candidate) in route
            .iter()
            .zip(points)
            .skip(1)
            .take(points.len().saturating_sub(2))
        {
            if !charge_step(budget) {
                return None;
            }
            if existing != candidate {
                duplicate = false;
                break;
            }
        }
        if duplicate {
            return Some(true);
        }
    }
    Some(false)
}

/// Recovered `Nodes.intersectsNode` rejects a slingshot flight segment when it
/// touches any routed-scope node that is not an ancestor of either endpoint.
///
/// This is stricter than OVG adjacency: the visibility graph can contain a
/// collinear chain through container boundary nodes, but `launch` validates
/// the complete unsplit source-to-anchor and anchor-to-target segments again.
fn launch_segment_is_clear(
    graph: &ArenaGraph,
    boxes: &std::collections::BTreeMap<NodeId, Rect>,
    obstacle_node_order: &[NodeId],
    edge_index: usize,
    from: Point,
    to: Point,
    budget: Option<&LocalRouteSearchWork>,
) -> bool {
    let edge = &graph.edge_records[edge_index];
    obstacle_node_order.iter().all(|node| {
        if !charge_step(budget) {
            return false;
        }
        let rect = boxes
            .get(node)
            .expect("OVG obstacle is present in the route box inventory");
        // Go's `Node.isDescendentOf` includes the node itself and follows
        // active container/cluster/sequence ownership. Such nodes are part
        // of the endpoint's own vessel, not obstacles for this flight leg.
        if *node == edge.from
            || *node == edge.to
            || graph.is_descendant_of_scope(edge.from, Some(*node))
            || graph.is_descendant_of_scope(edge.to, Some(*node))
        {
            return true;
        }
        if !charge_step(budget) {
            return false;
        }
        let intersects = super::route_geometry::recovered_segment_intersects_box(from, to, *rect);
        if intersects && crate::internal::trace_env_enabled("WEFTAN_TRACE_SLINGSHOT_OBSTACLE") {
            eprintln!(
                "SLINGSHOT_OBSTACLE_RUST edge={}->{} obstacle={} rect={:?} from={:?} to={:?}",
                graph.nodes[edge.from.0 as usize].tala_id,
                graph.nodes[edge.to.0 as usize].tala_id,
                graph.nodes[node.0 as usize].tala_id,
                rect,
                from,
                to
            );
        }
        !intersects
    })
}

/// Translation of the L-shaped half of recovered `ovgEdgeRouter.slingshot`.
/// A successful L route wins immediately in TALA and is charged exactly one
/// turn. S-shaped recovery remains the next stage when this returns `None`.
pub(super) fn l_route(
    graph: &ArenaGraph,
    visibility: &VisibilityGraph,
    edge_index: usize,
    ports: &std::collections::BTreeMap<NodeId, Vec<Port>>,
    boxes: &std::collections::BTreeMap<NodeId, Rect>,
    obstacle_node_order: &[NodeId],
    routes: &[Vec<Point>],
    interaction_index: &super::search::RouteInteractionIndex,
    _trace_flavor: &str,
    budget: Option<&LocalRouteSearchWork>,
    checkers: &SlingshotCheckers,
) -> Option<SlingshotRoute> {
    let edge = &graph.edge_records[edge_index];
    if edge.has_table_column()
        || graph.nodes[edge.from.0 as usize].cluster.is_some()
        || graph.nodes[edge.to.0 as usize].cluster.is_some()
    {
        return None;
    }
    let orientation = graph.sized_orientation(edge.from, edge.to);
    if !orientation.is_diagonal() {
        return None;
    }
    // The Go implementation charges the L-shaped worker after the outer
    // slingshot call has passed its eligibility checks.
    if !charge_step(budget) {
        return None;
    }
    let (vertical_preference, strong_preference) =
        super::prefer_launching_vertically(graph, edge.from, edge.to, orientation);
    let order = if vertical_preference {
        [true, false]
    } else {
        [false, true]
    };
    let routed_segments = RoutedEdgeSet::from_routes(edge_index, routes);
    let mut best = None::<SlingshotRoute>;
    for vertical in order {
        if !charge_step(budget) {
            return None;
        }
        if !charge_add(budget, ports[&edge.from].len() as u64) {
            return None;
        }
        let source_ports =
            directional_ports(&ports[&edge.from], launch_side(orientation, vertical));
        if !charge_add(budget, ports[&edge.to].len() as u64) {
            return None;
        }
        let target_ports = directional_ports(&ports[&edge.to], landing_side(orientation, vertical));
        for source_port in source_ports {
            if !charge_step(budget) {
                return None;
            }
            let mut current = visibility.index_of(source_port.point)?;
            loop {
                if !charge_step(budget) {
                    return None;
                }
                let current_point = visibility.point(current);
                let Some(next) = visibility.neighbors(current).find(|next| {
                    charge_step(budget)
                        && follows_flight(
                            current_point,
                            visibility.point(*next),
                            orientation,
                            vertical,
                        )
                }) else {
                    break;
                };
                let next_point = visibility.point(next);
                let is_overshot =
                    overshot(next_point, boxes[&edge.to], orientation, vertical, true);
                let is_occupied = if is_overshot {
                    false
                } else {
                    occupied_unshareably(
                        graph,
                        edge_index,
                        current_point,
                        next_point,
                        routes,
                        Some(interaction_index),
                        budget,
                        Some((&checkers.occupied, current, next)),
                    )
                };
                if is_overshot || is_occupied {
                    break;
                }
                if undershot(next_point, boxes[&edge.to], orientation, vertical) {
                    current = next;
                    continue;
                }
                for target_port in target_ports.iter().copied() {
                    if !charge_step(budget) {
                        return None;
                    }
                    if (vertical && next_point.y != target_port.point.y)
                        || (!vertical && next_point.x != target_port.point.x)
                    {
                        continue;
                    }
                    let Some(target_index) = visibility.index_of(target_port.point) else {
                        continue;
                    };
                    let source_clear = launch_segment_is_clear(
                        graph,
                        boxes,
                        obstacle_node_order,
                        edge_index,
                        source_port.point,
                        next_point,
                        budget,
                    );
                    if budget.is_some_and(|budget| budget.exhausted()) {
                        return None;
                    }
                    if !source_clear {
                        continue;
                    }
                    let target_clear = launch_segment_is_clear(
                        graph,
                        boxes,
                        obstacle_node_order,
                        edge_index,
                        target_port.point,
                        next_point,
                        budget,
                    );
                    if budget.is_some_and(|budget| budget.exhausted()) {
                        return None;
                    }
                    if !target_clear {
                        continue;
                    }
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_SLINGSHOT")
                        && ((target_port.point.y == 1339.0
                            && (target_port.point.x == 1472.0 || target_port.point.x == 1487.0))
                            || (target_port.point.y == 1471.0
                                && (target_port.point.x == 2178.0
                                    || target_port.point.x == 2193.0
                                    || target_port.point.x == 2209.0)))
                    {
                        eprintln!(
                            "SLINGSHOT_L_RUST from={} to={} vertical={} source={:?} next={:?} target={:?} source_clear={} target_clear={}",
                            graph.nodes[edge.from.0 as usize].tala_id,
                            graph.nodes[edge.to.0 as usize].tala_id,
                            vertical,
                            source_port.point,
                            next_point,
                            target_port.point,
                            source_clear,
                            target_clear
                        );
                    }
                    let mut cost = crate::internal::routing::channel_points::go_hypot(
                        source_port.point.x - next_point.x,
                        source_port.point.y - next_point.y,
                    ) + crate::internal::routing::channel_points::go_hypot(
                        target_port.point.x - next_point.x,
                        target_port.point.y - next_point.y,
                    );
                    let source_crosses = crossed_cached(
                        checkers,
                        visibility,
                        &routed_segments,
                        edge_index,
                        source_port.point,
                        next_point,
                        budget,
                        #[cfg(feature = "diagnostic-traces")]
                        graph,
                    );
                    if budget.is_some_and(|budget| budget.exhausted()) {
                        return None;
                    }
                    if source_crosses {
                        cost += graph.routing_costs.0;
                    }
                    let target_crosses = crossed_cached(
                        checkers,
                        visibility,
                        &routed_segments,
                        edge_index,
                        next_point,
                        target_port.point,
                        budget,
                        #[cfg(feature = "diagnostic-traces")]
                        graph,
                    );
                    if budget.is_some_and(|budget| budget.exhausted()) {
                        return None;
                    }
                    if target_crosses {
                        cost += graph.routing_costs.0;
                    }
                    if strong_preference && vertical_preference != vertical {
                        cost += graph.routing_costs.0 * 0.5;
                    }
                    // Go compares the distance/crossing/preference subtotal
                    // against the best label-inclusive score before building
                    // the route. Keep this comparison ahead of fillPath and
                    // leave the fixed turn charge until the selected route
                    // is returned; adding it per candidate can change both
                    // rounding and tie selection.
                    if best.as_ref().is_some_and(|best| cost >= best.cost) {
                        continue;
                    }
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_SLINGSHOT_ALL") {
                        eprintln!(
                            "L_RUST edge={}->{} vertical={} candidate source={:?} next={:?} target={:?} d={}",
                            graph.nodes[edge.from.0 as usize].tala_id,
                            graph.nodes[edge.to.0 as usize].tala_id,
                            vertical,
                            source_port.point,
                            next_point,
                            target_port.point,
                            cost,
                        );
                    }
                    let Some(mut first) = fill_path(
                        visibility,
                        visibility.index_of(source_port.point)?,
                        next,
                        budget,
                    ) else {
                        continue;
                    };
                    let Some(second) = fill_path(visibility, next, target_index, budget) else {
                        continue;
                    };
                    // Recovered findLShapedRoute appends the target port to
                    // fillPath's target-exclusive result, then checks
                    // `i < len(p)-2`. That covers every segment returned by
                    // fillPath while deliberately excluding the final hop
                    // into the target port.
                    if path_is_occupied_unshareably(
                        graph,
                        visibility,
                        edge_index,
                        &second,
                        routes,
                        Some(interaction_index),
                        budget,
                        Some(checkers),
                    ) {
                        continue;
                    }
                    first.extend(second);
                    first.push(target_index);
                    let Some(node_ids) =
                        complete_route_node_ids(graph, visibility, edge_index, &first)
                    else {
                        continue;
                    };
                    let points = first
                        .iter()
                        .map(|index| visibility.point(*index))
                        .collect::<Vec<_>>();
                    if duplicate_route_exists(
                        routes,
                        &points,
                        source_port.point,
                        target_port.point,
                        budget,
                    ) != Some(false)
                    {
                        continue;
                    }
                    cost += interaction_index.candidate_arrowhead_label_cost_for_route_guarded(
                        graph, edge_index, &points, budget,
                    )?;
                    if best.as_ref().is_none_or(|best| cost < best.cost) {
                        if crate::internal::trace_env_enabled("WEFTAN_TRACE_SLINGSHOT_ALL") {
                            eprintln!(
                                "L_RUST edge={}->{} selected source={:?} next={:?} target={:?} d={}",
                                graph.nodes[edge.from.0 as usize].tala_id,
                                graph.nodes[edge.to.0 as usize].tala_id,
                                source_port.point,
                                next_point,
                                target_port.point,
                                cost,
                            );
                        }
                        best = Some(SlingshotRoute {
                            points,
                            node_ids,
                            source: source_port,
                            target: target_port,
                            cost,
                        });
                    }
                }
                current = next;
            }
        }
    }
    add_fixed_turn_cost_after_selection(best, graph.routing_costs.1)
}

/// Translation of recovered `findSShapedRoute`, which runs only after the
/// L-shaped slingshot has no legal path.
pub(super) fn s_route(
    graph: &ArenaGraph,
    visibility: &VisibilityGraph,
    edge_index: usize,
    ports: &std::collections::BTreeMap<NodeId, Vec<Port>>,
    boxes: &std::collections::BTreeMap<NodeId, Rect>,
    obstacle_node_order: &[NodeId],
    routes: &[Vec<Point>],
    interaction_index: &super::search::RouteInteractionIndex,
    trace_flavor: &str,
    budget: Option<&LocalRouteSearchWork>,
    checkers: &SlingshotCheckers,
) -> Option<SlingshotRoute> {
    let edge = &graph.edge_records[edge_index];
    if edge.has_table_column()
        || graph.nodes[edge.from.0 as usize].cluster.is_some()
        || graph.nodes[edge.to.0 as usize].cluster.is_some()
    {
        return None;
    }
    let orientation = graph.sized_orientation(edge.from, edge.to);
    if !orientation.is_diagonal() {
        return None;
    }
    // The outer slingshot charge is shared with the L attempt; this is the
    // entry charge for the S-shaped worker.
    if !charge_step(budget) {
        return None;
    }
    let (vertical_preference, strong_preference) =
        super::prefer_launching_vertically(graph, edge.from, edge.to, orientation);
    let order = if vertical_preference {
        [true, false]
    } else {
        [false, true]
    };
    let ideal_axes =
        super::search::ideal_turn_axes_for_slingshot(boxes[&edge.from], boxes[&edge.to]);
    let routed_segments = RoutedEdgeSet::from_routes(edge_index, routes);
    let mut best = None::<SlingshotRoute>;
    let trace_s = crate::internal::trace_env_enabled("WEFTAN_TRACE_SLINGSHOT")
        && (crate::internal::trace_env_enabled("WEFTAN_TRACE_SLINGSHOT_ALL")
            || (graph.nodes[edge.from.0 as usize].tala_id == 1036879579
                && graph.nodes[edge.to.0 as usize].tala_id == 3356923009)
            || (graph.nodes[edge.from.0 as usize].tala_id == 1070434817
                && graph.nodes[edge.to.0 as usize].tala_id == 3340145390)
            || (graph.nodes[edge.from.0 as usize].tala_id == 3390478247
                && graph.nodes[edge.to.0 as usize].tala_id == 1120767674));

    for vertical in order {
        if !charge_step(budget) {
            return None;
        }
        if !charge_add(budget, ports[&edge.from].len() as u64) {
            return None;
        }
        let source_ports =
            directional_ports(&ports[&edge.from], launch_side(orientation, vertical));
        if !charge_add(budget, ports[&edge.to].len() as u64) {
            return None;
        }
        let target_ports =
            directional_ports(&ports[&edge.to], landing_side(orientation, !vertical));
        if trace_s {
            eprint!(
                "SLINGSHOT_S_RUST flavor={trace_flavor} edge={}->{} ports vertical={vertical} source=",
                graph.nodes[edge.from.0 as usize].tala_id, graph.nodes[edge.to.0 as usize].tala_id
            );
            for port in &source_ports {
                eprint!("{:?}/{:?};", port.point, port.direction);
            }
            eprint!(" target=");
            for port in &target_ports {
                eprint!("{:?}/{:?};", port.point, port.direction);
            }
            eprintln!();
        }
        let mut source_anchor_ports = std::collections::BTreeMap::<usize, Port>::new();
        let mut target_anchor_ports = std::collections::BTreeMap::<usize, Port>::new();
        let mut source_anchor_order = Vec::new();
        let mut target_anchor_order = Vec::new();

        for source_port in source_ports {
            if !charge_step(budget) {
                return None;
            }
            let Some(mut current) = visibility.index_of(source_port.point) else {
                continue;
            };
            loop {
                if !charge_step(budget) {
                    return None;
                }
                let current_point = visibility.point(current);
                let Some(next) = visibility.neighbors(current).find(|next| {
                    charge_step(budget)
                        && follows_flight(
                            current_point,
                            visibility.point(*next),
                            orientation,
                            vertical,
                        )
                }) else {
                    break;
                };
                let next_point = visibility.point(next);
                if overshot(next_point, boxes[&edge.to], orientation, vertical, false)
                    || occupied_unshareably(
                        graph,
                        edge_index,
                        current_point,
                        next_point,
                        routes,
                        Some(interaction_index),
                        budget,
                        Some((&checkers.occupied, current, next)),
                    )
                {
                    break;
                }
                current = next;
                source_anchor_ports.insert(current, source_port);
                source_anchor_order.push(current);
            }
        }
        let reverse_orientation = orientation.opposite();
        for target_port in target_ports {
            if !charge_step(budget) {
                return None;
            }
            let Some(mut current) = visibility.index_of(target_port.point) else {
                continue;
            };
            loop {
                if !charge_step(budget) {
                    return None;
                }
                let current_point = visibility.point(current);
                let Some(next) = visibility.neighbors(current).find(|next| {
                    let rejected = visibility.is_rejected_hierarchy_point(*next);
                    !rejected
                        && charge_step(budget)
                        && follows_flight(
                            current_point,
                            visibility.point(*next),
                            reverse_orientation,
                            vertical,
                        )
                }) else {
                    break;
                };
                let next_point = visibility.point(next);
                let is_overshot = overshot(
                    next_point,
                    boxes[&edge.from],
                    reverse_orientation,
                    vertical,
                    false,
                );
                // Go stops an overshot target launch before consulting the
                // occupied-route checker. Even a zero-work miss is observable
                // in its checker cache and diagnostic execution order.
                let is_occupied = !is_overshot
                    && occupied_unshareably(
                        graph,
                        edge_index,
                        current_point,
                        next_point,
                        routes,
                        Some(interaction_index),
                        budget,
                        Some((&checkers.occupied, current, next)),
                    );
                if trace_s {
                    eprintln!(
                        "SLINGSHOT_S_RUST flavor={trace_flavor} target_walk port={:?} current={:?} next={:?} overshot={} occupied={}",
                        target_port.point, current_point, next_point, is_overshot, is_occupied
                    );
                }
                if is_overshot || is_occupied {
                    break;
                }
                current = next;
                target_anchor_ports.insert(current, target_port);
                target_anchor_order.push(current);
            }
        }
        if trace_s {
            eprint!(
                "SLINGSHOT_S_RUST flavor={trace_flavor} edge={}->{} anchors vertical={vertical} source=",
                graph.nodes[edge.from.0 as usize].tala_id, graph.nodes[edge.to.0 as usize].tala_id
            );
            for index in &source_anchor_order {
                eprint!("{:?};", visibility.point(*index));
            }
            eprint!(" target=");
            for index in &target_anchor_order {
                eprint!("{:?};", visibility.point(*index));
            }
            eprintln!();
        }
        if !charge_sort(budget, source_anchor_order.len()) {
            return None;
        }
        source_anchor_order.sort_by(|left, right| {
            let left = visibility.point(*left);
            let right = visibility.point(*right);
            let axis = if vertical {
                ideal_axes.get(1).map_or(left.y, |(_, position)| *position)
            } else {
                ideal_axes.first().map_or(left.x, |(_, position)| *position)
            };
            let left_distance = if vertical {
                (left.y - axis).abs()
            } else {
                (left.x - axis).abs()
            };
            let right_distance = if vertical {
                (right.y - axis).abs()
            } else {
                (right.x - axis).abs()
            };
            left_distance.total_cmp(&right_distance)
        });

        for source_anchor in source_anchor_order.iter().copied() {
            if !charge_step(budget) {
                return None;
            }
            let source_port = source_anchor_ports[&source_anchor];
            for target_anchor in target_anchor_order.iter().copied() {
                if !charge_step(budget) {
                    return None;
                }
                let target_port = target_anchor_ports[&target_anchor];
                let source_anchor_point = visibility.point(source_anchor);
                let target_anchor_point = visibility.point(target_anchor);
                if (vertical && source_anchor_point.y != target_anchor_point.y)
                    || (!vertical && source_anchor_point.x != target_anchor_point.x)
                {
                    continue;
                }
                let pairs = [
                    (source_port.point, source_anchor_point),
                    (source_anchor_point, target_anchor_point),
                    (target_anchor_point, target_port.point),
                ];
                let mut clear = true;
                let mut cost = 0.0;
                for (from, to) in pairs {
                    if !charge_step(budget) {
                        return None;
                    }
                    let segment_clear = launch_segment_is_clear(
                        graph,
                        boxes,
                        obstacle_node_order,
                        edge_index,
                        from,
                        to,
                        budget,
                    );
                    if budget.is_some_and(|budget| budget.exhausted()) {
                        return None;
                    }
                    if !segment_clear {
                        clear = false;
                        break;
                    }
                    cost += crate::internal::routing::channel_points::go_hypot(
                        from.x - to.x,
                        from.y - to.y,
                    );
                    let crosses = crossed_cached(
                        checkers,
                        visibility,
                        &routed_segments,
                        edge_index,
                        from,
                        to,
                        budget,
                        #[cfg(feature = "diagnostic-traces")]
                        graph,
                    );
                    if budget.is_some_and(|budget| budget.exhausted()) {
                        return None;
                    }
                    if crosses {
                        cost += graph.routing_costs.0;
                    }
                }
                if trace_s {
                    eprintln!(
                        "SLINGSHOT_S_RUST flavor={trace_flavor} edge={}->{} candidate source={:?} anchor={:?} target={:?} clear={}",
                        graph.nodes[edge.from.0 as usize].tala_id,
                        graph.nodes[edge.to.0 as usize].tala_id,
                        source_port.point,
                        source_anchor_point,
                        target_anchor_point,
                        clear
                    );
                }
                if !clear {
                    continue;
                }
                if strong_preference && vertical_preference != vertical {
                    cost += graph.routing_costs.0 * 0.5;
                }
                // findSShapedRoute checks its subtotal against the current
                // label-inclusive winner before filling any route paths.
                // The two turn costs are added once, after selection.
                if best.as_ref().is_some_and(|best| cost >= best.cost) {
                    continue;
                }
                if trace_s {
                    eprintln!(
                        "SLINGSHOT_S_RUST flavor={trace_flavor} edge={}->{} cost source={:?} anchor={:?} target={:?} prepath={}",
                        graph.nodes[edge.from.0 as usize].tala_id,
                        graph.nodes[edge.to.0 as usize].tala_id,
                        source_port.point,
                        source_anchor_point,
                        target_anchor_point,
                        cost
                    );
                }
                let source_port_index = visibility.index_of(source_port.point)?;
                let target_port_index = visibility.index_of(target_port.point)?;
                let Some(mut first) =
                    fill_path(visibility, source_port_index, source_anchor, budget)
                else {
                    if trace_s {
                        eprintln!(
                            "SLINGSHOT_S_RUST reject source_fill source={:?} anchor={:?}",
                            source_port.point, source_anchor_point
                        );
                    }
                    continue;
                };
                let Some(middle) = fill_path(visibility, source_anchor, target_anchor, budget)
                else {
                    if trace_s {
                        eprintln!(
                            "SLINGSHOT_S_RUST reject middle_fill anchor={:?} target={:?}",
                            source_anchor_point, target_anchor_point
                        );
                    }
                    continue;
                };
                // As in findLShapedRoute, recovered findSShapedRoute appends
                // the target anchor before its `len(p)-2` loop. Since
                // fill_path excludes that anchor, every segment present here
                // is checked.
                if path_is_occupied_unshareably(
                    graph,
                    visibility,
                    edge_index,
                    &middle,
                    routes,
                    Some(interaction_index),
                    budget,
                    Some(checkers),
                ) {
                    if trace_s {
                        eprintln!(
                            "SLINGSHOT_S_RUST reject occupied_middle anchor={:?} target={:?}",
                            source_anchor_point, target_anchor_point
                        );
                    }
                    continue;
                }
                first.extend(
                    middle
                        .into_iter()
                        .take_while(|index| *index != target_anchor),
                );
                let Some(last) = fill_path(visibility, target_anchor, target_port_index, budget)
                else {
                    if trace_s {
                        eprintln!(
                            "SLINGSHOT_S_RUST reject target_fill target={:?} port={:?}",
                            target_anchor_point, target_port.point
                        );
                    }
                    continue;
                };
                first.extend(last);
                first.push(target_port_index);
                let Some(node_ids) = complete_route_node_ids(graph, visibility, edge_index, &first)
                else {
                    continue;
                };
                let points = first
                    .iter()
                    .map(|index| visibility.point(*index))
                    .collect::<Vec<_>>();
                cost += interaction_index.candidate_arrowhead_label_cost_for_route_guarded(
                    graph, edge_index, &points, budget,
                )?;
                if best.as_ref().is_none_or(|best| cost < best.cost) {
                    if trace_s {
                        eprintln!(
                            "SLINGSHOT_S_RUST flavor={trace_flavor} edge={}->{} selected source={:?} anchor={:?} target={:?} cost={}",
                            graph.nodes[edge.from.0 as usize].tala_id,
                            graph.nodes[edge.to.0 as usize].tala_id,
                            source_port.point,
                            source_anchor_point,
                            target_anchor_point,
                            cost
                        );
                    }
                    best = Some(SlingshotRoute {
                        points,
                        node_ids,
                        source: source_port,
                        target: target_port,
                        cost,
                    });
                }
            }
        }
    }
    add_fixed_turn_cost_after_selection(best, graph.routing_costs.1 * 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentAlignment, Graph, Insets, LabelPosition, Node, ShapeKind, Size};

    #[test]
    fn routed_edge_axes_match_go_float_map_equality_for_signed_zero() {
        let mut routed = RoutedEdgeSet::default();
        routed.add(IndexedRouteSegment {
            edge_index: 1,
            from: Point { x: -0.0, y: 0.0 },
            to: Point { x: -0.0, y: 10.0 },
        });
        routed.add(IndexedRouteSegment {
            edge_index: 2,
            from: Point { x: 0.0, y: 20.0 },
            to: Point { x: 0.0, y: 30.0 },
        });

        assert_eq!(routed.verticals.len(), 1);
        assert_eq!(routed.vertical_edges.len(), 1);
        assert_eq!(routed.vertical_edges.values().next().unwrap().len(), 2);
    }

    fn node(name: &str, position: Point) -> Node {
        Node {
            d2_near_key: None,
            external_id: name.into(),
            size: Size {
                width: 20.0,
                height: 20.0,
            },
            declared_size: None,
            d2_authored_geometry: None,
            label_size: None,
            font_size: None,
            label_position: LabelPosition::Unset,
            parent: None,
            locked_position: Some(position),
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
            shape: ShapeKind::Rectangle,
        }
    }

    #[test]
    fn checker_cache_does_not_store_work_limit_failures() {
        let cache = CheckerCache::default();
        let work = LocalRouteSearchWork::new(0);
        assert_eq!(
            cache.check(1, 2, Some(&work), || {
                assert!(work.step().is_err());
                true
            }),
            (true, false)
        );
        assert!(work.exhausted());
        assert!(cache.values.borrow().is_empty());
        let fresh = LocalRouteSearchWork::new(10);
        assert_eq!(cache.check(1, 2, Some(&fresh), || false), (false, false));
    }

    #[test]
    fn checker_cache_preserves_directed_node_identity_and_false_results() {
        let cache = CheckerCache::default();
        assert_eq!(cache.check(1, 2, None, || false), (false, false));
        assert_eq!(
            cache.check(1, 2, None, || panic!("cached false recomputed")),
            (false, true)
        );
        assert_eq!(cache.check(2, 1, None, || true), (true, false));
        assert_eq!(cache.check(3, 2, None, || true), (true, false));
        assert_eq!(
            cache.check(2, 1, None, || panic!("cached true recomputed")),
            (true, true)
        );
        // Each outer slingshot owns separate occupied/crossed caches; a new
        // edge cannot inherit a result after its route inventory changes.
        let next = SlingshotCheckers::default();
        assert_eq!(next.occupied.check(1, 2, None, || true), (true, false));
        assert_eq!(next.crossed.check(1, 2, None, || false), (false, false));
    }

    #[test]
    fn slingshot_candidates_deduplicate_canonical_tunnel_ports() {
        let ordinary = Port {
            point: Point::default(),
            direction: PortSide::Right,
            side: PortSide::Right,
            index: 0,
            tunnel: None,
            is_center: false,
        };
        let tunnel = Port {
            tunnel: Some(1),
            ..ordinary
        };
        let other_direction = Port {
            direction: PortSide::Left,
            ..ordinary
        };
        let ports = [ordinary, tunnel, other_direction];
        assert_eq!(directional_ports(&ports, PortSide::Right).len(), 1);
        assert_eq!(directional_ports(&ports, PortSide::Left).len(), 1);
        assert_eq!(directional_ports(&ports, PortSide::Right)[0].tunnel, None);
        assert_eq!(ports.len(), 3);
    }

    #[test]
    fn turn_cost_is_added_after_label_cost_for_source_float_order() {
        let base_distance = 39.595_000_000_005_f64;
        let label_penalty = 9.315_000_000_005_f64;
        let turn_cost = 236.450_000_000_005_f64;
        let port = Port {
            point: Point::default(),
            direction: PortSide::Right,
            side: PortSide::Right,
            index: 0,
            tunnel: None,
            is_center: false,
        };
        let selected = SlingshotRoute {
            points: Vec::new(),
            node_ids: Vec::new(),
            source: port,
            target: port,
            cost: base_distance + label_penalty,
        };

        let finished = add_fixed_turn_cost_after_selection(Some(selected), turn_cost)
            .expect("selected L or S route");
        let source_order = (base_distance + label_penalty) + turn_cost;
        let per_candidate_turn = (base_distance + turn_cost) + label_penalty;

        assert_eq!(finished.cost.to_bits(), source_order.to_bits());
        assert_eq!(source_order.to_bits(), 0x4071_d5c2_8f5c_29fe);
        assert_ne!(finished.cost.to_bits(), per_candidate_turn.to_bits());
    }

    #[test]
    fn recovered_launch_rechecks_the_complete_leg_against_nodes() {
        let mut input = Graph::default();
        let source = input.add_node(node("source", Point { x: 0.0, y: 0.0 }));
        let target = input.add_node(node("target", Point { x: 100.0, y: 100.0 }));
        let obstacle = input.add_node(node("obstacle", Point { x: 40.0, y: 0.0 }));
        input.add_edge(crate::Edge { source, target });
        let graph = ArenaGraph::from_input(&input);
        let boxes = [source, target, obstacle]
            .into_iter()
            .map(|node| {
                let arena_node = &graph.nodes[node.0 as usize];
                (
                    node,
                    Rect {
                        origin: arena_node.position.unwrap(),
                        size: arena_node.rect.size,
                    },
                )
            })
            .collect();

        assert!(!launch_segment_is_clear(
            &graph,
            &boxes,
            &[source, target, obstacle],
            0,
            Point { x: 20.0, y: 10.0 },
            Point { x: 100.0, y: 10.0 },
            None,
        ));
        assert!(launch_segment_is_clear(
            &graph,
            &boxes,
            &[source, target, obstacle],
            0,
            Point { x: 10.0, y: 20.0 },
            Point { x: 10.0, y: 80.0 },
            None,
        ));
    }

    #[test]
    fn target_exclusive_fill_path_checks_its_only_middle_segment() {
        let mut input = Graph::default();
        let first_source = input.add_node(node("first-source", Point { x: 0.0, y: 0.0 }));
        let first_target = input.add_node(node("first-target", Point { x: 100.0, y: 0.0 }));
        let second_source = input.add_node(node("second-source", Point { x: 0.0, y: 100.0 }));
        let second_target = input.add_node(node("second-target", Point { x: 100.0, y: 100.0 }));
        input.add_edge(crate::Edge {
            source: first_source,
            target: first_target,
        });
        input.add_edge(crate::Edge {
            source: second_source,
            target: second_target,
        });
        let graph = ArenaGraph::from_input(&input);
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
            .collect();
        let first = Point { x: 40.0, y: 70.0 };
        let second = Point { x: 80.0, y: 70.0 };
        let visibility = VisibilityGraph::from_ports(
            &graph,
            &boxes,
            &graph.graph_node_order(),
            &graph.graph_node_order(),
            [
                (
                    second_source,
                    Port {
                        point: first,
                        direction: PortSide::Right,
                        side: PortSide::Right,
                        index: 0,
                        tunnel: None,
                        is_center: false,
                    },
                ),
                (
                    second_target,
                    Port {
                        point: second,
                        direction: PortSide::Left,
                        side: PortSide::Left,
                        index: 0,
                        tunnel: None,
                        is_center: false,
                    },
                ),
            ]
            .into_iter(),
        );
        let path = [
            visibility.index_of(first).unwrap(),
            visibility.index_of(second).unwrap(),
        ];
        let routes = vec![vec![first, second], Vec::new()];

        assert!(path_is_occupied_unshareably(
            &graph,
            &visibility,
            1,
            &path,
            &routes,
            None,
            None,
            None,
        ));
    }

    #[test]
    fn opposing_route_checks_terminal_port_segment() {
        // Go's Route.OVGNodes includes center nodes at both ends. Rust keeps
        // the equivalent accepted route from source port to target port, so
        // the last port segment must still participate in opposing detection.
        let route = vec![
            Point {
                x: 3890.0,
                y: 1842.0,
            },
            Point {
                x: 3728.0,
                y: 1842.0,
            },
            Point {
                x: 3728.0,
                y: 1681.0,
            },
            Point {
                x: 3919.0,
                y: 1681.0,
            },
            Point {
                x: 3919.0,
                y: 1813.0,
            },
        ];
        assert!(route_is_opposing_collinear(
            &route,
            Point {
                x: 3919.0,
                y: 1813.0
            },
            Point {
                x: 3919.0,
                y: 1736.5
            },
            true,
        ));
    }

    #[test]
    fn crossing_checker_counts_interior_collinear_overlap_but_not_shared_endpoints() {
        let routes = vec![vec![Point { x: 0.0, y: 20.0 }, Point { x: 0.0, y: 80.0 }]];
        let indexed_segments = RoutedEdgeSet::from_routes(1, &routes);

        // Recovered ovgEdgeSet.intersectsWith treats an interior collinear
        // overlap as a crossing even when the candidate is a longer segment.
        assert!(indexed_segments.crosses(
            1,
            Point { x: 0.0, y: 0.0 },
            Point { x: 0.0, y: 100.0 },
            None,
        ));

        // OVGEdge.sharePoints suppresses intersections when the two concrete
        // segments share an endpoint.
        assert!(!indexed_segments.crosses(
            1,
            Point { x: 0.0, y: 20.0 },
            Point { x: 0.0, y: 100.0 },
            None,
        ));
    }
}
