// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Direct and one-line route candidates between shape ports.
//!
//! Simple candidates are clipped against endpoint boxes and scored against
//! existing routes before the visibility-graph search is needed.

use super::route_search_guard::LocalRouteSearchWork;
use super::straight::estimated_edge_cost_against;
use super::{
    ArenaGraph, Orientation, Port, PortSide, node_rect, ports_with_table_column_count,
    segments_intersect,
};
#[cfg(test)]
use crate::internal::RoutePoints;
use crate::{NodeId, Point, Rect};
use std::collections::BTreeMap;

/// The source line kernel accepts both route-search and post-routing guards.
pub(super) trait LineWork {
    fn step(&self) -> Result<(), ()>;
    fn add(&self, units: u64) -> Result<(), ()>;
    fn check(&self) -> Result<(), ()>;
}
impl LineWork for LocalRouteSearchWork<'_> {
    fn step(&self) -> Result<(), ()> {
        LocalRouteSearchWork::step(self)
    }
    fn add(&self, units: u64) -> Result<(), ()> {
        LocalRouteSearchWork::add(self, units)
    }
    fn check(&self) -> Result<(), ()> {
        LocalRouteSearchWork::check(self)
    }
}
impl LineWork for super::route_stage_guard::RouteAtomicGuard<'_> {
    fn step(&self) -> Result<(), ()> {
        super::route_stage_guard::RouteBudget::reserve(self, 1)
    }
    fn add(&self, units: u64) -> Result<(), ()> {
        super::route_stage_guard::RouteBudget::reserve(self, units)
    }
    fn check(&self) -> Result<(), ()> {
        super::route_stage_guard::RouteBudget::reserve(self, 0)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LineRoute {
    pub(super) source: Port,
    pub(super) target: Port,
    pub(super) cost: f64,
}

fn first_box_intersection(start: Point, end: Point, rect: Rect) -> Option<Point> {
    d2_rs_core::shape_geometry::box_intersections(rect, start, end)
        .into_iter()
        .next()
}

/// The `ovgEdgeRouter.routeLine` wrapper has an evidence-backed last resort
/// that is distinct from `Graph.RouteLine`: when no legal port pair exists it
/// clips the center-to-center segment to both endpoint boxes and charges the
/// Euclidean center distance.
pub(super) fn clipped_center_fallback(
    graph: &ArenaGraph,
    edge_index: usize,
) -> Option<(Vec<Point>, f64)> {
    let edge = &graph.edge_records[edge_index];
    let source_rect = node_rect(graph, edge.from)?;
    let target_rect = node_rect(graph, edge.to)?;
    Some(clipped_center_segment(source_rect, target_rect))
}

fn clipped_center_segment(source_rect: Rect, target_rect: Rect) -> (Vec<Point>, f64) {
    let center = |rect: Rect| {
        #[cfg(target_arch = "aarch64")]
        {
            Point {
                x: rect.size.width.mul_add(0.5, rect.origin.x),
                y: rect.size.height.mul_add(0.5, rect.origin.y),
            }
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            rect.center()
        }
    };
    // Node.Center fuses width/2 + origin on the pinned Go ARM64 build.
    let source_center = center(source_rect);
    let target_center = center(target_rect);
    let source_port =
        first_box_intersection(source_center, target_center, source_rect).unwrap_or(source_center);
    let target_port =
        first_box_intersection(source_port, target_center, target_rect).unwrap_or(target_center);
    // geo.EuclideanDistance uses axis shortcuts and a sum of squares, not
    // math.Hypot's scaled ratio. Preserve the pinned ARM64 fused addition.
    let dx = source_center.x - target_center.x;
    let dy = source_center.y - target_center.y;
    let distance = if source_center.x == target_center.x {
        dy.abs()
    } else if source_center.y == target_center.y {
        dx.abs()
    } else {
        #[cfg(target_arch = "aarch64")]
        {
            dy.mul_add(dy, dx * dx).sqrt()
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            (dx * dx + dy * dy).sqrt()
        }
    };
    (vec![source_port, target_port], distance)
}

#[cfg(feature = "diagnostic-traces")]
pub(in crate::internal) fn diagnostic_box_intersections(
    value: serde_json::Value,
) -> serde_json::Value {
    use serde_json::json;
    let number = |v: &serde_json::Value| {
        f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
    };
    let point = |v: &serde_json::Value| Point {
        x: number(&v[0]),
        y: number(&v[1]),
    };
    let rect = |v: &serde_json::Value| Rect {
        origin: Point {
            x: number(&v[0]),
            y: number(&v[1]),
        },
        size: crate::Size {
            width: number(&v[2]),
            height: number(&v[3]),
        },
    };
    let bits = |p: Point| {
        [
            format!("{:016x}", p.x.to_bits()),
            format!("{:016x}", p.y.to_bits()),
        ]
    };
    json!(value.as_array().unwrap().iter().map(|row| {
        let g = &row["geometry"];let source = rect(&g["rect"]); let target = rect(&g["to_rect"]);
        let start = point(&g["segment"][0]);let end = point(&g["segment"][1]);
        let clip = clipped_center_segment(source, target);
        json!({"name":row["name"],
            "intersection":d2_rs_core::shape_geometry::line_intersection_point(start,end,point(&g["other"][0]),point(&g["other"][1])).map(bits),
            "box":d2_rs_core::shape_geometry::box_intersections(source,start,end).into_iter().map(bits).collect::<Vec<_>>(),
            "clip":clip.0.into_iter().map(bits).collect::<Vec<_>>(),"distance":format!("{:016x}",clip.1.to_bits())})
    }).collect::<Vec<_>>())
}

fn relative_orientation(first: Rect, second: Rect) -> Orientation {
    if first.bottom() < second.origin.y {
        if first.right() < second.origin.x {
            Orientation::TopLeft
        } else if second.right() < first.origin.x {
            Orientation::TopRight
        } else {
            Orientation::Top
        }
    } else if second.bottom() < first.origin.y {
        if first.right() < second.origin.x {
            Orientation::BottomLeft
        } else if second.right() < first.origin.x {
            Orientation::BottomRight
        } else {
            Orientation::Bottom
        }
    } else if second.right() < first.origin.x {
        Orientation::Right
    } else if first.right() < second.origin.x {
        Orientation::Left
    } else {
        Orientation::None
    }
}

fn ports_for_orientation(
    graph: &ArenaGraph,
    node: NodeId,
    rect: Rect,
    orientation: Orientation,
) -> Vec<Port> {
    let ordered_sides: &[PortSide] = match orientation {
        Orientation::Top => &[PortSide::Top],
        Orientation::TopRight => &[PortSide::Top, PortSide::Right],
        Orientation::Right => &[PortSide::Right],
        Orientation::BottomRight => &[PortSide::Bottom, PortSide::Right],
        Orientation::Bottom => &[PortSide::Bottom],
        Orientation::BottomLeft => &[PortSide::Bottom, PortSide::Left],
        Orientation::Left => &[PortSide::Left],
        Orientation::TopLeft => &[PortSide::Top, PortSide::Left],
        Orientation::None => &[],
    };
    let all = ports_with_table_column_count(
        rect,
        graph.nodes[node.0 as usize].shape,
        graph.nodes[node.0 as usize].table_column_count,
    );
    ordered_sides
        .iter()
        .flat_map(|side| all.iter().copied().filter(move |port| port.side == *side))
        .collect()
}

fn segment_intersects_rect(start: Point, end: Point, rect: Rect) -> bool {
    super::route_geometry::recovered_segment_intersects_box(start, end, rect)
}

fn has_sharp_angle_to_border(from: Point, to: Point, orientation: Orientation) -> bool {
    if matches!(orientation, Orientation::Top | Orientation::Bottom) {
        let mut angle = (to.y - from.y).atan2(to.x - from.x).to_degrees().abs();
        if angle > 90.0 {
            angle = 180.0 - angle;
        }
        angle < 30.0
    } else if matches!(orientation, Orientation::Left | Orientation::Right) {
        let mut angle = (to.x - from.x).atan2(to.y - from.y).to_degrees().abs();
        if angle > 90.0 {
            angle = 180.0 - angle;
        }
        angle < 30.0
    } else {
        false
    }
}

// coordinator.go compares the stored arrowhead strings even when the value
// does not draw an arrow (for example, "none" versus the empty string).
fn source_port_arrowhead(graph: &ArenaGraph, edge_index: usize, node: NodeId) -> &str {
    let edge = &graph.edge_records[edge_index];
    if edge.from == node {
        edge.source_arrowhead.as_deref().unwrap_or("")
    } else {
        edge.target_arrowhead.as_deref().unwrap_or("")
    }
}
fn target_port_arrowhead(graph: &ArenaGraph, edge_index: usize, node: NodeId) -> &str {
    let edge = &graph.edge_records[edge_index];
    if edge.to == node {
        edge.target_arrowhead.as_deref().unwrap_or("")
    } else {
        edge.source_arrowhead.as_deref().unwrap_or("")
    }
}

fn duplicate(graph: &ArenaGraph, edge_index: usize, other_index: usize) -> bool {
    let edge = &graph.edge_records[edge_index];
    let other = &graph.edge_records[other_index];
    (edge.from == other.from && edge.to == other.to)
        || (edge.from == other.to && edge.to == other.from)
}

/// Source checks duplicates first, then scans arrowheads only if no duplicate
/// exists. Preserve both separate passes and their early-return boundaries.
fn port_occupants_match(
    occupants: &[usize],
    budget: Option<&dyn LineWork>,
    mut is_duplicate: impl FnMut(usize) -> bool,
    mut arrowhead_matches: impl FnMut(usize) -> bool,
) -> Option<bool> {
    for &other in occupants {
        if let Some(budget) = budget {
            budget.step().ok()?;
        }
        if is_duplicate(other) {
            return Some(false);
        }
    }
    for &other in occupants {
        if let Some(budget) = budget {
            budget.step().ok()?;
        }
        if !arrowhead_matches(other) {
            return Some(false);
        }
    }
    Some(true)
}

/// edgeCanOverlapEdgesGuarded reserves ten full passes before calling its
/// non-interruptible predicate, then polls cancellation at the boundary.
fn overlap_eligibility_with_budget(
    edge_count: usize,
    budget: Option<&dyn LineWork>,
    eligible: impl FnOnce() -> bool,
) -> Option<bool> {
    if let Some(budget) = budget {
        for _ in 0..10 {
            budget.add(edge_count as u64).ok()?;
        }
        budget.check().ok()?;
    }
    Some(eligible())
}

fn max_side_length(rect: Rect, orientation: Orientation) -> f64 {
    match orientation {
        Orientation::Top | Orientation::Bottom => rect.size.width,
        Orientation::Left | Orientation::Right => rect.size.height,
        _ => rect.size.width.max(rect.size.height),
    }
}

fn vertical_or_horizontal_overlap(
    start: Point,
    end: Point,
    other_start: Point,
    other_end: Point,
) -> bool {
    let both_vertical = start.x == end.x && other_start.x == other_end.x;
    let both_horizontal = start.y == end.y && other_start.y == other_end.y;
    (both_vertical || both_horizontal) && segments_intersect(other_start, other_end, start, end)
}

/// RouteLine variant used by StraightEdgesFallback. The Go fallback evaluates
/// a candidate against only the routes whose segments actually overlap it;
/// ordinary route-line callers retain the recovered staged-search gate.
pub(super) fn route_line_allow_matching_overlap(
    graph: &ArenaGraph,
    edge_index: usize,
    all_edge_indices: &[usize],
    routes: Option<&[Vec<Point>]>,
) -> Option<LineRoute> {
    route_line_with_overlap_mode(
        graph,
        edge_index,
        all_edge_indices,
        routes,
        routes,
        routes.is_some(),
        true,
        None,
        None,
        None,
    )
}

/// Guarded quick-route scoring for materialized prior edge points.
// routeLine passes ordinary edge point lists to
// positionedArrowheadLabelCostGuarded, rather than OVG route nodes.
fn arrowhead_label_cost_with_budget(
    graph: &ArenaGraph,
    candidate: &crate::internal::labels::PositionedArrowheadLabel,
    nodes: &[NodeId],
    labels: &[crate::internal::labels::PositionedArrowheadLabel],
    routes: &[Vec<Point>],
    other_edges: &[usize],
    route_slots: Option<&[usize]>,
    budget: Option<&dyn LineWork>,
) -> Option<f64> {
    let step = || -> Option<()> {
        if let Some(guard) = budget {
            guard.step().ok()?;
        }
        Some(())
    };
    for other in labels {
        step()?;
        if candidate.edge_index == other.edge_index && candidate.is_target == other.is_target {
            continue;
        }
        if crate::internal::labels::overlaps(candidate.rect, other.rect, 0.0)
            && candidate.text != other.text
        {
            return Some(f64::INFINITY);
        }
    }
    let mut node_overlaps = 0;
    for &node in nodes {
        step()?;
        if node_rect(graph, node)
            .is_some_and(|rect| crate::internal::labels::overlaps(candidate.rect, rect, 5.0))
        {
            node_overlaps += 1;
        }
    }
    let mut edge_overlaps = 0;
    for (index, &other) in other_edges.iter().enumerate() {
        step()?;
        if other == candidate.edge_index && route_slots.is_none() {
            continue;
        }
        for segment in routes[route_slots.map_or(other, |slots| slots[index])].windows(2) {
            step()?;
            if crate::internal::labels::positioned_arrowhead_label_overlaps_route(
                candidate, segment,
            ) {
                edge_overlaps += 1;
                break;
            }
        }
    }
    Some(
        4.0 * graph.turn_cost_value() * node_overlaps as f64
            + graph.turn_cost_value() * edge_overlaps as f64,
    )
}

#[cfg(test)]
pub(super) fn route_line_with_budget(
    graph: &ArenaGraph,
    edge_index: usize,
    all_edge_indices: &[usize],
    routes: Option<&[Vec<Point>]>,
    budget: Option<&LocalRouteSearchWork>,
) -> Option<LineRoute> {
    route_line_with_overlap_mode(
        graph,
        edge_index,
        all_edge_indices,
        routes,
        routes,
        routes.is_some(),
        true,
        None,
        None,
        budget.map(|guard| guard as &dyn LineWork),
    )
}

/// Error fallback uses original edge points for ordinary geometry queries.
/// Published OVG routes supply only the prior positioned-label inventory.
#[cfg(test)]
pub(super) fn route_line_with_published_labels(
    graph: &ArenaGraph,
    edge_index: usize,
    all_edge_indices: &[usize],
    original_points: &[Vec<Point>],
    label_routes: &[Vec<Point>],
    budget: Option<&LocalRouteSearchWork>,
) -> Option<LineRoute> {
    route_line_with_overlap_mode(
        graph,
        edge_index,
        all_edge_indices,
        Some(original_points),
        Some(label_routes),
        false,
        true,
        None,
        None,
        budget.map(|guard| guard as &dyn LineWork),
    )
}

/// quickRoute creates one temporary Edge for each published Route. Keep its
/// point list per occurrence, even when several copies share a GEdge.
pub(super) fn route_line_with_route_records(
    graph: &ArenaGraph,
    edge_index: usize,
    records: &[(usize, Vec<Point>)],
    budget: Option<&LocalRouteSearchWork>,
) -> Option<LineRoute> {
    let edges = records.iter().map(|record| record.0).collect::<Vec<_>>();
    route_line_with_overlap_mode(
        graph,
        edge_index,
        &edges,
        None,
        None,
        true,
        true,
        Some(records),
        None,
        budget.map(|guard| guard as &dyn LineWork),
    )
}

pub(super) fn route_line_with_published_records(
    graph: &ArenaGraph,
    edge_index: usize,
    all_edge_indices: &[usize],
    original_points: &[Vec<Point>],
    records: &[super::published_route::PublishedRoute],
    budget: Option<&LocalRouteSearchWork>,
) -> Option<LineRoute> {
    debug_assert_eq!(all_edge_indices.len(), records.len());
    route_line_with_overlap_mode(
        graph,
        edge_index,
        all_edge_indices,
        Some(original_points),
        None,
        false,
        true,
        None,
        Some(records),
        budget.map(|guard| guard as &dyn LineWork),
    )
}

fn route_line_with_overlap_mode(
    graph: &ArenaGraph,
    edge_index: usize,
    all_edge_indices: &[usize],
    edge_points: Option<&[Vec<Point>]>,
    label_routes: Option<&[Vec<Point>]>,
    project_prior_styles: bool,
    allow_matching_overlap: bool,
    prior_records: Option<&[(usize, Vec<Point>)]>,
    published_records: Option<&[super::published_route::PublishedRoute]>,
    budget: Option<&dyn LineWork>,
) -> Option<LineRoute> {
    let step = || -> Option<()> {
        if let Some(budget) = budget {
            budget.step().ok()?;
        }
        Some(())
    };
    step()?;
    let edge = &graph.edge_records[edge_index];
    // quickRoute copies arrows and labels into NewEdge, but leaves Style
    // empty. Original edge metadata remains authoritative for error fallback.
    let prior_style_override = project_prior_styles.then(crate::EdgeStyle::default);
    let source_rect = node_rect(graph, edge.from)?;
    let target_rect = node_rect(graph, edge.to)?;
    let source_orientation = relative_orientation(source_rect, target_rect);
    let target_orientation = relative_orientation(target_rect, source_rect);
    if source_orientation == Orientation::None || target_orientation == Orientation::None {
        return None;
    }
    let source_ports =
        ports_for_orientation(graph, edge.from, source_rect, source_orientation.opposite());
    let target_ports =
        ports_for_orientation(graph, edge.to, target_rect, target_orientation.opposite());
    let trace_candidates =
        crate::internal::trace_env_value("TALA_TRACE_ROUTE_LINE").is_some_and(|target| {
            target == "all"
                || target
                    == format!(
                        "{}>{}",
                        graph.nodes[edge.from.0 as usize].tala_id,
                        graph.nodes[edge.to.0 as usize].tala_id
                    )
        });
    if crate::internal::trace_env_enabled("TALA_TRACE_ROUTE_LINE")
        && (trace_candidates
            || crate::internal::trace_env_value("TALA_TRACE_ROUTE_LINE").as_deref() == Some("all"))
    {
        eprintln!(
            "ROUTE_LINE_START edge={} from={}>{} source_orientation={:?} target_orientation={:?} source_ports={} target_ports={}",
            edge_index,
            graph.nodes[edge.from.0 as usize].tala_id,
            graph.nodes[edge.to.0 as usize].tala_id,
            source_orientation,
            target_orientation,
            source_ports.len(),
            target_ports.len()
        );
    }

    let route_points = |slot: usize, other_index: usize| -> std::borrow::Cow<'_, [Point]> {
        if let Some(records) = prior_records {
            return std::borrow::Cow::Borrowed(records[slot].1.as_slice());
        }
        edge_points
            .and_then(|active| active.get(other_index))
            .map(|points| std::borrow::Cow::Borrowed(points.as_slice()))
            .unwrap_or_else(|| {
                std::borrow::Cow::Owned(graph.edge_records[other_index].route.read().clone())
            })
    };

    let mut occupied = BTreeMap::<(NodeId, u64, u64), Vec<usize>>::new();
    // portEdgesGuarded performs one complete edge scan per endpoint. A
    // self-loop belongs to its source port because From is tested first.
    for node in [edge.from, edge.to] {
        for (slot, &other_index) in all_edge_indices.iter().enumerate() {
            step()?;
            let other = &graph.edge_records[other_index];
            let route = route_points(slot, other_index);
            let point = if other.from == node {
                route.first()
            } else if other.to == node {
                route.last()
            } else {
                None
            };
            if let Some(point) = point {
                occupied
                    .entry((
                        node,
                        super::go_float_map_key(point.x),
                        super::go_float_map_key(point.y),
                    ))
                    .or_default()
                    .push(other_index);
            }
        }
    }
    // filterEdge visits every stored edge before excluding the current one.
    for occupants in occupied.values_mut() {
        let mut filtered = Vec::with_capacity(occupants.len());
        for &other in occupants.iter() {
            step()?;
            if other != edge_index || prior_records.is_some() {
                filtered.push(other);
            }
        }
        *occupants = filtered;
    }
    let guarded_cluster_members = if budget.is_some() {
        Some(super::interactions::cluster_memberships_with_work(
            graph, edge.from, edge.to, budget,
        )?)
    } else {
        None
    };
    let arrowhead_nodes =
        if edge.source_arrowhead_label.is_some() || edge.target_arrowhead_label.is_some() {
            let mut nodes = Vec::new();
            for &node in &graph.node_order {
                step()?;
                if node != edge.from
                    && node != edge.to
                    && (super::shortcut::descendant(graph, Some(edge.from), node)
                        || super::shortcut::descendant(graph, Some(edge.to), node))
                {
                    continue;
                }
                nodes.push(node);
            }
            nodes
        } else {
            Vec::new()
        };
    let mut positioned_labels = Vec::new();
    for (slot, &other_index) in all_edge_indices.iter().enumerate() {
        step()?;
        if other_index == edge_index && prior_records.is_none() {
            continue;
        }
        let route = if let Some(records) = published_records {
            records[slot].segment_endpoints()?
        } else if let Some(records) = prior_records {
            records[slot].1.clone()
        } else {
            label_routes.map_or_else(
                || graph.edge_records[other_index].route.read().clone(),
                |routes| {
                    let points = routes
                        .get(other_index)
                        .filter(|points| !points.is_empty())
                        .cloned()
                        .unwrap_or_else(|| graph.edge_records[other_index].route.read().clone());
                    if budget.is_some() {
                        points
                    } else {
                        super::route_geometry::simplify_route(points)
                    }
                },
            )
        };
        if route.is_empty() {
            continue;
        }
        if let Some(budget) = budget {
            budget.add(route.len() as u64).ok()?;
        }
        for is_target in [false, true] {
            if let Some(label) = crate::internal::labels::positioned_arrowhead_label_for_route(
                other_index,
                &graph.edge_records[other_index],
                &route,
                is_target,
            ) {
                positioned_labels.push(label);
            }
        }
    }
    let mut visible_routes = vec![
        Vec::new();
        if prior_records.is_some() {
            all_edge_indices.len()
        } else {
            graph.edge_records.len()
        }
    ];
    for (slot, &other_index) in all_edge_indices.iter().enumerate() {
        visible_routes[if prior_records.is_some() {
            slot
        } else {
            other_index
        }] = route_points(slot, other_index).to_vec();
    }

    let source_arrowhead = source_port_arrowhead(graph, edge_index, edge.from);
    let target_arrowhead = target_port_arrowhead(graph, edge_index, edge.to);
    let mut best = None::<LineRoute>;
    for source in source_ports {
        step()?;
        let source_occupants = occupied
            .get(&(
                edge.from,
                super::go_float_map_key(source.point.x),
                super::go_float_map_key(source.point.y),
            ))
            .map(Vec::as_slice)
            .unwrap_or_default();
        if !port_occupants_match(
            source_occupants,
            budget,
            |other| duplicate(graph, edge_index, other),
            |other| source_port_arrowhead(graph, other, edge.from) == source_arrowhead,
        )? {
            continue;
        }
        let source_port_cost = if !source_occupants.is_empty() && edge.source_arrow {
            max_side_length(source_rect, source_orientation) / 2.0
        } else {
            0.0
        };

        for target in target_ports.iter().copied() {
            step()?;
            let target_occupants = occupied
                .get(&(
                    edge.to,
                    super::go_float_map_key(target.point.x),
                    super::go_float_map_key(target.point.y),
                ))
                .map(Vec::as_slice)
                .unwrap_or_default();
            if !port_occupants_match(
                target_occupants,
                budget,
                |other| duplicate(graph, edge_index, other),
                |other| target_port_arrowhead(graph, other, edge.to) == target_arrowhead,
            )? {
                continue;
            }
            let target_port_cost = if target_occupants.is_empty() {
                0.0
            } else {
                max_side_length(target_rect, target_orientation) * 0.5
            };

            // routeIntersectsNodeGuarded charges every node, including the
            // endpoints, before its unguarded ancestry exclusion.
            let mut intersects_node = false;
            for &node_id in &graph.node_order {
                step()?;
                if node_id == edge.from
                    || node_id == edge.to
                    || graph.is_descendant_of_scope(edge.from, Some(node_id))
                    || graph.is_descendant_of_scope(edge.to, Some(node_id))
                {
                    continue;
                }
                step()?;
                if node_rect(graph, node_id)
                    .is_some_and(|rect| segment_intersects_rect(source.point, target.point, rect))
                {
                    intersects_node = true;
                    break;
                }
            }
            if intersects_node {
                if trace_candidates {
                    eprintln!(
                        "ROUTE_LINE_REJECT_RUST source={:?} target={:?} reason=intersects_node",
                        source.point, target.point,
                    );
                }
                continue;
            }

            // filterEdge precedes overlappingEdgesGuarded in routeLineChecked.
            let mut all_other_edges = Vec::new();
            let mut all_other_slots = Vec::new();
            for (slot, &other_index) in all_edge_indices.iter().enumerate() {
                step()?;
                if other_index != edge_index || prior_records.is_some() {
                    all_other_edges.push(other_index);
                    all_other_slots.push(slot);
                }
            }
            let mut overlapping_edges = Vec::new();
            for (&slot, &other_index) in all_other_slots.iter().zip(&all_other_edges) {
                step()?;
                for segment in route_points(slot, other_index).windows(2) {
                    step()?;
                    if vertical_or_horizontal_overlap(
                        source.point,
                        target.point,
                        segment[0],
                        segment[1],
                    ) {
                        overlapping_edges.push(other_index);
                        break;
                    }
                }
            }
            let overlap_edges = if allow_matching_overlap {
                &overlapping_edges
            } else {
                // The ordinary recovered route-line path keeps the full
                // staged topology as its overlap gate. StraightEdgesFallback
                // opts into the Go candidate-only behavior above.
                &all_other_edges
            };
            if !overlapping_edges.is_empty()
                && !overlap_eligibility_with_budget(overlap_edges.len(), budget, || {
                    if let Some((source_members, target_members)) = &guarded_cluster_members {
                        super::interactions::edges_can_overlap_all_with_members_and_style(
                            graph,
                            edge_index,
                            overlap_edges,
                            (!source_members.is_empty()).then_some(source_members),
                            (!target_members.is_empty()).then_some(target_members),
                            prior_style_override.as_ref(),
                        )
                    } else {
                        super::interactions::edges_can_overlap_all_with_style(
                            graph,
                            edge_index,
                            overlap_edges,
                            prior_style_override.as_ref(),
                        )
                    }
                })?
            {
                if trace_candidates {
                    eprintln!(
                        "ROUTE_LINE_REJECT_RUST source={:?} target={:?} reason=edge_overlap overlapping={:?}",
                        source.point, target.point, overlapping_edges,
                    );
                }
                continue;
            }

            let candidate = [source.point, target.point];
            let route_cost = if let Some(budget) = budget {
                super::straight::estimated_selected_edge_cost_with_work(
                    if prior_records.is_some() {
                        usize::MAX
                    } else {
                        edge_index
                    },
                    &candidate,
                    &visible_routes,
                    if prior_records.is_some() {
                        &all_other_slots
                    } else {
                        &all_other_edges
                    },
                    budget,
                )?
            } else if prior_records.is_some() {
                struct Unbounded;
                impl LineWork for Unbounded {
                    fn step(&self) -> Result<(), ()> {
                        Ok(())
                    }
                    fn add(&self, _: u64) -> Result<(), ()> {
                        Ok(())
                    }
                    fn check(&self) -> Result<(), ()> {
                        Ok(())
                    }
                }
                super::straight::estimated_selected_edge_cost_with_work(
                    usize::MAX,
                    &candidate,
                    &visible_routes,
                    &all_other_slots,
                    &Unbounded,
                )?
            } else {
                estimated_edge_cost_against(graph, edge_index, &candidate, &visible_routes)
            };
            let mut cost = source_port_cost + target_port_cost + route_cost;
            if source.point.x != target.point.x && target.point.y != source.point.y {
                let is_axis_aligned = (source.point.x - target.point.x).abs() < 1.0
                    || (target.point.y - source.point.y).abs() < 1.0;
                if !is_axis_aligned {
                    cost *= 4.0;
                }
            }
            if has_sharp_angle_to_border(source.point, target.point, source_orientation)
                || has_sharp_angle_to_border(source.point, target.point, target_orientation)
            {
                if trace_candidates {
                    eprintln!(
                        "ROUTE_LINE_REJECT_RUST source={:?} target={:?} reason=sharp_angle source_orientation={:?} target_orientation={:?}",
                        source.point, target.point, source_orientation, target_orientation,
                    );
                }
                continue;
            }
            if !source.is_center {
                cost += graph.non_center_port_cost_value();
            }
            if !target.is_center {
                cost += graph.non_center_port_cost_value();
            }
            for is_target in [false, true] {
                let Some(label) = crate::internal::labels::positioned_arrowhead_label_for_route(
                    edge_index, edge, &candidate, is_target,
                ) else {
                    continue;
                };
                cost += arrowhead_label_cost_with_budget(
                    graph,
                    &label,
                    &arrowhead_nodes,
                    &positioned_labels,
                    &visible_routes,
                    &all_other_edges,
                    prior_records.map(|_| all_other_slots.as_slice()),
                    budget,
                )?;
            }
            if trace_candidates {
                eprintln!(
                    "ROUTE_LINE_RUST edge={} from={}>{} source={:?}:{:?}:center={} target={:?}:{:?}:center={} source_port_cost={} target_port_cost={} cost={}",
                    edge_index,
                    graph.nodes[edge.from.0 as usize].tala_id,
                    graph.nodes[edge.to.0 as usize].tala_id,
                    source.point,
                    source.side,
                    source.is_center,
                    target.point,
                    target.side,
                    target.is_center,
                    source_port_cost,
                    target_port_cost,
                    cost,
                );
            }
            if cost < best.map_or(f64::INFINITY, |best| best.cost) {
                best = Some(LineRoute {
                    source,
                    target,
                    cost,
                });
            }
        }
    }
    best
}

pub(super) fn route_line_with_stage_budget(
    graph: &ArenaGraph,
    edge_index: usize,
    all_edges: &[usize],
    guard: &super::route_stage_guard::RouteAtomicGuard<'_>,
) -> Option<LineRoute> {
    route_line_with_overlap_mode(
        graph,
        edge_index,
        all_edges,
        None,
        None,
        false,
        true,
        None,
        None,
        Some(guard),
    )
}

#[cfg(test)]
mod tests {

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn clipped_centers_preserve_subnormal_fused_rounding() {
        use crate::{Point, Rect, Size};
        let tiny = f64::from_bits(1);
        let from = Rect {
            origin: Point { x: tiny, y: tiny },
            size: Size {
                width: tiny,
                height: tiny,
            },
        };
        let to = Rect {
            origin: Point::default(),
            size: from.size,
        };
        let (points, _) = super::clipped_center_segment(from, to);
        assert_eq!(points[0].x.to_bits(), 2);
        assert_eq!(points[1].x.to_bits(), 0);
    }

    #[test]
    fn clipped_centers_follow_box_side_order_and_rounded_offsets() {
        use crate::{Point, Rect, Size};
        let rect = |x, y, width, height| Rect {
            origin: Point { x, y },
            size: Size { width, height },
        };
        let (points, distance) = super::clipped_center_segment(
            rect(0.0, 0.0, 0.125, 0.125),
            rect(0.125, 0.125, 0.125, 0.125),
        );
        assert_eq!(
            points,
            vec![
                Point {
                    x: 0.0625,
                    y: 0.0625
                };
                2
            ]
        );
        assert_eq!(distance.to_bits(), 0x3fc6a09e667f3bcd);
        // Side order takes the farther right hit before the nearer left hit.
        assert_eq!(
            super::first_box_intersection(
                Point { x: -5.75, y: 5.25 },
                Point { x: 15.25, y: 5.25 },
                rect(0.25, 0.25, 10.0, 10.0)
            ),
            Some(Point { x: 10.25, y: 5.25 })
        );
        // Closed segment endpoints participate; t=0 is not discarded.
        assert_eq!(
            super::first_box_intersection(
                Point { x: 0.25, y: 5.25 },
                Point { x: 5.25, y: 5.25 },
                rect(0.25, 0.25, 10.0, 10.0)
            ),
            Some(Point { x: 0.25, y: 5.25 })
        );
    }
    use super::*;
    use crate::Size;

    #[test]
    fn overlap_reservation_finishes_before_the_predicate_runs() {
        let work = LocalRouteSearchWork::new(20);
        assert_eq!(
            overlap_eligibility_with_budget(2, Some(&work), || true),
            Some(true)
        );
        assert_eq!(work.used(), 20);
        for limit in 0..20 {
            let work = LocalRouteSearchWork::new(limit);
            assert_eq!(
                overlap_eligibility_with_budget(2, Some(&work), || panic!(
                    "rejected reservation must not execute predicate"
                )),
                None
            );
            assert_eq!(work.used(), limit / 2 * 2);
        }
        assert_eq!(
            overlap_eligibility_with_budget(2, None, || false),
            Some(false)
        );
    }

    #[test]
    fn shared_port_checks_preserve_pass_order_and_exhaustion() {
        use std::cell::RefCell;
        let visits = RefCell::new(Vec::new());
        let work = LocalRouteSearchWork::new(6);
        assert_eq!(
            port_occupants_match(
                &[4, 5, 6],
                Some(&work),
                |other| {
                    visits.borrow_mut().push(("duplicate", other));
                    false
                },
                |other| {
                    visits.borrow_mut().push(("arrowhead", other));
                    other != 5
                },
            ),
            Some(false)
        );
        assert_eq!(work.used(), 5);
        assert_eq!(
            *visits.borrow(),
            vec![
                ("duplicate", 4),
                ("duplicate", 5),
                ("duplicate", 6),
                ("arrowhead", 4),
                ("arrowhead", 5)
            ]
        );
        let work = LocalRouteSearchWork::new(1);
        assert_eq!(
            port_occupants_match(
                &[4, 5],
                Some(&work),
                |other| other == 4,
                |_| panic!("duplicate must skip arrowhead pass")
            ),
            Some(false)
        );
        assert_eq!(work.used(), 1);
        for limit in 0..4 {
            let work = LocalRouteSearchWork::new(limit);
            assert_eq!(
                port_occupants_match(&[4, 5], Some(&work), |_| false, |_| true),
                None
            );
            assert_eq!(work.used(), limit);
        }
        let work = LocalRouteSearchWork::new(0);
        assert_eq!(
            port_occupants_match(&[], Some(&work), |_| unreachable!(), |_| unreachable!()),
            Some(true)
        );
        assert_eq!(work.used(), 0);
    }

    #[test]
    fn fallback_keeps_original_edge_points_separate_from_published_label_routes() {
        let mut input = crate::Graph::default();
        let size = Size {
            width: 20.0,
            height: 20.0,
        };
        let source = input.add_node(crate::Node::new("source", size));
        let target = input.add_node(crate::Node::new("target", size));
        let prior_source = input.add_node(crate::Node::new("prior_source", size));
        let prior_target = input.add_node(crate::Node::new("prior_target", size));
        input.add_edge(crate::Edge { source, target });
        input.add_edge(crate::Edge {
            source: prior_source,
            target: prior_target,
        });
        let mut graph = ArenaGraph::from_input(&input);
        for (node, x, y) in [
            (source, 0.0, 0.0),
            (target, 100.0, 0.0),
            (prior_source, 40.0, -100.0),
            (prior_target, 40.0, 100.0),
        ] {
            graph.set_position(node, Point { x, y });
        }
        let published = vec![
            vec![],
            vec![Point { x: 50.0, y: -80.0 }, Point { x: 50.0, y: 100.0 }],
        ];
        let original_points = graph
            .edge_records
            .iter()
            .map(|edge| edge.route.read().clone())
            .collect::<Vec<_>>();
        let baseline = route_line_with_budget(&graph, 0, &[1], None, None).unwrap();
        let label_work = LocalRouteSearchWork::new(100_000);
        let fallback = route_line_with_published_labels(
            &graph,
            0,
            &[1],
            &original_points,
            &published,
            Some(&label_work),
        )
        .unwrap();
        assert_eq!(fallback.source.point, baseline.source.point);
        assert_eq!(fallback.target.point, baseline.target.point);
        assert_eq!(fallback.cost, baseline.cost);
        // QuickRoute explicitly gives its temporary edges these points, so
        // that path must still account for the crossing.
        let materialized = route_line_with_budget(&graph, 0, &[1], Some(&published), None).unwrap();
        assert!(materialized.cost > fallback.cost);
        // Pre-routed loops have committed point geometry in Go. The
        // ordinary view may contain it independently of label geometry.
        let pre_routed =
            route_line_with_published_labels(&graph, 0, &[1], &published, &published, None)
                .unwrap();
        assert_eq!(pre_routed.cost, materialized.cost);
        let exact = LocalRouteSearchWork::new(label_work.used());
        assert!(
            route_line_with_published_labels(
                &graph,
                0,
                &[1],
                &original_points,
                &published,
                Some(&exact)
            )
            .is_some()
        );
        for limit in 0..label_work.used() {
            let limited = LocalRouteSearchWork::new(limit);
            assert!(
                route_line_with_published_labels(
                    &graph,
                    0,
                    &[1],
                    &original_points,
                    &published,
                    Some(&limited)
                )
                .is_none()
            );
            assert!(limited.exhausted());
        }
    }

    #[test]
    fn line_with_only_infinite_label_candidates_is_invalid() {
        let mut input = crate::Graph::default();
        let source = input.add_node(crate::Node::new(
            "source",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let target = input.add_node(crate::Node::new(
            "target",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let prior = input.add_edge(crate::Edge { source, target });
        let current = input.add_edge(crate::Edge { source, target });
        for (edge, text) in [(prior, "prior"), (current, "current")] {
            input.set_edge_arrowhead_labels(
                edge,
                crate::EdgeArrowheadLabels {
                    source: Some(crate::ArrowheadLabel {
                        text: text.into(),
                        size: Size {
                            width: 1000.0,
                            height: 1000.0,
                        },
                        color: None,
                    }),
                    target: None,
                },
            );
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 130.0, y: 0.0 });
        graph.edge_records[prior.0 as usize]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: 100.0, y: 50.0 },
                Point { x: 130.0, y: 50.0 },
            ]));
        let budget = LocalRouteSearchWork::new(100_000);
        assert!(
            route_line_with_budget(
                &graph,
                current.0 as usize,
                &[prior.0 as usize],
                None,
                Some(&budget)
            )
            .is_none()
        );
        assert!(!budget.exhausted());
    }

    #[test]
    fn guarded_line_arrowhead_cost_charges_visits_and_rejects_partial_work() {
        let mut input = crate::Graph::default();
        let source = input.add_node(crate::Node::new(
            "source",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let target = input.add_node(crate::Node::new(
            "target",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        input.add_edge(crate::Edge { source, target });
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 100.0, y: 0.0 });
        graph.set_turn_cost(7.0);
        let candidate = crate::internal::labels::PositionedArrowheadLabel {
            rect: Rect {
                origin: Point { x: 21.0, y: 9.0 },
                size: Size {
                    width: 2.0,
                    height: 2.0,
                },
            },
            edge_index: 0,
            is_target: false,
            text: "port".into(),
        };
        let routes = vec![
            vec![],
            vec![
                Point { x: 0.0, y: 10.0 },
                Point { x: 50.0, y: 10.0 },
                Point { x: 100.0, y: 10.0 },
            ],
        ];
        let score = |guard: Option<&LocalRouteSearchWork>| {
            arrowhead_label_cost_with_budget(
                &graph,
                &candidate,
                &[source, target],
                std::slice::from_ref(&candidate),
                &routes,
                &[0, 1],
                None,
                guard.map(|work| work as &dyn LineWork),
            )
        };
        let exact = LocalRouteSearchWork::new(6);
        assert_eq!(score(Some(&exact)), Some(35.0));
        assert_eq!(exact.used(), 6);
        assert_eq!(score(None), Some(35.0));
        for limit in 0..6 {
            let guard = LocalRouteSearchWork::new(limit);
            assert_eq!(score(Some(&guard)), None);
            assert!(guard.exhausted());
        }
        let conflict = crate::internal::labels::PositionedArrowheadLabel {
            edge_index: 1,
            text: "other".into(),
            ..candidate.clone()
        };
        let early = LocalRouteSearchWork::new(1);
        assert_eq!(
            arrowhead_label_cost_with_budget(
                &graph,
                &candidate,
                &[source, target],
                &[conflict],
                &routes,
                &[1],
                None,
                Some(&early)
            ),
            Some(f64::INFINITY)
        );
        assert_eq!(early.used(), 1);
    }

    #[test]
    fn straight_route_ignores_storage_nodes_outside_the_component() {
        let mut input = crate::Graph::default();
        let source = input.add_node(crate::Node::new(
            "source",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let target = input.add_node(crate::Node::new(
            "target",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let unrelated = input.add_node(crate::Node::new(
            "unrelated",
            Size {
                width: 30.0,
                height: 200.0,
            },
        ));
        input.add_edge(crate::Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(source, Point { x: 0.0, y: 0.0 });
        graph.set_position(target, Point { x: 100.0, y: 0.0 });
        graph.set_position(unrelated, Point { x: 40.0, y: -100.0 });
        assert!(route_line_with_budget(&graph, 0, &[0], None, None).is_none());
        graph.node_order = vec![source, target];
        let work = LocalRouteSearchWork::new(100_000);
        let route = route_line_with_budget(&graph, 0, &[0], None, Some(&work))
            .expect("unrelated component cannot block a straight route");
        let used = work.used();
        graph.set_position(
            unrelated,
            Point {
                x: 1000.0,
                y: 1000.0,
            },
        );
        let far_work = LocalRouteSearchWork::new(100_000);
        let far = route_line_with_budget(&graph, 0, &[0], None, Some(&far_work)).unwrap();
        assert_eq!(route.source.point, far.source.point);
        assert_eq!(route.target.point, far.target.point);
        assert_eq!(route.cost, far.cost);
        assert_eq!(used, far_work.used());
    }

    #[test]
    fn line_blocker_detects_a_thin_positive_interval_without_corner_rejection() {
        let start = Point { x: 0.0, y: 0.0 };
        let end = Point { x: 100.0, y: 100.0 };
        assert!(segment_intersects_rect(
            start,
            end,
            Rect {
                origin: Point { x: 50.0, y: 50.0 },
                size: Size {
                    width: 0.1,
                    height: 0.1
                },
            }
        ));
        assert!(!segment_intersects_rect(
            start,
            end,
            Rect {
                origin: Point { x: 50.0, y: 49.0 },
                size: Size {
                    width: 1.0,
                    height: 1.0
                },
            }
        ));
    }

    #[test]
    fn segment_box_test_detects_a_short_bottom_left_crossing() {
        let rect = Rect {
            origin: Point {
                x: 3453.0,
                y: 916.0,
            },
            size: Size {
                width: 117.0,
                height: 76.0,
            },
        };

        // The pinned Go predicate detects a positive clipped interval here;
        // the former perturbed-endpoint implementation incorrectly missed it.
        assert!(segment_intersects_rect(
            Point {
                x: 4299.0,
                y: 1484.0,
            },
            Point {
                x: 3170.0,
                y: 827.0,
            },
            rect,
        ));
        assert!(segment_intersects_rect(
            Point {
                x: 3500.0,
                y: 1100.0,
            },
            Point {
                x: 3500.0,
                y: 800.0,
            },
            rect,
        ));
    }
}
