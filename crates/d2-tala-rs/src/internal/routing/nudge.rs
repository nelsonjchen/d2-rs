// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! nudge_channels.go over actual graph headers and shared geo.Point allocations.
use super::channel_points::go_hypot;
use super::route_search_guard::{MAX_ROUTE_STAGE_WORK_UNITS, RouteStageWork};
use super::route_stage_guard::{
    self, RouteAtomicGuard, RouteMutationSnapshot, RouteStageError, RuntimeRouteContext,
};
use super::{channel_candidates as candidates, channel_safety as safety, channel_solver as solver};
use crate::internal::owner_pointer_slices::SharedPointSlice;
use crate::internal::owner_points::SharedOwnerPoint;
use crate::internal::{ArenaGraph, OwnerGraphId, RoutePoints, limits, quality};
use crate::{EdgeId, NodeId, Point, ShapeKind};
use solver::{maximum, minimum};
use std::collections::{BTreeMap, BTreeSet};
const EPSILON: f64 = 1e-6;
const SPACING: f64 = 40.0;
type Result<T> = std::result::Result<T, RouteStageError>;
pub(super) fn nodes(graph: &ArenaGraph) -> impl Iterator<Item = NodeId> + use<> {
    graph
        .owner_graph_view(OwnerGraphId(0))
        .unwrap()
        .nodes
        .require_nodes()
}
pub(super) fn edges(graph: &ArenaGraph) -> impl Iterator<Item = EdgeId> + use<> {
    graph.edge_order.header().iter_slots().map(Option::unwrap)
}
pub(super) fn coordinate(point: Point, horizontal: bool) -> f64 {
    if horizontal { point.x } else { point.y }
}
#[derive(Clone)]
pub(super) struct Segment {
    pub(super) start: SharedOwnerPoint,
    pub(super) end: SharedOwnerPoint,
    pub(super) owner: Option<EdgeId>,
}
impl Segment {
    pub(super) fn position(&self, horizontal: bool) -> f64 {
        coordinate(self.start.get(), horizontal)
    }
    pub(super) fn overlaps(&self, other: &Self, horizontal: bool, buffer: f64) -> bool {
        let (a, b, c, d) = (
            coordinate(self.start.get(), !horizontal),
            coordinate(self.end.get(), !horizontal),
            coordinate(other.start.get(), !horizontal),
            coordinate(other.end.get(), !horizontal),
        );
        !(minimum(a, b) - maximum(c, d) >= buffer || minimum(c, d) - maximum(a, b) >= buffer)
    }
}
struct Group {
    segments: Vec<Segment>,
    position: f64,
    lower: f64,
    upper: f64,
    fixed: bool,
    clearance: Option<(f64, f64)>,
}
struct Problem {
    groups: Vec<Group>,
    solver: solver::Problem,
}
fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i]
    }
    i
}
pub(super) fn bounds(
    segment: &Segment,
    walls: &[Segment],
    guard: &RouteAtomicGuard<'_>,
) -> Result<(f64, f64)> {
    let (mut lower, mut upper) = (f64::NEG_INFINITY, f64::INFINITY);
    if segment.start.get() == segment.end.get() {
        guard.step()?;
        return Ok((lower, upper));
    }
    let horizontal = segment.start.get().x == segment.end.get().x;
    for wall in walls {
        guard.step()?;
        if coordinate(wall.end.get(), !horizontal)
            < coordinate(segment.start.get(), !horizontal) - SPACING
        {
            continue;
        }
        if coordinate(wall.start.get(), !horizontal)
            > coordinate(segment.end.get(), !horizontal) + SPACING
        {
            continue;
        }
        let value = wall.position(horizontal);
        let current = segment.position(horizontal);
        if value <= current {
            lower = maximum(lower, value)
        }
        if value > current {
            upper = minimum(upper, value)
        }
    }
    Ok((lower, upper))
}
fn sort_groups(order: &mut [usize], groups: &[Group], guard: &RouteAtomicGuard<'_>) -> Result<()> {
    if order.len() < 2 {
        return guard.step();
    }
    let mut temporary = vec![0; order.len()];
    let mut width = 1;
    while width < order.len() {
        for start in (0..order.len()).step_by(width * 2) {
            let middle = (start + width).min(order.len());
            let end = (start + width * 2).min(order.len());
            let (mut left, mut right, mut out) = (start, middle, start);
            while left < middle && right < end {
                guard.step()?;
                if groups[order[right]].position < groups[order[left]].position {
                    temporary[out] = order[right];
                    right += 1
                } else {
                    temporary[out] = order[left];
                    left += 1
                };
                out += 1;
            }
            while left < middle {
                guard.step()?;
                temporary[out] = order[left];
                left += 1;
                out += 1
            }
            while right < end {
                guard.step()?;
                temporary[out] = order[right];
                right += 1;
                out += 1
            }
        }
        guard.add(order.len() as u64)?;
        order.copy_from_slice(&temporary);
        if width > order.len() / 2 {
            break;
        }
        width *= 2;
    }
    Ok(())
}
fn build(graph: &ArenaGraph, horizontal: bool, guard: &RouteAtomicGuard<'_>) -> Result<Problem> {
    let mut segments = Vec::new();
    // edgeSegmentsGuarded is a separate traversal before classification.
    for edge in edges(graph) {
        guard.step()?;
        let points = &graph.edge_records[edge.0 as usize].route;
        for i in 1..points.len() {
            guard.step()?;
            let a = points.slot(i - 1).unwrap();
            let b = points.slot(i).unwrap();
            if coordinate(a.get(), horizontal) != coordinate(b.get(), horizontal) {
                continue;
            }
            let (start, end) =
                if coordinate(a.get(), !horizontal) < coordinate(b.get(), !horizontal) {
                    (a, b)
                } else {
                    (b, a)
                };
            segments.push(Segment {
                start,
                end,
                owner: Some(edge),
            });
        }
    }
    let mut fixed_points = BTreeSet::new();
    for node in nodes(graph) {
        if let Some(point) = graph.owner_node_point(node).unwrap().shared_point() {
            fixed_points.insert(point.allocation_key());
        }
    }
    let mut locked = BTreeSet::new();
    for edge in edges(graph) {
        guard.step()?;
        let e = &graph.edge_records[edge.0 as usize];
        let p = &e.route;
        if p.len() < 2 {
            continue;
        }
        fixed_points.insert(p.slot(0).unwrap().allocation_key());
        fixed_points.insert(p.slot(p.len() - 1).unwrap().allocation_key());
        let tree = graph
            .owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(e.from))
            .unwrap()
            .is_some()
            || graph
                .owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(e.to))
                .unwrap()
                .is_some();
        let ports = e.has_table_column()
            || graph.nodes[e.from.0 as usize].shape == ShapeKind::Diamond
            || graph.nodes[e.to.0 as usize].shape == ShapeKind::Diamond;
        let a = p.point(0);
        let b = p.point(p.len() - 1);
        if e.is_curve
            || e.from == e.to
            || tree
            || (!ports && ((a.x - b.x).abs() == 1.0 || (a.y - b.y).abs() == 1.0))
        {
            locked.insert(edge);
        }
        for i in 1..p.len() {
            guard.step()?;
            let a = p.point(i - 1);
            let b = p.point(i);
            if (a.x != b.x && a.y != b.y) || a == b {
                locked.insert(edge);
            }
        }
    }
    let mut parent: Vec<_> = (0..segments.len()).collect();
    for (i, s) in segments.iter().enumerate() {
        for (j, other) in segments[..i].iter().enumerate() {
            guard.step()?;
            if s.position(horizontal) == other.position(horizontal)
                && s.overlaps(other, horizontal, SPACING)
            {
                let a = root(&mut parent, i);
                let b = root(&mut parent, j);
                parent[a] = b;
            }
        }
    }
    let mut by_root = BTreeMap::new();
    let mut groups: Vec<Group> = Vec::new();
    for (i, s) in segments.iter().enumerate() {
        guard.step()?;
        let id = root(&mut parent, i);
        let index = *by_root.entry(id).or_insert_with(|| {
            let index = groups.len();
            groups.push(Group {
                segments: Vec::new(),
                position: s.position(horizontal),
                lower: 0.,
                upper: 0.,
                fixed: false,
                clearance: None,
            });
            index
        });
        let group = &mut groups[index];
        group.segments.push(s.clone());
        group.fixed = group.fixed
            || locked.contains(&s.owner.unwrap())
            || fixed_points.contains(&s.start.allocation_key())
            || fixed_points.contains(&s.end.allocation_key());
        for node in nodes(graph) {
            guard.step()?;
            let n = &graph.nodes[node.0 as usize];
            if n.rect.size.width != 1.0 || n.rect.size.height != 1.0 {
                continue;
            }
            for p in [&s.start, &s.end] {
                let p = p.get();
                let origin = graph.position(node).unwrap();
                if (p.x == origin.x || p.x == origin.x + 1.0)
                    && (p.y == origin.y || p.y == origin.y + 1.0)
                {
                    group.fixed = true
                }
            }
        }
    }
    let mut walls = Vec::new();
    for node in nodes(graph) {
        guard.step()?;
        let n = &graph.nodes[node.0 as usize];
        let first = graph
            .owner_node_point(node)
            .unwrap()
            .shared_point()
            .unwrap();
        let p = first.get();
        let (w, h) = (n.rect.size.width, n.rect.size.height);
        let (end, a, b) = if horizontal {
            (
                Point { x: p.x, y: p.y + h },
                Point { x: p.x + w, y: p.y },
                Point {
                    x: p.x + w,
                    y: p.y + h,
                },
            )
        } else {
            (
                Point { x: p.x + w, y: p.y },
                Point { x: p.x, y: p.y + h },
                Point {
                    x: p.x + w,
                    y: p.y + h,
                },
            )
        };
        walls.push(Segment {
            start: first,
            end: SharedOwnerPoint::new(end),
            owner: None,
        });
        walls.push(Segment {
            start: SharedOwnerPoint::new(a),
            end: SharedOwnerPoint::new(b),
            owner: None,
        });
    }
    for group in &mut groups {
        if group.fixed {
            continue;
        }
        let (mut lower, mut upper) = (f64::NEG_INFINITY, f64::INFINITY);
        for s in &group.segments {
            let (lo, hi) = bounds(s, &walls, guard)?;
            if lo != f64::NEG_INFINITY {
                lower = maximum(lower, lo + minimum(group.position - lo, SPACING))
            }
            if hi != f64::INFINITY {
                upper = minimum(upper, hi - minimum(hi - group.position, SPACING))
            }
        }
        group.clearance = Some((lower, upper));
    }
    for group in &groups {
        if group.fixed {
            walls.extend(group.segments.iter().cloned())
        }
    }
    for _ in 0..2 {
        for group in &mut groups {
            if group.fixed {
                continue;
            }
            group.lower = f64::NEG_INFINITY;
            group.upper = f64::INFINITY;
            for s in &group.segments {
                let (lo, hi) = bounds(s, &walls, guard)?;
                group.lower = maximum(group.lower, lo);
                group.upper = minimum(group.upper, hi)
            }
            if group.lower.is_infinite()
                || group.upper.is_infinite()
                || group.lower > group.position
                || group.upper < group.position
                || group.lower >= group.upper
            {
                group.fixed = true;
                walls.extend(group.segments.iter().cloned())
            }
        }
    }
    let mut order: Vec<_> = groups
        .iter()
        .enumerate()
        .filter_map(|(i, g)| (!g.fixed).then_some(i))
        .collect();
    sort_groups(&mut order, &groups, guard)?;
    let mut arcs = Vec::new();
    for (i, &a) in order.iter().enumerate() {
        for (j, &b) in order.iter().enumerate().skip(i + 1) {
            let (a, b) = (&groups[a], &groups[b]);
            let owner = a.segments[0].owner;
            let same = a
                .segments
                .iter()
                .chain(&b.segments)
                .all(|s| s.owner == owner);
            let buffer = if same { 0. } else { SPACING };
            let mut linked = false;
            for s in &a.segments {
                for t in &b.segments {
                    guard.step()?;
                    linked = linked || s.overlaps(t, horizontal, buffer)
                }
            }
            if linked {
                arcs.push(solver::Arc {
                    from: i,
                    to: j,
                    separate: !same,
                })
            }
        }
    }
    let mut groups: Vec<Option<Group>> = groups.into_iter().map(Some).collect();
    let groups: Vec<_> = order
        .into_iter()
        .map(|i| groups[i].take().unwrap())
        .collect();
    let values = groups
        .iter()
        .map(|g| solver::Group {
            lower: g.lower,
            upper: g.upper,
            node_clearance: g.clearance,
        })
        .collect();
    Ok(Problem {
        groups,
        solver: solver::Problem {
            groups: values,
            arcs,
        },
    })
}
// Keys identify actual allocations. Coordinate-equal, independently allocated
// points are never merged. Commit order itself is unspecified by Go's map.
type Proposal = BTreeMap<usize, (SharedOwnerPoint, SharedOwnerPoint)>;
struct Evaluation<'a, 'g> {
    graph: &'a ArenaGraph,
    groups: Vec<&'a Group>,
    horizontal: bool,
    guard: &'a RouteAtomicGuard<'g>,
}
impl candidates::Evaluator for Evaluation<'_, '_> {
    type Error = RouteStageError;
    type Proposal = Proposal;
    fn reserve(&mut self, units: u64) -> Result<()> {
        self.guard.add(units)
    }
    fn proposal(&mut self, positions: &[f64]) -> Result<Option<Proposal>> {
        let mut proposal = Proposal::new();
        for (i, group) in self.groups.iter().enumerate() {
            for s in &group.segments {
                for point in [&s.start, &s.end] {
                    self.guard.step()?;
                    let mut candidate = point.get();
                    if self.horizontal {
                        candidate.x = positions[i]
                    } else {
                        candidate.y = positions[i]
                    }
                    let key = point.allocation_key();
                    if proposal
                        .get(&key)
                        .is_some_and(|(_, old)| old.get() != candidate)
                    {
                        return Ok(None);
                    }
                    proposal.insert(key, (point.clone(), SharedOwnerPoint::new(candidate)));
                }
            }
        }
        Ok(Some(proposal))
    }
    fn wire_length(&mut self, proposal: Option<&Proposal>) -> Result<f64> {
        let mut length = 0.;
        for edge in edges(self.graph) {
            let route = &self.graph.edge_records[edge.0 as usize].route;
            for i in 1..route.len() {
                self.guard.step()?;
                let get = |i| {
                    let p = route.slot(i).unwrap();
                    proposal
                        .and_then(|q| q.get(&p.allocation_key()))
                        .map_or_else(|| p.get(), |(_, q)| q.get())
                };
                let a = get(i - 1);
                let b = get(i);
                length += go_hypot(b.x - a.x, b.y - a.y);
            }
        }
        Ok(length)
    }
    fn safe(&mut self, proposal: &Proposal) -> Result<bool> {
        proposal_safe(self.graph, proposal, self.guard)
    }
}
fn proposal_safe(
    graph: &ArenaGraph,
    proposal: &Proposal,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool> {
    let order: Vec<_> = edges(graph).collect();
    let mut candidates = Vec::new();
    let mut changed = vec![false; order.len()];
    for (index, &edge) in order.iter().enumerate() {
        let e = &graph.edge_records[edge.0 as usize];
        let candidate = SharedPointSlice::with_len_cap(e.route.len(), e.route.len()).unwrap();
        for i in 0..e.route.len() {
            candidate.write_capacity_slot(i, e.route.slot(i)).unwrap();
        }
        for i in 0..e.route.len() {
            let point = e.route.slot(i).unwrap();
            guard.step()?;
            if let Some((_, p)) = proposal.get(&point.allocation_key()) {
                candidate.write_capacity_slot(i, Some(p.clone())).unwrap();
                changed[index] = changed[index] || p.get() != point.get();
            }
        }
        let candidate = RoutePoints::from_header(candidate);
        candidates.push(candidate);
        if !changed[index] {
            continue;
        }
        if e.is_curve {
            return Ok(false);
        }
        let candidate = &candidates[index];
        if !super::shortcut::changed_route_clear(graph, edge, candidate, guard)? {
            return Ok(false);
        }
        let (mut before_length, mut after_length) = (0., 0.);
        let mut nonzero: Vec<SharedOwnerPoint> = Vec::new();
        for i in 0..candidate.len() {
            let point = candidate.slot(i).unwrap();
            guard.step()?;
            let value = point.get();
            if !value.x.is_finite() || !value.y.is_finite() {
                return Ok(false);
            }
            if i > 0 {
                let old = e.route.point(i - 1);
                let previous = candidate.point(i - 1);
                let original = e.route.point(i);
                let before = go_hypot(original.x - old.x, original.y - old.y);
                let after = go_hypot(value.x - previous.x, value.y - previous.y);
                before_length += before;
                after_length += after;
                if after > EPSILON && after + EPSILON < minimum(before, SPACING) {
                    return Ok(false);
                }
            }
            if nonzero.last().is_none_or(|p| p.get() != value) {
                nonzero.push(point)
            }
        }
        if after_length > before_length + EPSILON {
            return Ok(false);
        }
        for i in 2..nonzero.len() {
            let (a, b, c) = (nonzero[i - 2].get(), nonzero[i - 1].get(), nonzero[i].get());
            if (b.x - a.x) * (c.x - b.x) + (b.y - a.y) * (c.y - b.y) < 0. {
                return Ok(false);
            }
        }
        for i in 0..nonzero.len().saturating_sub(1) {
            for j in i + 2..nonzero.len().saturating_sub(1) {
                guard.step()?;
                if safety::contact(
                    nonzero[i].get(),
                    nonzero[i + 1].get(),
                    nonzero[j].get(),
                    nonzero[j + 1].get(),
                ) {
                    return Ok(false);
                }
            }
        }
    }
    for (i, &edge) in order.iter().enumerate() {
        for (j, &other) in order.iter().enumerate().skip(i + 1) {
            if !changed[i] && !changed[j] {
                continue;
            }
            let (e, f) = (
                &graph.edge_records[edge.0 as usize],
                &graph.edge_records[other.0 as usize],
            );
            if e.is_curve || f.is_curve {
                return Ok(false);
            }
            let (mut before, mut after) = (0, 0);
            for a in 0..e.route.len().saturating_sub(1) {
                for b in 0..f.route.len().saturating_sub(1) {
                    guard.step()?;
                    let old = [
                        e.route.point(a),
                        e.route.point(a + 1),
                        f.route.point(b),
                        f.route.point(b + 1),
                    ];
                    let new = [
                        candidates[i].point(a),
                        candidates[i].point(a + 1),
                        candidates[j].point(b),
                        candidates[j].point(b + 1),
                    ];
                    before += usize::from(safety::crossing(old[0], old[1], old[2], old[3]));
                    after += usize::from(safety::crossing(new[0], new[1], new[2], new[3]));
                    if safety::overlap(old[0], old[1], old[2], old[3]) == 0.
                        && safety::overlap(new[0], new[1], new[2], new[3]) > 0.
                    {
                        return Ok(false);
                    }
                    if !safety::contact(old[0], old[1], old[2], old[3])
                        && safety::contact(new[0], new[1], new[2], new[3])
                    {
                        return Ok(false);
                    }
                }
            }
            if after > before {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
pub(super) fn remove_duplicates(
    points: &RoutePoints,
    guard: &RouteAtomicGuard<'_>,
) -> Result<RoutePoints> {
    if points.len() <= 1 {
        guard.step()?;
        return Ok(points.clone());
    }
    let mut result = SharedPointSlice::with_len_cap(0, points.len()).unwrap();
    result = result.append(&[points.slot(0)]).unwrap();
    for i in 1..points.len() {
        guard.step()?;
        if points.point(i - 1) != points.point(i) {
            result = result.append(&[points.slot(i)]).unwrap();
        }
    }
    if result.len() == 1 {
        let mut point = result.get(0).flatten().unwrap().get();
        point.x += 1.;
        result = result
            .append(&[Some(SharedOwnerPoint::new(point))])
            .unwrap();
    }
    Ok(RoutePoints::from_header(result))
}
fn run(graph: &mut ArenaGraph, guard: &RouteAtomicGuard<'_>) -> Result<()> {
    for node in nodes(graph) {
        graph.materialize_owner_node_point(node).unwrap();
    }
    for horizontal in [true, false] {
        let problem = build(graph, horizontal, guard)?;
        if problem.groups.is_empty() {
            continue;
        }
        let snapshot = RouteMutationSnapshot::capture(graph, OwnerGraphId(0), &[], guard)?;
        if !snapshot.inventory_is_closed(graph, guard)? {
            return Ok(());
        }
        let mut before_points = BTreeMap::new();
        for edge in edges(graph) {
            let route = &graph.edge_records[edge.0 as usize].route;
            guard.add(route.len() as u64 + 1)?;
            before_points.insert(
                edge,
                (0..route.len()).map(|i| route.point(i)).collect::<Vec<_>>(),
            );
        }
        let before = quality::inspect_owner(graph, OwnerGraphId(0), guard.context())?;
        let mut changed = false;
        for component in candidates::components(&problem.solver) {
            let positions: Vec<_> = component
                .indices
                .iter()
                .map(|&i| problem.groups[i].position)
                .collect();
            let mut evaluation = Evaluation {
                graph,
                groups: component
                    .indices
                    .iter()
                    .map(|&i| &problem.groups[i])
                    .collect(),
                horizontal,
                guard,
            };
            if let Some(proposal) =
                candidates::select(&component.problem, &positions, &mut evaluation)?
            {
                for (_, (original, candidate)) in proposal {
                    guard.step()?;
                    original.set(candidate.get());
                }
                changed = true;
            }
        }
        if !changed {
            continue;
        }
        let after = quality::inspect_owner(graph, OwnerGraphId(0), guard.context())?;
        if after.route_obstructions > before.route_obstructions
            || after.crossings > before.crossings
            || after.text_occlusions > before.text_occlusions
            || after.route_length > before.route_length + EPSILON
        {
            snapshot.restore(graph);
            continue;
        }
        for edge in edges(graph) {
            let route = &graph.edge_records[edge.0 as usize].route;
            let before = &before_points[&edge];
            if (0..route.len()).any(|i| route.point(i) != before[i]) {
                graph.edge_records[edge.0 as usize]
                    .route
                    .replace(remove_duplicates(route, guard)?);
            }
        }
    }
    Ok(())
}
pub(super) fn nudge_with_context(
    graph: &mut ArenaGraph,
    context: Option<&dyn limits::Context>,
    budget: &RouteStageWork,
) -> Result<()> {
    if let Some(context) = context {
        if let Some(cause) = context.error() {
            return Err(limits::Error::Context {
                location: String::new(),
                cause,
            }
            .into());
        }
        if super::shortcut::input_too_large(graph) {
            return context.error().map_or(Ok(()), |cause| {
                Err(limits::Error::Context {
                    location: String::new(),
                    cause,
                }
                .into())
            });
        }
    }
    route_stage_guard::run_atomic_route_stage_typed(
        graph,
        Some(OwnerGraphId(0)),
        context,
        "NudgeChannels",
        &[],
        budget,
        run,
    )
}
pub(in crate::internal) fn nudge_edge_channels_with_budget(
    graph: &mut ArenaGraph,
    budget: &RouteStageWork,
) -> Result<()> {
    nudge_with_context(graph, Some(&RuntimeRouteContext), budget)
}
pub(in crate::internal) fn nudge_edge_channels(graph: &mut ArenaGraph) {
    let _ =
        nudge_edge_channels_with_budget(graph, &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, Size};
    fn fixture() -> ArenaGraph {
        let mut input = Graph::default();
        let a = input.add_node(Node::new(
            "a",
            Size {
                width: 100.,
                height: 100.,
            },
        ));
        let b = input.add_node(Node::new(
            "b",
            Size {
                width: 100.,
                height: 100.,
            },
        ));
        input.add_edge(Edge {
            source: a,
            target: b,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(a, Point { x: 0., y: 0. });
        graph.set_position(b, Point { x: 300., y: 200. });
        graph.edge_records[0].route.replace(RoutePoints::make(
            &[(100., 50.), (120., 50.), (120., 250.), (300., 250.)].map(|(x, y)| Point { x, y }),
            4,
        ));
        graph
    }
    #[test]
    fn channel_moves_live_points_and_retains_the_original_route_header() {
        let mut graph = fixture();
        let old = graph.edge_records[0].route.clone();
        let work = RouteStageWork::new(1_000_000);
        nudge_edge_channels_with_budget(&mut graph, &work).unwrap();
        assert_eq!(work.used(), 256);
        assert_eq!(
            old.point(1),
            Point {
                x: f64::from_bits(0x4068fffffec00000),
                y: 50.
            }
        );
        assert_eq!(
            old.point(2),
            Point {
                x: f64::from_bits(0x4068fffffec00000),
                y: 250.
            }
        );
        let new = &graph.edge_records[0].route;
        assert_eq!((new.len(), new.capacity()), (4, 4));
        assert_ne!(new.header().slot_address(0), old.header().slot_address(0));
        for i in 0..4 {
            assert!(old.slot(i).unwrap().same_allocation(&new.slot(i).unwrap()));
        }
        assert_eq!(graph.position(NodeId(0)), Some(Point { x: 0., y: 0. }));
    }
    #[test]
    fn channel_work_failure_restores_published_points_and_headers() {
        for limit in [0, 1, 64, 128, 255] {
            let mut graph = fixture();
            let old = graph.edge_records[0].route.clone();
            let error = nudge_edge_channels_with_budget(&mut graph, &RouteStageWork::new(limit))
                .unwrap_err();
            assert!(error.is_route_work_limit());
            assert_eq!(
                graph.edge_records[0].route.header().slot_address(0),
                old.header().slot_address(0)
            );
            assert_eq!(old.point(1), Point { x: 120., y: 50. });
            assert_eq!(old.point(2), Point { x: 120., y: 250. });
        }
    }
    #[test]
    fn cancellation_observes_the_live_commit_then_restores_it() {
        struct Changed(SharedOwnerPoint);
        impl limits::Context for Changed {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<limits::ContextError> {
                (self.0.get().x != 120.).then_some(limits::ContextError::Cancelled)
            }
        }
        let mut graph = fixture();
        let old = graph.edge_records[0].route.clone();
        let context = Changed(old.slot(1).unwrap());
        let error = nudge_with_context(&mut graph, Some(&context), &RouteStageWork::new(1_000_000))
            .unwrap_err();
        assert!(error.to_string().contains("context canceled"));
        assert_eq!(old.point(1).x, 120.);
        assert_eq!(
            graph.edge_records[0].route.header().slot_address(0),
            old.header().slot_address(0)
        );
    }
}
