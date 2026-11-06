// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! coordinator.go finalizeSelectedRoutes/reorderSelectedRoutes.
use super::{
    ArenaGraph, Point, published_route::PublishedRoute, route_search_guard::LocalRouteSearchWork,
};
use crate::{
    LayoutError,
    internal::{RoutePoints, limits::Context},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Updates = Vec<(usize, RoutePoints)>;
#[derive(Debug)]
pub(super) enum Error {
    Invariant(String),
    Guard(LayoutError),
}
impl Error {
    pub(super) fn into_layout_error(self) -> LayoutError {
        match self {
            Self::Invariant(reason) => LayoutError::InvariantViolation(reason),
            Self::Guard(error) => error,
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invariant(reason) => write!(f, "layout invariant violated: {reason}"),
            Self::Guard(LayoutError::Cancelled) => f.write_str("EdgeRouting: context canceled"),
            Self::Guard(LayoutError::DeadlineExceeded) => {
                f.write_str("EdgeRouting: context deadline exceeded")
            }
            Self::Guard(error) => error.fmt(f),
        }
    }
}
fn invalid(message: &str) -> Error {
    Error::Invariant(message.into())
}
pub(super) fn failure(work: &LocalRouteSearchWork, flavor: &'static str) -> Error {
    Error::Guard(match work.failure() {
        cause @ (LayoutError::Cancelled | LayoutError::DeadlineExceeded) => cause,
        _ => LayoutError::RouteSearchWorkLimit {
            flavor,
            limit: work.limit(),
        },
    })
}

/// Route IDs preserve pointer identity even when a slice repeats a Route or
/// distinct Routes refer to one edge. All updates remain private until finish.
pub(super) fn selected<'a>(
    graph: Option<&ArenaGraph>,
    records: &mut [PublishedRoute],
    order: &[Option<usize>],
    selected_edges: Option<&BTreeSet<usize>>,
    work: Option<&LocalRouteSearchWork<'a>>,
    context: &'a dyn Context,
    flavor: &'static str,
    reorder_parallel_edges: bool,
) -> Result<Updates, Error> {
    let work = work.ok_or_else(|| invalid("selected edge-routing flavor has no work guard"))?;
    let graph = graph.ok_or_else(|| invalid("selected edge-routing flavor has no graph"))?;
    // The caller-owned guard must be able to retain this context until it is
    // next rebound. This operation uses the same lifetime as its work guard.
    selected_bound(
        graph,
        records,
        order,
        selected_edges,
        work,
        context,
        flavor,
        reorder_parallel_edges,
    )
}

fn selected_bound<'a>(
    graph: &ArenaGraph,
    records: &mut [PublishedRoute],
    order: &[Option<usize>],
    selected_edges: Option<&BTreeSet<usize>>,
    work: &LocalRouteSearchWork<'a>,
    context: &'a dyn Context,
    flavor: &'static str,
    reorder_parallel_edges: bool,
) -> Result<Updates, Error> {
    work.bind(context).map_err(|_| failure(work, flavor))?;
    let mut finished = false;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let step = || work.step().map_err(|_| failure(work, flavor));
        let add = |units: usize| work.add(units as u64).map_err(|_| failure(work, flavor));
        let mut routes = order.to_vec();
        if let Some(selected) = selected_edges {
            let mut filtered = Vec::with_capacity(selected.len());
            let mut found = BTreeSet::new();
            for id in &routes {
                step()?;
                let id = id
                    .filter(|id| *id < records.len())
                    .ok_or_else(|| invalid("selected edge-routing flavor produced a nil route"))?;
                let edge = records[id].edge;
                if !selected.contains(&edge) {
                    continue;
                }
                if !found.insert(edge) {
                    return Err(invalid(
                        "selected edge-routing flavor produced duplicate routes for an edge",
                    ));
                }
                filtered.push(Some(id));
            }
            if found.len() != selected.len() {
                return Err(invalid(
                    "selected edge-routing flavor omitted a requested edge",
                ));
            }
            routes = filtered;
        }
        for id in &routes {
            step()?;
            let id = id
                .filter(|id| *id < records.len())
                .ok_or_else(|| invalid("selected edge-routing flavor produced an invalid route"))?;
            let route = &records[id];
            let edge = graph
                .edge_records
                .get(route.edge)
                .ok_or_else(|| invalid("selected edge-routing flavor produced an invalid route"))?;
            if graph.nodes.get(edge.from.0 as usize).is_none()
                || graph.nodes.get(edge.to.0 as usize).is_none()
                || route.len() < 2
            {
                return Err(invalid(
                    "selected edge-routing flavor produced an invalid route",
                ));
            }
            for _ in 0..route.len() {
                step()?;
            }
        }
        // Successful validation makes every ID present and all later edge
        // dereferences valid. The Route records themselves retain live points.
        let routes = routes.into_iter().map(Option::unwrap).collect::<Vec<_>>();
        if reorder_parallel_edges {
            reorder(graph, records, &routes, work, flavor)?;
        }
        add(routes.len())?;
        let mut updates = Vec::with_capacity(routes.len());
        for id in routes {
            step()?;
            add(records[id].len())?;
            updates.push((records[id].edge, records[id].endpoint_slice()));
        }
        work.finish().map_err(|_| failure(work, flavor))?;
        finished = true;
        Ok(updates)
    }));
    // As in Go's defer, record accepted work after an error or panic without
    // replacing an existing ordinary error with a later cleanup error.
    if !finished {
        let _ = work.finish();
    }
    match result {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn can_swap(
    graph: &ArenaGraph,
    records: &[PublishedRoute],
    first: usize,
    second: usize,
    routes: &[usize],
    work: &LocalRouteSearchWork,
    flavor: &'static str,
) -> Result<bool, Error> {
    let a = &graph.edge_records[records[first].edge];
    let b = &graph.edge_records[records[second].edge];
    if a.from == a.to || b.from == b.to {
        return Ok(false);
    }
    let same = a.from == b.from && a.to == b.to;
    if !same && !(a.from == b.to && a.to == b.from) {
        return Ok(false);
    }
    let orientation = graph.sized_orientation(a.from, a.to);
    if orientation.is_diagonal() {
        return Ok(false);
    }
    let ports = |id: usize| {
        records[id]
            .endpoints
            .unwrap_or([Point { x: 0.0, y: 0.0 }; 2])
    };
    let [af, at] = ports(first);
    let [bf, bt] = ports(second);
    if same
        && ((orientation.is_vertical() && (af.y != bf.y || at.y != bt.y))
            || (orientation.is_horizontal() && (af.x != bf.x || at.x != bt.x)))
    {
        return Ok(false);
    }
    for &other in routes {
        work.step().map_err(|_| failure(work, flavor))?;
        if other == first || other == second {
            continue;
        }
        let [from, to] = ports(other);
        if [af, at, bf, bt]
            .iter()
            .any(|point| *point == from || *point == to)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn reorder(
    graph: &ArenaGraph,
    records: &mut [PublishedRoute],
    routes: &[usize],
    work: &LocalRouteSearchWork,
    flavor: &'static str,
) -> Result<(), Error> {
    let step = || work.step().map_err(|_| failure(work, flavor));
    let add = |n: usize| work.add(n as u64).map_err(|_| failure(work, flavor));
    let sort = |n: usize| work.reserve_sort(n).map_err(|_| failure(work, flavor));
    add(routes.len())?;
    let mut grouped = BTreeSet::new();
    let mut buckets = Vec::new();
    for (index, &route) in routes.iter().enumerate() {
        step()?;
        if !grouped.insert(route) {
            continue;
        }
        let mut bucket = vec![route];
        for &other in &routes[index + 1..] {
            step()?;
            if grouped.contains(&other) {
                continue;
            }
            if can_swap(graph, records, route, other, routes, work, flavor)? {
                bucket.push(other);
                grouped.insert(other);
            }
        }
        if bucket.len() > 1 {
            buckets.push(bucket);
        }
    }
    add(graph.edge_order.len())?;
    let mut edge_indices = BTreeMap::new();
    for (index, edge) in graph.edge_order.header().iter_slots().enumerate() {
        step()?;
        edge_indices.insert(edge.map(|e| e.0 as usize), index as isize);
    }
    for mut bucket in buckets {
        step()?;
        sort(bucket.len())?;
        crate::internal::go_sort::sort_by(&mut bucket, |&left, &right| {
            let a = &records[left];
            let b = &records[right];
            let ae = &graph.edge_records[a.edge];
            let be = &graph.edge_records[b.edge];
            let ap = a.endpoints.unwrap_or([Point { x: 0.0, y: 0.0 }; 2]);
            let bp = b.endpoints.unwrap_or([Point { x: 0.0, y: 0.0 }; 2]);
            let other = if ae.from == be.from { bp[0] } else { bp[1] };
            if graph.sized_orientation(ae.from, ae.to).is_vertical() {
                ap[0].x < other.x
            } else {
                ap[0].y < other.y
            }
        });
        add(bucket.len())?;
        let mut edges = Vec::with_capacity(bucket.len());
        for &id in &bucket {
            step()?;
            edges.push(records[id].edge);
        }
        sort(edges.len())?;
        crate::internal::go_sort::sort_by(&mut edges, |left, right| {
            edge_indices.get(&Some(*left)).copied().unwrap_or(-1)
                < edge_indices.get(&Some(*right)).copied().unwrap_or(-1)
        });
        for (id, edge) in bucket.into_iter().zip(edges) {
            step()?;
            if graph.edge_records[records[id].edge].from != graph.edge_records[edge].from {
                add(records[id].len())?;
                records[id].reverse();
            }
            records[id].edge = edge;
        }
    }
    work.check().map_err(|_| failure(work, flavor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::owner_points::SharedOwnerPoint;

    #[derive(Default)]
    struct Caller {
        polls: std::cell::Cell<usize>,
        cancel_at: usize,
    }
    impl Context for Caller {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<crate::internal::limits::ContextError> {
            let count = self.polls.get() + 1;
            self.polls.set(count);
            (self.cancel_at != 0 && count >= self.cancel_at)
                .then_some(crate::internal::limits::ContextError::Cancelled)
        }
    }
    fn fixture() -> (ArenaGraph, Vec<PublishedRoute>) {
        let mut graph = crate::Graph::default();
        let a = graph.add_node(crate::Node::new(
            "a",
            crate::Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let b = graph.add_node(crate::Node::new(
            "b",
            crate::Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        graph.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        graph.add_edge(crate::Edge {
            source: b,
            target: a,
        });
        let mut graph = ArenaGraph::from_input(&graph);
        graph.set_position(a, Point { x: 0.0, y: 0.0 });
        graph.set_position(b, Point { x: 100.0, y: 0.0 });
        let mut records = Vec::new();
        for (edge, y) in [(0, 15.0), (1, 5.0)] {
            let mut nodes = vec![
                Point { x: 10.0, y: 10.0 },
                Point { x: 20.0, y },
                Point { x: 100.0, y },
                Point { x: 110.0, y: 10.0 },
            ];
            if edge == 1 {
                nodes.reverse();
            }
            records.push(PublishedRoute::new(
                edge,
                nodes.clone(),
                None,
                Some([nodes[1], nodes[2]]),
                vec![],
            ));
        }
        (graph, records)
    }

    #[test]
    fn finalization_limits_leave_graph_unchanged_and_preserve_route_identity() {
        let (graph, initial) = fixture();
        let before = graph
            .edge_records
            .iter()
            .map(|e| e.route.read().to_vec())
            .collect::<Vec<_>>();
        let caller = Caller::default();
        let work = LocalRouteSearchWork::with_context(1000, &caller).unwrap();
        let mut records = initial.clone();
        let updates = selected(
            Some(&graph),
            &mut records,
            &[Some(0), Some(1)],
            None,
            Some(&work),
            &caller,
            "Default",
            true,
        )
        .unwrap();
        let total = work.used();
        assert_eq!(
            records.iter().map(|r| r.edge).collect::<Vec<_>>(),
            vec![1, 0]
        );
        assert_eq!(
            updates.iter().map(|(edge, _)| *edge).collect::<Vec<_>>(),
            vec![1, 0]
        );
        assert_eq!(updates[0].1.point(0).x, 100.0);
        assert_eq!(updates[1].1.point(0).x, 20.0);
        assert!(total > 20);
        for limit in 0..total {
            let caller = Caller::default();
            let work = LocalRouteSearchWork::with_context(limit, &caller).unwrap();
            assert!(
                selected(
                    Some(&graph),
                    &mut initial.clone(),
                    &[Some(0), Some(1)],
                    None,
                    Some(&work),
                    &caller,
                    "Default",
                    true
                )
                .is_err(),
                "limit {limit}"
            );
            assert!(work.used() <= limit);
            assert_eq!(
                graph
                    .edge_records
                    .iter()
                    .map(|e| e.route.read().to_vec())
                    .collect::<Vec<_>>(),
                before
            );
        }
        let caller = Caller {
            cancel_at: caller.polls.get(),
            ..Caller::default()
        };
        let work = LocalRouteSearchWork::with_context(1000, &caller).unwrap();
        assert!(matches!(
            selected(
                Some(&graph),
                &mut initial.clone(),
                &[Some(0), Some(1)],
                None,
                Some(&work),
                &caller,
                "Default",
                true
            ),
            Err(Error::Guard(LayoutError::Cancelled))
        ));
        assert_eq!(work.used(), total);
        assert_eq!(
            graph
                .edge_records
                .iter()
                .map(|e| e.route.read().to_vec())
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn selected_route_validation_rejects_nil_duplicate_and_missing_before_commit() {
        let (graph, records) = fixture();
        let edges = BTreeSet::from([0]);
        for (order, expected, used) in [
            (vec![None, Some(0)], "nil route", 1),
            (vec![Some(0), Some(0)], "duplicate routes for an edge", 2),
            (vec![], "omitted a requested edge", 0),
        ] {
            let caller = Caller::default();
            let work = LocalRouteSearchWork::with_context(1000, &caller).unwrap();
            let error = selected(
                Some(&graph),
                &mut records.clone(),
                &order,
                Some(&edges),
                Some(&work),
                &caller,
                "Default",
                false,
            )
            .unwrap_err();
            assert!(error.to_string().ends_with(expected), "{error}");
            assert_eq!(work.used(), used);
        }
        let caller = Caller::default();
        let error =
            selected(None, &mut [], &[], None, None, &caller, "Default", false).unwrap_err();
        assert!(error.to_string().ends_with("no work guard"));
        let work = LocalRouteSearchWork::with_context(1000, &caller).unwrap();
        let error = selected(
            None,
            &mut [],
            &[],
            None,
            Some(&work),
            &caller,
            "Default",
            false,
        )
        .unwrap_err();
        assert!(error.to_string().ends_with("no graph"));
    }

    #[test]
    fn materialization_reads_retained_points_and_copies_go_slice_storage() {
        let live = SharedOwnerPoint::new(Point { x: 0.0, y: 100.0 });
        let route = PublishedRoute::new(
            0,
            vec![
                Point { x: -10.0, y: 0.0 },
                Point { x: 0.0, y: 0.0 },
                Point { x: 555.0, y: 555.0 },
                Point { x: 100.0, y: 100.0 },
                Point { x: 110.0, y: 100.0 },
            ],
            None,
            None,
            vec![None, None, Some(live.clone()), None, None],
        );
        let points = route.endpoint_slice();
        assert_eq!(points.len(), 3);
        assert_eq!(points.capacity(), 4);
        assert_eq!(points.point(1), live.get());
        live.set(Point { x: 2.0, y: 3.0 });
        assert_eq!(points.point(1), Point { x: 0.0, y: 100.0 });
        assert!(points.header().read_capacity_slot(3).unwrap().is_none());
    }
}

#[cfg(feature = "diagnostic-traces")]
#[path = "finalize_probe.rs"]
pub(super) mod probe;
