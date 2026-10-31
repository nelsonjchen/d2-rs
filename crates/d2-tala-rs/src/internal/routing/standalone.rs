// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned standalone_router.go entry validation and atomic API boundary.
use super::route_search_guard::RouteStageWork;
use super::route_stage_guard::{self, RouteAtomicGuard, RouteStageError};
use crate::EdgeId;
use crate::internal::{ArenaGraph, OwnerGraphId, limits};
use std::collections::BTreeSet;
fn invalid(message: impl Into<String>) -> RouteStageError {
    limits::Error::Invalid(message.into()).into()
}
pub(super) fn run(
    graph: &mut ArenaGraph,
    receiver: Option<OwnerGraphId>,
    context: Option<&dyn limits::Context>,
    selected: &[Option<EdgeId>],
    budget: &RouteStageWork,
    operation: impl FnOnce(&mut ArenaGraph, &RouteAtomicGuard<'_>) -> Result<(), RouteStageError>,
) -> Result<(), RouteStageError> {
    let receiver = receiver
        .filter(|id| graph.owner_graph_view(*id).is_some())
        .ok_or_else(|| invalid("cannot route edges on a nil graph"))?;
    let context = context.ok_or_else(|| invalid("cannot route edges without a context"))?;
    if let Some(cause) = context.error() {
        return Err(limits::Error::Context {
            location: String::new(),
            cause,
        }
        .into());
    }
    if selected.len() > 1_000_000 {
        return Err(invalid(
            "TALA EdgeRouting selected edge references exceed limit 1000000",
        ));
    }
    graph.validate_owner_graph_topology(Some(receiver), Some(context), "RouteEdges")?;
    if selected.is_empty() {
        return Ok(());
    }
    let view = graph.owner_graph_view(receiver).unwrap();
    if view.nodes.len() == 0 {
        return Err(invalid("cannot route edges on an empty graph"));
    }
    let edges = view.edges.header();
    if selected.len() > edges.len() {
        return Err(invalid(format!(
            "selected edge count {} exceeds graph edge count {}",
            selected.len(),
            edges.len()
        )));
    }
    let mut graph_nodes = BTreeSet::new();
    for (index, node) in view.nodes.iter_slots().enumerate() {
        let node = node.ok_or_else(|| invalid(format!("graph node at index {index} is nil")))?;
        graph_nodes.insert(node);
    }
    let mut graph_edges = BTreeSet::new();
    for (index, edge) in edges.iter_slots().enumerate() {
        let edge = edge.ok_or_else(|| invalid(format!("graph edge at index {index} is nil")))?;
        let e = &graph.edge_records[edge.0 as usize];
        if graph.position(e.from).is_none() || graph.position(e.to).is_none() {
            return Err(invalid(format!(
                "graph edge {} has unplaced or missing endpoints",
                e.tala_id
            )));
        }
        if !graph_nodes.contains(&e.from) {
            return Err(invalid(format!(
                "graph edge {} source node does not belong to the graph",
                e.tala_id
            )));
        }
        if !graph_nodes.contains(&e.to) {
            return Err(invalid(format!(
                "graph edge {} target node does not belong to the graph",
                e.tala_id
            )));
        }
        graph_edges.insert(edge);
    }
    let mut selected_set = BTreeSet::new();
    for edge in selected {
        let edge = edge.ok_or_else(|| invalid("cannot route a nil edge"))?;
        if !graph_edges.contains(&edge) {
            let id = graph.edge_records[edge.0 as usize].tala_id;
            return Err(invalid(format!("edge {id} does not belong to the graph")));
        }
        if !selected_set.insert(edge) {
            return Err(invalid(format!(
                "edge {} was selected more than once",
                graph.edge_records[edge.0 as usize].tala_id
            )));
        }
    }
    for edge in edges.iter_slots() {
        let edge = edge.unwrap();
        if selected_set.contains(&edge) {
            continue;
        }
        let e = &graph.edge_records[edge.0 as usize];
        if e.route.len() < 2 || e.route.header().iter_slots().any(|p| p.is_none()) {
            return Err(invalid(format!(
                "unselected edge {} has no complete route",
                e.tala_id
            )));
        }
    }
    route_stage_guard::run_atomic_route_stage_after_preflight(
        graph,
        receiver,
        Some(context),
        "EdgeRouting",
        selected,
        budget,
        |g, guard| {
            guard.add(g.owner_graph_view(receiver).unwrap().nodes.len() as u64)?;
            operation(g, guard)
        },
    )
}

fn fallback(
    g: &mut ArenaGraph,
    id: EdgeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<(), RouteStageError> {
    guard.step()?;
    let all = super::nudge::edges(g)
        .map(|e| e.0 as usize)
        .collect::<Vec<_>>();
    let routes = g
        .edge_records
        .iter()
        .map(|e| e.route.read().clone())
        .collect::<Vec<_>>();
    let original = super::straight::estimated_selected_edge_cost_with_work(
        id.0 as usize,
        &routes[id.0 as usize],
        &routes,
        &all,
        guard,
    )
    .ok_or_else(|| guard.take_kernel_failure().expect("cost work failure"))?;
    let line = super::route_line::route_line_with_stage_budget(g, id.0 as usize, &all, guard);
    if let Some(error) = guard.take_kernel_failure() {
        return Err(error);
    }
    let Some(line) = line else {
        return Ok(());
    };
    let route = &g.edge_records[id.0 as usize].route;
    let mut cost = line.cost;
    if route.len() == 4 && super::euclidean_distance(route.point(1), route.point(2)) <= 5.0 {
        cost /= 4.0;
    }
    guard.step()?;
    if cost < original {
        g.edge_records[id.0 as usize]
            .route
            .replace(crate::internal::RoutePoints::literal(&[
                line.source.point,
                line.target.point,
            ]));
    }
    Ok(())
}
fn straight(
    g: &ArenaGraph,
    id: EdgeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool, RouteStageError> {
    let r = &g.edge_records[id.0 as usize].route;
    if r.len() < 2 {
        guard.step()?;
        return Ok(false);
    }
    for i in 2..r.len() {
        guard.step()?;
        if crate::internal::labels::orientation(r.point(i - 2), r.point(i - 1), r.point(i)) != 0.0 {
            return Ok(false);
        }
    }
    Ok(true)
}
fn overlapping_end(
    g: &ArenaGraph,
    id: EdgeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool, RouteStageError> {
    let e = &g.edge_records[id.0 as usize];
    let start = e.route.slot(0);
    let end = e.route.slot(e.route.len() - 1);
    for (node, point) in [(e.from, start), (e.to, end)] {
        for other in g.nodes[node.0 as usize]
            .edges
            .header()
            .iter_slots()
            .map(Option::unwrap)
        {
            guard.step()?;
            if other == id {
                continue;
            }
            let r = &g.edge_records[other.0 as usize].route;
            let equal = |value: Option<crate::internal::owner_points::SharedOwnerPoint>| {
                value
                    .zip(point.as_ref())
                    .is_some_and(|(a, b)| a.get() == b.get())
            };
            if equal(r.slot(0)) || equal(r.slot(r.len() - 1)) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn reserve_sort(len: usize, guard: &RouteAtomicGuard<'_>) -> Result<(), RouteStageError> {
    if len < 2 {
        return guard.step();
    }
    let mut width = 1;
    while width < len {
        guard.add(len as u64)?;
        if width > len / 2 {
            break;
        }
        width *= 2;
    }
    guard.finish()
}
fn reverse(
    g: &ArenaGraph,
    id: EdgeId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<(), RouteStageError> {
    let r = &g.edge_records[id.0 as usize].route;
    for i in 0..r.len() / 2 {
        guard.step()?;
        r.swap_slots(i, r, r.len() - 1 - i);
    }
    Ok(())
}
fn reorder(
    g: &mut ArenaGraph,
    edges: &[EdgeId],
    guard: &RouteAtomicGuard<'_>,
) -> Result<(), RouteStageError> {
    for &id in edges {
        guard.step()?;
        let e = &g.edge_records[id.0 as usize];
        if e.has_table_column() || e.label.is_none() {
            continue;
        }
        if !straight(g, id, guard)? {
            continue;
        }
        let overlap = overlapping_end(g, id, guard)?;
        let mut duplicates = Vec::new();
        for (index, &other) in edges.iter().enumerate() {
            guard.step()?;
            if other == id
                || !crate::internal::labels::is_duplicate(
                    &g.edge_records[other.0 as usize],
                    &g.edge_records[id.0 as usize],
                )
            {
                continue;
            }
            if !straight(g, other, guard)? {
                continue;
            }
            if (overlap || overlapping_end(g, other, guard)?)
                && !crate::internal::labels::duplicate_arrowheads_match(
                    &g.edge_records[id.0 as usize],
                    &g.edge_records[other.0 as usize],
                )
            {
                continue;
            }
            duplicates.push(index);
        }
        if duplicates.len() <= 1 {
            continue;
        }
        let e = &g.edge_records[id.0 as usize];
        let a = e.route.point(0);
        let b = e.route.point(e.route.len() - 1);
        let x = b.y - a.y > b.x - a.x;
        let mut source = vec![id];
        let mut target = vec![id];
        for i in duplicates {
            guard.step()?;
            source.push(edges[i]);
            target.push(edges[i]);
        }
        reserve_sort(source.len(), guard)?;
        reserve_sort(target.len(), guard)?;
        let coordinate = |p: crate::Point| if x { p.x } else { p.y };
        crate::internal::go_sort::sort_by(&mut source, |a, b| {
            coordinate(g.edge_records[a.0 as usize].route.point(0))
                < coordinate(g.edge_records[b.0 as usize].route.point(0))
        });
        crate::internal::go_sort::sort_by(&mut target, |a, b| {
            let a = &g.edge_records[a.0 as usize].route;
            let b = &g.edge_records[b.0 as usize].route;
            coordinate(a.point(a.len() - 1)) < coordinate(b.point(b.len() - 1))
        });
        let mut consistent = true;
        for i in 0..source.len() {
            guard.step()?;
            if source[i] != target[i] {
                consistent = false;
                break;
            }
        }
        if !consistent {
            continue;
        }
        let mut current = None;
        for (i, &e) in source.iter().enumerate() {
            guard.step()?;
            if e == id {
                current = Some(i);
                break;
            }
        }
        if current == Some(0) || current == Some(source.len() - 1) {
            continue;
        }
        let first = source[0];
        let last = *source.last().unwrap();
        if g.edge_records[first.0 as usize].label.is_some()
            && g.edge_records[last.0 as usize].label.is_some()
        {
            continue;
        }
        guard.step()?;
        let other = if g.edge_records[last.0 as usize].label.is_none() {
            last
        } else {
            first
        };
        let a = g.edge_records[id.0 as usize].route.clone();
        let b = g.edge_records[other.0 as usize].route.clone();
        g.edge_records[id.0 as usize].route.replace(b);
        g.edge_records[other.0 as usize].route.replace(a);
        if g.edge_records[other.0 as usize].from == g.edge_records[id.0 as usize].to {
            reverse(g, id, guard)?;
            reverse(g, other, guard)?;
        }
    }
    guard.finish()
}
#[cfg(feature = "diagnostic-traces")]
fn trace(g: &ArenaGraph, stage: &str) {
    crate::internal::snapshot::trace_standalone_stage(g, stage);
}
pub(super) fn body(
    g: &mut ArenaGraph,
    requested: &[EdgeId],
    seed: i64,
    guard: &RouteAtomicGuard<'_>,
) -> Result<(), RouteStageError> {
    // Arena positions become the same shared point storage used by the owner APIs.
    for node in super::nudge::nodes(g) {
        g.materialize_owner_node_point(node).unwrap();
    }
    g.compute_cell_size();
    super::initialize_standalone_routing_costs(g);
    let routes = super::search::route_additional_edges_with_context(
        g,
        requested,
        Some(super::MAX_ROUTE_SEARCH_WORK_UNITS),
        seed,
        guard.context(),
    )?;
    let selected = requested.iter().copied().collect::<BTreeSet<_>>();
    for (index, route) in routes {
        g.edge_records[index].route.replace(route);
    }
    #[cfg(feature = "diagnostic-traces")]
    trace(g, "finalized");
    guard.step()?;
    for &id in requested {
        if g.edge_records[id.0 as usize].has_table_column() {
            guard.step()?;
            continue;
        }
        fallback(g, id, guard)?;
    }
    #[cfg(feature = "diagnostic-traces")]
    trace(g, "straight");
    let mut locked = Vec::new();
    for id in super::nudge::edges(g) {
        guard.step()?;
        if !selected.contains(&id) {
            locked.push(id);
        }
    }
    let mut regular = Vec::new();
    for &id in requested {
        guard.step()?;
        if super::balance_owner::special(g, id) {
            locked.push(id);
        } else {
            regular.push(id);
        }
    }
    super::balance_owner::regular(g, &locked, &regular, guard)?;
    #[cfg(feature = "diagnostic-traces")]
    trace(g, "balanced");
    for &id in requested {
        super::trace::trace_edge_guarded(g, id, guard)?;
    }
    #[cfg(feature = "diagnostic-traces")]
    trace(g, "clipped");
    reorder(g, requested, guard)?;
    #[cfg(feature = "diagnostic-traces")]
    trace(g, "duplicates");
    guard.step()?;
    crate::internal::labels::place_new_edge_labels(
        g,
        &requested.iter().map(|e| e.0 as usize).collect::<Vec<_>>(),
        guard.context(),
    )?;
    #[cfg(feature = "diagnostic-traces")]
    trace(g, "labels");
    guard.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Point, internal::RoutePoints};
    fn graph(size: f64, fixed: bool, source_arrow: &str) -> ArenaGraph {
        let mut nodes = vec![
            serde_json::json!({"id":1,"width":size,"height":size,"topLeft":{"x":0,"y":0}}),
            serde_json::json!({"id":2,"width":size,"height":size,"topLeft":{"x":if fixed {200.0} else {size},"y":0}}),
        ];
        let mut edges = vec![
            serde_json::json!({"id":11,"from":1,"to":2,"points":[],"sourceArrowhead":source_arrow}),
        ];
        if fixed {
            nodes.push(
                serde_json::json!({"id":3,"width":size,"height":size,"topLeft":{"x":100,"y":120}}),
            );
            edges.push(serde_json::json!({"id":12,"from":1,"to":3,"points":[{"x":40,"y":20},{"x":70,"y":20},{"x":70,"y":140},{"x":100,"y":140}]}));
        }
        let ids = (1..=nodes.len()).collect::<Vec<_>>();
        let bytes = serde_json::to_vec(
            &serde_json::json!({"nodes":nodes,"edges":edges,"containers":{"0":ids}}),
        )
        .unwrap();
        let input = crate::graphjson::graphjson_to_graph(&bytes).unwrap();
        let mut graph = ArenaGraph::from_input(&input);
        // The source fixture starts with an uninitialized cell-size cache.
        graph.cell_size = 0.0;
        graph
    }
    fn route(g: &mut ArenaGraph, limit: u64) -> Result<u64, RouteStageError> {
        let budget = RouteStageWork::new(limit);
        run(
            g,
            Some(OwnerGraphId(0)),
            Some(&super::super::route_stage_guard::RuntimeRouteContext),
            &[Some(EdgeId(0))],
            &budget,
            |g, guard| body(g, &[EdgeId(0)], 1, guard),
        )?;
        Ok(budget.used())
    }
    #[test]
    fn standalone_shared_port_publishes_two_points_before_balancing() {
        // Pinned source standalone observer: both sizes consume 81 units.
        for size in [0.0, 80.125] {
            let mut g = graph(size, false, "");
            assert_eq!(route(&mut g, 81).unwrap(), 81);
            let current = &g.edge_records[0].route;
            assert_eq!((current.len(), current.capacity()), (2, 2));
            assert_eq!(
                current.point(0),
                Point {
                    x: size,
                    y: (size / 2.0).round()
                }
            );
            assert_eq!(
                current.point(1),
                Point {
                    x: size + 1.0,
                    y: (size / 2.0).round()
                }
            );
            assert!(
                !current
                    .slot(0)
                    .unwrap()
                    .same_allocation(&current.slot(1).unwrap())
            );
        }
    }
    #[test]
    fn standalone_every_shared_port_work_failure_restores_the_owner() {
        for limit in 0..81 {
            let mut g = graph(80.125, false, "");
            // Spare capacity is observed independently by the differential
            // corpus; this regression keeps the source empty-route input.
            let old: RoutePoints = g.edge_records[0].route.clone();
            let positions = g.nodes.iter().map(|n| n.position).collect::<Vec<_>>();
            let cell = g.cell_size;
            let costs = g.raw_routing_costs();
            assert!(route(&mut g, limit).is_err(), "limit {limit}");
            assert_eq!(g.cell_size, cell, "limit {limit}");
            assert_eq!(g.raw_routing_costs(), costs, "limit {limit}");
            assert_eq!(
                g.nodes.iter().map(|n| n.position).collect::<Vec<_>>(),
                positions,
                "limit {limit}"
            );
            let current = &g.edge_records[0].route;
            assert_eq!(
                (current.len(), current.capacity(), current.header().is_nil()),
                (old.len(), old.capacity(), old.header().is_nil()),
                "limit {limit}"
            );
        }
    }
    #[test]
    fn standalone_raw_none_arrowhead_changes_the_guarded_port_selection() {
        for (arrow, expected) in [("", 281), ("none", 225)] {
            let mut g = graph(40.0, true, arrow);
            let old = g.edge_records[1].route.clone();
            let values = old.read().clone();
            assert_eq!(route(&mut g, expected).unwrap(), expected);
            assert!(
                g.edge_records[1]
                    .route
                    .header()
                    .same_backing_allocation(&old.header())
            );
            assert_eq!(g.edge_records[1].route.read(), values);
        }
    }
    #[test]
    fn standalone_closed_context_reaches_child_derivation_without_empty_reservation() {
        use crate::internal::limits::{Context, ContextError};
        struct Closed(std::cell::Cell<u64>);
        impl Context for Closed {
            fn done(&self) -> Option<bool> {
                Some(true)
            }
            fn error(&self) -> Option<ContextError> {
                self.0.set(self.0.get() + 1);
                None
            }
        }
        let mut g = graph(40.0, true, "");
        let before = g
            .edge_records
            .iter()
            .map(|e| e.route.clone())
            .collect::<Vec<_>>();
        let values = before.iter().map(|r| r.read().clone()).collect::<Vec<_>>();
        let cell = g.cell_size;
        let costs = g.raw_routing_costs();
        let context = Closed(std::cell::Cell::new(0));
        let work = RouteStageWork::new(50_000_000);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(
                &mut g,
                Some(OwnerGraphId(0)),
                Some(&context),
                &[Some(EdgeId(0))],
                &work,
                |g, guard| body(g, &[EdgeId(0)], 1, guard),
            )
        }));
        let panic = result.unwrap_err();
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied());
        assert_eq!(
            message,
            Some("context: internal error: missing cancel error")
        );
        // Pinned Go: 3,372 calls through base OVG construction, five fixed-route
        // connections, 18 router-constructor calls, then Err and Cause in WithCancel.
        assert_eq!(context.0.get(), 3397);
        assert_eq!(work.used(), 39);
        assert_eq!(g.cell_size, cell);
        assert_eq!(g.raw_routing_costs(), costs);
        for (i, old) in before.iter().enumerate() {
            assert!(
                g.edge_records[i]
                    .route
                    .header()
                    .same_backing_allocation(&old.header())
            );
            assert_eq!(g.edge_records[i].route.read(), values[i]);
        }
    }
    #[test]
    fn standalone_ovg_cancellation_and_panic_restore_the_owner() {
        use crate::internal::limits::{Context, ContextError};
        struct AtPoll {
            polls: std::cell::Cell<u64>,
            at: u64,
            mode: u8,
        }
        impl Context for AtPoll {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                let poll = self.polls.get() + 1;
                self.polls.set(poll);
                if poll < self.at {
                    return None;
                }
                match self.mode {
                    0 => Some(ContextError::Cancelled),
                    1 => Some(ContextError::DeadlineExceeded),
                    _ => panic!("route context panic"),
                }
            }
        }
        // The direct Go observer retains 39 outer work units when failure
        // occurs in OVG construction. Its independent work is not charged to
        // the post-routing budget, and the owner snapshot is already armed.
        for mode in 0..3 {
            for at in 13..=128 {
                let mut g = graph(40.0, true, "");
                let before = g
                    .edge_records
                    .iter()
                    .map(|e| e.route.clone())
                    .collect::<Vec<_>>();
                let values = before.iter().map(|r| r.read().clone()).collect::<Vec<_>>();
                let cell = g.cell_size;
                let costs = g.raw_routing_costs();
                let context = AtPoll {
                    polls: std::cell::Cell::new(0),
                    at,
                    mode,
                };
                let work = RouteStageWork::new(50_000_000);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(
                        &mut g,
                        Some(OwnerGraphId(0)),
                        Some(&context),
                        &[Some(EdgeId(0))],
                        &work,
                        |g, guard| body(g, &[EdgeId(0)], 1, guard),
                    )
                }));
                match mode {
                    2 => assert!(result.is_err(), "poll {at}"),
                    _ => {
                        let error = result.unwrap().unwrap_err().to_string();
                        assert_eq!(
                            error,
                            if mode == 0 {
                                "EdgeRouting: context canceled"
                            } else {
                                "EdgeRouting: context deadline exceeded"
                            }
                        );
                    }
                }
                assert_eq!(context.polls.get(), at);
                assert_eq!(work.used(), 39);
                assert_eq!(g.cell_size, cell);
                assert_eq!(g.raw_routing_costs(), costs);
                for (i, old) in before.iter().enumerate() {
                    assert!(
                        g.edge_records[i]
                            .route
                            .header()
                            .same_backing_allocation(&old.header())
                    );
                    assert_eq!(g.edge_records[i].route.read(), values[i]);
                }
            }
        }
    }
}
