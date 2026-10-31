// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! D2 v0.9.0 postprocess.go and balance_* guards over actual owner storage.
use super::channel_solver::{maximum, minimum};
use super::nudge::{Segment, bounds, coordinate, edges, nodes, remove_duplicates};
use super::route_search_guard::{MAX_ROUTE_STAGE_WORK_UNITS, RouteStageWork};
use super::route_stage_guard::{
    self, RouteAtomicGuard, RouteMutationSnapshot, RouteStageError, RuntimeRouteContext,
};
use crate::internal::owner_groups::OwnerGroupKind;
use crate::internal::owner_points::SharedOwnerPoint;
use crate::internal::{ArenaGraph, OwnerGraphId, limits, quality};
use crate::{EdgeId, Point, ShapeKind};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, RouteStageError>;
type SegmentKey = (usize, usize, Option<EdgeId>);
fn key(s: &Segment) -> SegmentKey {
    (s.start.allocation_key(), s.end.allocation_key(), s.owner)
}
fn eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1.0
}
fn fixed_ports(g: &ArenaGraph, edge: EdgeId) -> bool {
    let e = &g.edge_records[edge.0 as usize];
    e.has_table_column()
        || g.nodes[e.from.0 as usize].shape == ShapeKind::Diamond
        || g.nodes[e.to.0 as usize].shape == ShapeKind::Diamond
}
fn in_tree(g: &ArenaGraph, edge: EdgeId) -> bool {
    let e = &g.edge_records[edge.0 as usize];
    g.owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(e.from))
        .unwrap()
        .is_some()
        || g.owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(e.to))
            .unwrap()
            .is_some()
}
pub(super) fn special(g: &ArenaGraph, edge: EdgeId) -> bool {
    let e = &g.edge_records[edge.0 as usize];
    if in_tree(g, edge) || e.has_table_column() || e.from == e.to || fixed_ports(g, edge) {
        return true;
    }
    let a = e.route.point(0);
    let b = e.route.point(e.route.len() - 1);
    (a.x - b.x).abs() == 1.0 || (a.y - b.y).abs() == 1.0
}
fn segments(
    g: &ArenaGraph,
    inventory: &[EdgeId],
    horizontal: bool,
    guard: &RouteAtomicGuard<'_>,
) -> Result<Vec<Segment>> {
    let mut out = Vec::new();
    for &edge in inventory {
        guard.step()?;
        let points = &g.edge_records[edge.0 as usize].route;
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
            out.push(Segment {
                start,
                end,
                owner: Some(edge),
            });
        }
    }
    Ok(out)
}
fn node_walls(
    g: &ArenaGraph,
    horizontal: bool,
    guard: &RouteAtomicGuard<'_>,
) -> Result<Vec<Segment>> {
    let mut out = Vec::new();
    for node in nodes(g) {
        guard.step()?;
        let start = g.owner_node_point(node).unwrap().shared_point().unwrap();
        let p = start.get();
        let size = g.nodes[node.0 as usize].rect.size;
        let right = Point {
            x: p.x + size.width,
            y: p.y,
        };
        let bottom = Point {
            x: p.x,
            y: p.y + size.height,
        };
        let corner = Point {
            x: p.x + size.width,
            y: p.y + size.height,
        };
        let (end, other) = if horizontal {
            (bottom, right)
        } else {
            (right, bottom)
        };
        out.push(Segment {
            start,
            end: SharedOwnerPoint::new(end),
            owner: None,
        });
        out.push(Segment {
            start: SharedOwnerPoint::new(other),
            end: SharedOwnerPoint::new(corner),
            owner: None,
        });
    }
    Ok(out)
}
fn connected_to_unit(
    g: &ArenaGraph,
    point: &SharedOwnerPoint,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool> {
    for node in nodes(g) {
        guard.step()?;
        let size = g.nodes[node.0 as usize].rect.size;
        if size.width == 1.0 && size.height == 1.0 {
            let p = point.get();
            let origin = g.position(node).unwrap();
            if (p.x == origin.x || p.x == origin.x + 1.0)
                && (p.y == origin.y || p.y == origin.y + 1.0)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn shared_cluster(g: &ArenaGraph, a: EdgeId, b: EdgeId, horizontal: bool) -> bool {
    let a = &g.edge_records[a.0 as usize];
    let b = &g.edge_records[b.0 as usize];
    let group = |n| g.owner_node_group_id(n, OwnerGroupKind::Cluster).unwrap();
    let (fa, ta, fb, tb) = (group(a.from), group(a.to), group(b.from), group(b.to));
    let same = if fa.is_some() && (fa == fb || fa == tb) {
        fa
    } else if ta.is_some() && (ta == fb || ta == tb) {
        ta
    } else {
        None
    };
    same.is_some_and(|id| {
        let p = g.owner_cluster_policy(id).unwrap();
        p.arrangement == p.desired_arrangement
            && p.arrangement == if horizontal { "Column" } else { "Row" }
    })
}
fn sort_batch(
    batch: &mut [usize],
    all: &[Segment],
    horizontal: bool,
    guard: &RouteAtomicGuard<'_>,
) -> Result<()> {
    if batch.len() < 2 {
        return guard.step();
    }
    let less = |a: usize, b: usize| {
        let a = all[a].start.get();
        let b = all[b].start.get();
        if coordinate(a, horizontal) == coordinate(b, horizontal) {
            coordinate(a, !horizontal) < coordinate(b, !horizontal)
        } else {
            coordinate(a, horizontal) < coordinate(b, horizontal)
        }
    };
    let mut temporary = vec![0; batch.len()];
    let mut width = 1;
    while width < batch.len() {
        for start in (0..batch.len()).step_by(2 * width) {
            let middle = (start + width).min(batch.len());
            let end = (start + 2 * width).min(batch.len());
            let (mut left, mut right, mut output) = (start, middle, start);
            while left < middle && right < end {
                guard.step()?;
                if less(batch[right], batch[left]) {
                    temporary[output] = batch[right];
                    right += 1;
                } else {
                    temporary[output] = batch[left];
                    left += 1;
                }
                output += 1;
            }
            while left < middle {
                guard.step()?;
                temporary[output] = batch[left];
                left += 1;
                output += 1;
            }
            while right < end {
                guard.step()?;
                temporary[output] = batch[right];
                right += 1;
                output += 1;
            }
        }
        guard.add(batch.len() as u64)?;
        batch.copy_from_slice(&temporary);
        if width > batch.len() / 2 {
            break;
        }
        width *= 2;
    }
    Ok(())
}
fn distribute(
    floor: f64,
    ceil: f64,
    count: usize,
    guard: &RouteAtomicGuard<'_>,
) -> Result<Vec<f64>> {
    if count == 0 || floor >= ceil {
        guard.step()?;
        return Ok(Vec::new());
    }
    let increment = ((ceil - floor) / (count + 1) as f64).floor();
    if increment <= 0.0 {
        guard.step()?;
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(count);
    for i in 1..=count {
        guard.step()?;
        out.push(floor + i as f64 * increment);
    }
    Ok(out)
}
// Source enum order: preserved, reversed, contact changed.
fn check_order(
    batch: &[usize],
    all: &[Segment],
    other: &[Segment],
    same_inventory: bool,
    proposed: &[f64],
    horizontal: bool,
    guard: &RouteAtomicGuard<'_>,
) -> Result<u8> {
    let mut moving = false;
    for (i, &s) in batch.iter().enumerate() {
        guard.step()?;
        if proposed[i] != all[s].position(horizontal) {
            moving = true;
            break;
        }
    }
    if !moving {
        return Ok(0);
    }
    let mut order = 0;
    for (j, other) in other.iter().enumerate() {
        guard.step()?;
        if same_inventory && batch.contains(&j) {
            continue;
        }
        for (i, &s) in batch.iter().enumerate() {
            guard.step()?;
            let segment = &all[s];
            if segment.owner == other.owner {
                continue;
            }
            let old = segment.position(horizontal);
            let position = other.position(horizontal);
            let (a, b, c, d) = (
                coordinate(segment.start.get(), !horizontal),
                coordinate(segment.end.get(), !horizontal),
                coordinate(other.start.get(), !horizontal),
                coordinate(other.end.get(), !horizontal),
            );
            if proposed[i] == old {
                continue;
            }
            if maximum(a, b) < minimum(c, d) || maximum(c, d) < minimum(a, b) {
                continue;
            }
            if old == position || proposed[i] == position {
                return Ok(2);
            }
            if (old < position) != (proposed[i] < position) {
                order = 1;
            }
        }
    }
    Ok(order)
}
fn reversal_removes_crossings(
    g: &ArenaGraph,
    batch: &[usize],
    all: &[Segment],
    proposed: &[f64],
    horizontal: bool,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool> {
    let mut copies: BTreeMap<usize, SharedOwnerPoint> = BTreeMap::new();
    for (i, &s) in batch.iter().enumerate() {
        guard.step()?;
        for point in [&all[s].start, &all[s].end] {
            guard.step()?;
            let mut value = point.get();
            if horizontal {
                value.x = proposed[i];
            } else {
                value.y = proposed[i];
            }
            if copies
                .get(&point.allocation_key())
                .is_some_and(|old| old.get() != value)
            {
                return Ok(false);
            }
            copies.insert(point.allocation_key(), SharedOwnerPoint::new(value));
        }
    }
    let inventory = edges(g).collect::<Vec<_>>();
    guard.add(inventory.len() as u64)?;
    let mut candidates: Vec<Option<Vec<SharedOwnerPoint>>> = vec![None; inventory.len()];
    let mut affected = Vec::new();
    for (i, &id) in inventory.iter().enumerate() {
        guard.step()?;
        let e = &g.edge_records[id.0 as usize];
        let mut changed = false;
        for point in e.route.header().iter_slots().map(Option::unwrap) {
            guard.step()?;
            if copies
                .get(&point.allocation_key())
                .is_some_and(|p| p.get() != point.get())
            {
                changed = true;
            }
        }
        if !changed {
            continue;
        }
        if e.is_curve {
            return Ok(false);
        }
        guard.add(e.route.len() as u64)?;
        let mut candidate = Vec::with_capacity(e.route.len());
        for j in 0..e.route.len() {
            guard.step()?;
            let point = e.route.slot(j).unwrap();
            let p = point.get();
            let q = copies
                .get(&point.allocation_key())
                .cloned()
                .unwrap_or_else(|| point.clone());
            candidate.push(q.clone());
            let q = q.get();
            if j > 0 {
                let before = e.route.point(j - 1);
                let after = candidate[j - 1].get();
                if (before.x != p.x && before.y != p.y) || (after.x != q.x && after.y != q.y) {
                    return Ok(false);
                }
            }
        }
        candidates[i] = Some(candidate);
        affected.push(i);
    }
    let mut improved = false;
    for i in affected {
        guard.step()?;
        for (j, &id) in inventory.iter().enumerate() {
            guard.step()?;
            if i == j || (candidates[j].is_some() && j < i) {
                continue;
            }
            let e = &g.edge_records[inventory[i].0 as usize];
            let other = &g.edge_records[id.0 as usize];
            if other.is_curve {
                return Ok(false);
            }
            let mut before = 0;
            let mut after = 0;
            for a in 0..e.route.len().saturating_sub(1) {
                guard.step()?;
                for b in 0..other.route.len().saturating_sub(1) {
                    guard.step()?;
                    let (p, q, c, d) = (
                        e.route.point(a),
                        e.route.point(a + 1),
                        other.route.point(b),
                        other.route.point(b + 1),
                    );
                    let first = candidates[i].as_ref().unwrap();
                    let (np, nq) = (first[a].get(), first[a + 1].get());
                    let (nc, nd) = candidates[j]
                        .as_ref()
                        .map_or((c, d), |x| (x[b].get(), x[b + 1].get()));
                    if super::channel_safety::crossing(p, q, c, d) {
                        before += 1;
                    }
                    if super::channel_safety::crossing(np, nq, nc, nd) {
                        after += 1;
                    }
                    if super::channel_safety::overlap(p, q, c, d) == 0.0
                        && super::channel_safety::overlap(np, nq, nc, nd) > 0.0
                    {
                        return Ok(false);
                    }
                }
            }
            if after > before {
                return Ok(false);
            }
            improved |= after < before;
        }
    }
    Ok(improved)
}
pub(super) fn regular(
    g: &mut ArenaGraph,
    special: &[EdgeId],
    regular: &[EdgeId],
    guard: &RouteAtomicGuard<'_>,
) -> Result<()> {
    let mut fixed = BTreeSet::new();
    for &id in regular {
        guard.step()?;
        if !fixed_ports(g, id) {
            continue;
        }
        let p = &g.edge_records[id.0 as usize].route;
        for i in (0..p.len().min(2)).chain(p.len().saturating_sub(2)..p.len()) {
            fixed.insert(p.slot(i).unwrap().allocation_key());
        }
    }
    for horizontal in [true, false] {
        let mut locked_walls = node_walls(g, horizontal, guard)?;
        let mut locked = BTreeSet::new();
        let all = segments(g, regular, horizontal, guard)?;
        let mut eligible = Vec::new();
        let mut special_walls = Vec::new();
        for (i, s) in all.iter().enumerate() {
            if fixed.contains(&s.start.allocation_key()) || fixed.contains(&s.end.allocation_key())
            {
                special_walls.push(s.clone());
                continue;
            }
            let start = connected_to_unit(g, &s.start, guard)?;
            let end = connected_to_unit(g, &s.end, guard)?;
            if !start && !end {
                eligible.push(i);
            }
        }
        let others = segments(g, special, horizontal, guard)?;
        for s in &others {
            guard.step()?;
            special_walls.push(s.clone());
        }
        for &i in &eligible {
            let s = &all[i];
            for j in 0..special_walls.len() {
                guard.step()?;
                let other = &special_walls[j];
                if s.overlaps(other, horizontal, 1.0)
                    && eq(s.position(horizontal), other.position(horizontal))
                {
                    special_walls.push(s.clone());
                    locked.insert(key(s));
                    break;
                }
            }
        }
        locked_walls.extend(special_walls);
        while locked.len() != eligible.len() {
            guard.step()?;
            let mut min_range = (f64::NEG_INFINITY, f64::INFINITY);
            let mut ranges: BTreeMap<(u64, u64), Vec<usize>> = BTreeMap::new();
            for &i in &eligible {
                guard.step()?;
                let s = &all[i];
                if locked.contains(&key(s)) {
                    continue;
                }
                let (mut floor, mut ceil) = bounds(s, &locked_walls, guard)?;
                if floor == f64::NEG_INFINITY {
                    floor = s.position(horizontal) - 100.0;
                }
                if ceil == f64::INFINITY {
                    ceil = s.position(horizontal) + 100.0;
                }
                ranges
                    .entry((
                        super::go_float_map_key(floor),
                        super::go_float_map_key(ceil),
                    ))
                    .or_default()
                    .push(i);
                if ceil - floor < min_range.1 - min_range.0 {
                    min_range = (floor, ceil);
                }
            }
            let minimum = ranges
                .get(&(
                    super::go_float_map_key(min_range.0),
                    super::go_float_map_key(min_range.1),
                ))
                .cloned()
                .unwrap_or_default();
            for &seed in &minimum {
                guard.step()?;
                let s = &all[seed];
                if locked.contains(&key(s)) {
                    continue;
                }
                let mut batch = vec![seed];
                for &j in &minimum {
                    guard.step()?;
                    if j == seed || locked.contains(&key(&all[j])) {
                        continue;
                    }
                    let buffer = if s.owner == all[j].owner { 0.0 } else { 40.0 };
                    if s.overlaps(&all[j], horizontal, buffer) {
                        batch.push(j);
                    }
                }
                for &j in &eligible {
                    guard.step()?;
                    if batch.contains(&j) || locked.contains(&key(&all[j])) {
                        continue;
                    }
                    let other = &all[j];
                    let together =
                        shared_cluster(g, s.owner.unwrap(), other.owner.unwrap(), horizontal);
                    if together && eq(s.position(horizontal), other.position(horizontal)) {
                        batch.push(j);
                        continue;
                    }
                    if s.overlaps(other, horizontal, 1.0)
                        && eq(s.position(horizontal), other.position(horizontal))
                    {
                        batch.push(j);
                    }
                }
                sort_batch(&mut batch, &all, horizontal, guard)?;
                let mut distinct = BTreeSet::new();
                for &i in &batch {
                    guard.step()?;
                    distinct.insert(super::go_float_map_key(all[i].position(horizontal).round()));
                }
                let distribution = distribute(min_range.0, min_range.1, distinct.len(), guard)?;
                let mut proposed = Vec::with_capacity(batch.len());
                for &i in &batch {
                    guard.step()?;
                    proposed.push(all[i].position(horizontal));
                }
                let mut segment_index = 0;
                for val in distribution {
                    guard.step()?;
                    let mut count = 0;
                    while segment_index + count < batch.len() {
                        guard.step()?;
                        if eq(
                            all[batch[segment_index]].position(horizontal),
                            all[batch[segment_index + count]].position(horizontal),
                        ) {
                            count += 1;
                        } else {
                            break;
                        }
                    }
                    for item in proposed.iter_mut().skip(segment_index).take(count) {
                        guard.step()?;
                        *item = val;
                    }
                    segment_index += count;
                }
                let mut order =
                    check_order(&batch, &all, &all, true, &proposed, horizontal, guard)?;
                if order != 2 {
                    order = order.max(check_order(
                        &batch, &all, &others, false, &proposed, horizontal, guard,
                    )?);
                }
                let accept = if order == 0 {
                    true
                } else if order == 1 {
                    reversal_removes_crossings(g, &batch, &all, &proposed, horizontal, guard)?
                } else {
                    false
                };
                if accept {
                    for (i, &s) in batch.iter().enumerate() {
                        guard.step()?;
                        for point in [&all[s].start, &all[s].end] {
                            let mut p = point.get();
                            if horizontal {
                                p.x = proposed[i];
                            } else {
                                p.y = proposed[i];
                            }
                            point.set(p);
                        }
                    }
                }
                for &i in &batch {
                    guard.step()?;
                    locked.insert(key(&all[i]));
                    locked_walls.push(all[i].clone());
                }
            }
        }
    }
    for &edge in regular {
        g.edge_records[edge.0 as usize]
            .route
            .replace(remove_duplicates(
                &g.edge_records[edge.0 as usize].route,
                guard,
            )?);
    }
    Ok(())
}
fn port_interiors(g: &mut ArenaGraph, guard: &RouteAtomicGuard<'_>) -> Result<()> {
    let mut selected = Vec::new();
    let mut locked = Vec::new();
    for edge in edges(g) {
        guard.step()?;
        let e = &g.edge_records[edge.0 as usize];
        if fixed_ports(g, edge)
            && !e.is_curve
            && e.from != e.to
            && !in_tree(g, edge)
            && e.route.len() >= 6
        {
            selected.push(edge);
        } else {
            locked.push(edge);
        }
    }
    if selected.is_empty() {
        return Ok(());
    }
    let before_quality = quality::inspect_owner(g, OwnerGraphId(0), guard.context())?;
    let snapshot = RouteMutationSnapshot::capture(g, OwnerGraphId(0), &[], guard)?;
    let mut before = BTreeMap::new();
    for &id in &selected {
        let route = &g.edge_records[id.0 as usize].route;
        guard.add(route.len() as u64 + 1)?;
        before.insert(
            id,
            crate::internal::RoutePoints::literal(
                &(0..route.len()).map(|i| route.point(i)).collect::<Vec<_>>(),
            ),
        );
    }
    regular(g, &locked, &selected, guard)?;
    for &id in &selected {
        let e = &g.edge_records[id.0 as usize];
        if !super::shortcut::changed_route_clear_between(g, id, &before[&id], &e.route, guard)? {
            snapshot.restore(g);
            return Ok(());
        }
    }
    let after_quality = quality::inspect_owner(g, OwnerGraphId(0), guard.context())?;
    if after_quality.route_obstructions > before_quality.route_obstructions
        || after_quality.crossings > before_quality.crossings
        || after_quality.text_occlusions > before_quality.text_occlusions
        || after_quality.route_length > before_quality.route_length * 1.1
    {
        snapshot.restore(g);
    }
    Ok(())
}
fn run(g: &mut ArenaGraph, guard: &RouteAtomicGuard<'_>) -> Result<()> {
    for node in nodes(g) {
        g.materialize_owner_node_point(node).unwrap();
    }
    let mut special_edges = Vec::new();
    let mut regular_edges = Vec::new();
    for edge in edges(g) {
        guard.step()?;
        let e = &g.edge_records[edge.0 as usize];
        if e.route.len() < 2 {
            return Err(limits::Error::Invalid(format!(
                "TALA BalanceEdgeSegments edge {} has an incomplete route",
                e.tala_id
            ))
            .into());
        }
        if special(g, edge) {
            special_edges.push(edge);
        } else {
            regular_edges.push(edge);
        }
    }
    regular(g, &special_edges, &regular_edges, guard)?;
    port_interiors(g, guard)
}
pub(super) fn balance_with_context(
    g: &mut ArenaGraph,
    context: Option<&dyn limits::Context>,
    budget: &RouteStageWork,
) -> Result<()> {
    route_stage_guard::run_atomic_route_stage_typed(
        g,
        Some(OwnerGraphId(0)),
        context,
        "BalanceEdgeSegments",
        &[],
        budget,
        run,
    )
}
pub(in crate::internal) fn balance_edge_segments_with_budget(
    g: &mut ArenaGraph,
    budget: &RouteStageWork,
) -> Result<()> {
    balance_with_context(g, Some(&RuntimeRouteContext), budget)
}
pub(in crate::internal) fn balance_edge_segments(g: &mut ArenaGraph) {
    let _ = balance_edge_segments_with_budget(g, &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn z_graph() -> ArenaGraph {
        let input=br#"{"nodes":[{"id":1,"width":100,"height":100,"topLeft":{"x":0,"y":0},"shapeType":"Square"},{"id":2,"width":100,"height":100,"topLeft":{"x":300,"y":200},"shapeType":"Square"}],"edges":[{"id":9,"from":1,"to":2,"points":[{"x":100,"y":50},{"x":120,"y":50},{"x":120,"y":250},{"x":300,"y":250}]}],"containers":{"0":[1,2]}}"#;
        let core = crate::graphjson::graphjson_to_graph(input).unwrap();
        ArenaGraph::from_input(&core)
    }
    #[test]
    fn source_z_balancing_updates_retained_points_and_publishes_a_fresh_header() {
        let mut g = z_graph();
        let old = g.edge_records[0].route.clone();
        let budget = RouteStageWork::new(100000);
        balance_edge_segments_with_budget(&mut g, &budget).unwrap();
        assert_eq!(budget.used(), 122);
        assert_eq!(old.point(1).x, 200.0);
        let current = &g.edge_records[0].route;
        assert!(
            old.slot(1)
                .unwrap()
                .same_allocation(&current.slot(1).unwrap())
        );
        assert_ne!(
            old.header().slot_address(0),
            current.header().slot_address(0)
        );
        assert_eq!((current.len(), current.capacity()), (4, 4));
    }
    #[test]
    fn every_source_z_work_boundary_restores_original_points_and_header() {
        for limit in 0..122 {
            let mut g = z_graph();
            let old = g.edge_records[0].route.clone();
            let address = old.header().slot_address(0);
            assert!(
                balance_edge_segments_with_budget(&mut g, &RouteStageWork::new(limit)).is_err(),
                "limit {limit}"
            );
            assert_eq!(old.point(1).x, 120.0, "limit {limit}");
            assert_eq!(
                g.edge_records[0].route.header().slot_address(0),
                address,
                "limit {limit}"
            );
        }
    }
}
