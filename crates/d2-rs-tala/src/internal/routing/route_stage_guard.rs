// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! routing/route_stage_guard.go: checked discovery and the mutation boundary.
use super::route_search_guard::RouteStageWork;
use crate::internal::owner_groups::OwnerGroupKind;
use crate::internal::owner_points::SharedOwnerPoint;
use crate::internal::shared_topology::OwnerNodePointSnapshot;
use crate::internal::{ArenaEdge, ArenaGraph, OwnerGraphId, limits};
use crate::{EdgeId, LabelPosition, NodeId, Point};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(super) trait RouteBudget {
    fn reserve(&self, units: u64) -> Result<(), ()>;
}
impl RouteBudget for RouteStageWork {
    fn reserve(&self, units: u64) -> Result<(), ()> {
        RouteStageWork::reserve(self, units)
    }
}
impl RouteBudget for Arc<RouteStageWork> {
    fn reserve(&self, units: u64) -> Result<(), ()> {
        self.as_ref().reserve(units)
    }
}

#[derive(Debug)]
pub(in crate::internal) enum RouteStageError {
    Validation(limits::Error),
    WorkLimit { location: &'static str, limit: u64 },
    Invariant(String),
    Kernel(crate::LayoutError),
}
impl std::fmt::Display for RouteStageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(error) => error.fmt(f),
            Self::WorkLimit { location, limit } => write!(
                f,
                "TALA route-stage work limit exceeded: TALA {location} work exceeds limit {limit}"
            ),
            Self::Invariant(reason) => write!(f, "layout invariant violated: {reason}"),
            Self::Kernel(error) => error.fmt(f),
        }
    }
}
impl From<limits::Error> for RouteStageError {
    fn from(error: limits::Error) -> Self {
        Self::Validation(error)
    }
}
impl RouteStageError {
    pub(in crate::internal) fn is_route_work_limit(&self) -> bool {
        matches!(self, Self::WorkLimit { .. })
    }
    pub(in crate::internal) fn into_layout_error(self, seed: i64) -> crate::LayoutError {
        match self {
            Self::WorkLimit { location, limit } => crate::LayoutError::WorkLimit {
                seed,
                location,
                limit,
            },
            Self::Invariant(reason) => crate::LayoutError::InvariantViolation(reason),
            Self::Kernel(error) => error,
            Self::Validation(limits::Error::Context { cause, .. }) => match cause {
                limits::ContextError::Cancelled => crate::LayoutError::Cancelled,
                limits::ContextError::DeadlineExceeded => crate::LayoutError::DeadlineExceeded,
            },
            Self::Validation(error) => crate::LayoutError::EngineValidation(error.to_string()),
        }
    }
}
fn invalid(message: String) -> RouteStageError {
    limits::Error::Invalid(message).into()
}

pub(super) struct RuntimeRouteContext;
impl limits::Context for RuntimeRouteContext {
    fn reads_cancellation_scope(&self) -> bool {
        true
    }
    fn done(&self) -> Option<bool> {
        crate::internal::cancellation_done()
    }
    fn error(&self) -> Option<limits::ContextError> {
        crate::internal::cancellation_error()
    }
}

pub(super) struct RouteAtomicGuard<'a> {
    context: &'a dyn limits::Context,
    cancellable: bool,
    location: &'static str,
    budget: &'a RouteStageWork,
    failure: RefCell<Option<RouteStageError>>,
}
impl<'a> RouteAtomicGuard<'a> {
    pub(super) fn new(
        context: Option<&'a dyn limits::Context>,
        location: &'static str,
        budget: &'a RouteStageWork,
    ) -> Result<Self, RouteStageError> {
        let context =
            context.ok_or_else(|| invalid(format!("TALA {location} requires a context")))?;
        let guard = Self {
            context,
            cancellable: context.done().is_some(),
            location,
            budget,
            failure: RefCell::new(None),
        };
        guard.finish()?;
        Ok(guard)
    }
    pub(super) fn finish(&self) -> Result<(), RouteStageError> {
        if (!self.cancellable || self.context.done() == Some(true))
            && let Some(cause) = self.context.error()
        {
            return Err(limits::Error::Context {
                location: self.location.into(),
                cause,
            }
            .into());
        }
        Ok(())
    }
    pub(super) fn add(&self, units: u64) -> Result<(), RouteStageError> {
        self.budget.reserve_with_check(
            units,
            || self.finish(),
            |limit| RouteStageError::WorkLimit {
                location: self.location,
                limit,
            },
        )
    }
    pub(super) fn step(&self) -> Result<(), RouteStageError> {
        self.add(1)
    }
    pub(super) fn context(&self) -> &dyn limits::Context {
        self.context
    }
    pub(super) fn take_kernel_failure(&self) -> Option<RouteStageError> {
        self.failure.borrow_mut().take()
    }
    fn take_failure(&self) -> RouteStageError {
        self.failure
            .borrow_mut()
            .take()
            .expect("failed kernel reservation retains its error")
    }
}
impl RouteBudget for RouteAtomicGuard<'_> {
    fn reserve(&self, units: u64) -> Result<(), ()> {
        self.add(units).map_err(|error| {
            *self.failure.borrow_mut() = Some(error);
        })
    }
}

fn validate_node(
    graph: &ArenaGraph,
    node: Option<NodeId>,
    description: &str,
    seen: &mut BTreeSet<NodeId>,
    guard: &RouteAtomicGuard<'_>,
) -> Result<NodeId, RouteStageError> {
    guard.step()?;
    let node = node.ok_or_else(|| {
        invalid(format!(
            "TALA {} contains a nil {description}",
            guard.location
        ))
    })?;
    if seen.insert(node) && graph.position(node).is_none() {
        return Err(invalid(format!(
            "TALA {} {description} has no position",
            guard.location
        )));
    }
    Ok(node)
}
fn valid_label_position(position: &str) -> bool {
    matches!(
        position,
        "" | "OUTSIDE_TOP_LEFT"
            | "OUTSIDE_TOP_CENTER"
            | "OUTSIDE_TOP_RIGHT"
            | "INSIDE_MIDDLE_LEFT"
            | "INSIDE_MIDDLE_CENTER"
            | "INSIDE_MIDDLE_RIGHT"
            | "OUTSIDE_BOTTOM_LEFT"
            | "OUTSIDE_BOTTOM_CENTER"
            | "OUTSIDE_BOTTOM_RIGHT"
            | "UNLOCKED_TOP"
            | "UNLOCKED_MIDDLE"
            | "UNLOCKED_BOTTOM"
    )
}
fn validate_geometry(
    graph: &ArenaGraph,
    receiver: OwnerGraphId,
    extra: &[Option<EdgeId>],
    guard: &RouteAtomicGuard<'_>,
) -> Result<(), RouteStageError> {
    if extra.len() > 1_000_000 {
        return Err(invalid(format!(
            "TALA {} extra edge references exceed limit 1000000",
            guard.location
        )));
    }
    let view = graph.owner_graph_view(receiver).unwrap();
    let mut seen_nodes = BTreeSet::new();
    let mut graph_nodes = BTreeSet::new();
    for (index, node) in view.nodes.iter_slots().enumerate() {
        graph_nodes.insert(validate_node(
            graph,
            node,
            &format!("graph node at index {index}"),
            &mut seen_nodes,
            guard,
        )?);
    }
    let mut seen_edges = BTreeSet::new();
    let mut capacity = 0usize;
    let mut validate_edge =
        |edge: Option<EdgeId>, require_members: bool| -> Result<(), RouteStageError> {
            guard.step()?;
            let edge = edge
                .ok_or_else(|| invalid(format!("TALA {} contains a nil edge", guard.location)))?;
            if !seen_edges.insert(edge) {
                return Ok(());
            }
            let edge = &graph.edge_records[edge.0 as usize];
            let id = edge.tala_id;
            if require_members {
                if !graph_nodes.contains(&edge.from) {
                    return Err(invalid(format!(
                        "graph edge {id} source node does not belong to the graph"
                    )));
                }
                if !graph_nodes.contains(&edge.to) {
                    return Err(invalid(format!(
                        "graph edge {id} target node does not belong to the graph"
                    )));
                }
            }
            if edge.route.capacity() > 1_000_000 - capacity {
                return Err(invalid(format!(
                    "TALA {} route point capacity exceeds limit 1000000",
                    guard.location
                )));
            }
            capacity += edge.route.capacity();
            if edge.route.len() == 1 {
                return Err(RouteStageError::Invariant(format!(
                    "edge {id} has an incomplete route: expected at least two points, got 1"
                )));
            }
            let main = edge.label.as_ref().map(|label| label.position.d2_name());
            for (name, position) in [
                ("label", main),
                (
                    "source arrowhead label",
                    edge.source_arrowhead_label.as_ref().map(|_| {
                        edge.source_label_positions[1]
                            .unwrap_or(LabelPosition::Unset)
                            .d2_name()
                    }),
                ),
                (
                    "target arrowhead label",
                    edge.target_arrowhead_label.as_ref().map(|_| {
                        edge.source_label_positions[2]
                            .unwrap_or(LabelPosition::Unset)
                            .d2_name()
                    }),
                ),
            ] {
                if position.is_some_and(|position| !valid_label_position(position)) {
                    return Err(RouteStageError::Invariant(format!(
                        "edge {id} {name} has an invalid edge position"
                    )));
                }
            }
            validate_node(
                graph,
                Some(edge.from),
                &format!("edge {id} source"),
                &mut seen_nodes,
                guard,
            )?;
            validate_node(
                graph,
                Some(edge.to),
                &format!("edge {id} target"),
                &mut seen_nodes,
                guard,
            )?;
            for index in 0..edge.route.len() {
                guard.step()?;
                let point = edge
                    .route
                    .slot(index)
                    .ok_or_else(|| {
                        invalid(format!(
                            "TALA {} edge {id} contains a nil route point",
                            guard.location
                        ))
                    })?
                    .get();
                if !point.x.is_finite() || !point.y.is_finite() {
                    return Err(invalid(format!(
                        "TALA {} edge {id} has a non-finite route point",
                        guard.location
                    )));
                }
            }
            Ok(())
        };
    let edges = view.edges.header();
    for index in 0..edges.len() {
        validate_edge(edges.get(index).unwrap(), true)?;
    }
    for node in view.nodes.iter_slots().flatten() {
        let edges = graph.nodes[node.0 as usize].edges.header().clone();
        for index in 0..edges.len() {
            validate_edge(edges.get(index).unwrap(), false)?;
        }
    }
    for &edge in extra {
        validate_edge(edge, false)?;
    }
    guard.finish()
}

struct NodeSnapshot {
    point: Option<OwnerNodePointSnapshot>,
    inline: Option<Point>,
    origin: Point,
    owner: Option<OwnerGraphId>,
}
struct EdgeSnapshot {
    value: ArenaEdge,
    points: Vec<(Option<SharedOwnerPoint>, Option<Point>)>,
}
pub(super) struct RouteMutationSnapshot {
    receiver: OwnerGraphId,
    cell_size: f64,
    costs: (f64, f64, f64),
    nodes: BTreeMap<NodeId, NodeSnapshot>,
    edges: BTreeMap<EdgeId, EdgeSnapshot>,
    queue: Vec<NodeId>,
}
impl RouteMutationSnapshot {
    fn capture_node(
        &mut self,
        graph: &ArenaGraph,
        node: Option<NodeId>,
        guard: &RouteAtomicGuard<'_>,
    ) -> Result<(), RouteStageError> {
        guard.step()?;
        let Some(node) = node else {
            return Ok(());
        };
        if self.nodes.contains_key(&node) {
            return Ok(());
        }
        self.nodes.insert(
            node,
            NodeSnapshot {
                point: graph
                    .owner_node_point(node)
                    .ok()
                    .map(|point| point.snapshot()),
                inline: graph.nodes[node.0 as usize].position,
                origin: graph.nodes[node.0 as usize].rect.origin,
                owner: graph.topology_node_owner(node),
            },
        );
        self.queue.push(node);
        Ok(())
    }
    fn capture_edge(
        &mut self,
        graph: &ArenaGraph,
        edge: Option<EdgeId>,
        guard: &RouteAtomicGuard<'_>,
    ) -> Result<(), RouteStageError> {
        guard.step()?;
        let Some(id) = edge else {
            return Ok(());
        };
        if self.edges.contains_key(&id) {
            return Ok(());
        }
        let edge = &graph.edge_records[id.0 as usize];
        let mut points = Vec::with_capacity(edge.route.capacity());
        for index in 0..edge.route.capacity() {
            guard.step()?;
            let point = edge.route.header().read_capacity_slot(index).unwrap();
            let value = point.as_ref().map(SharedOwnerPoint::get);
            points.push((point, value));
        }
        self.edges.insert(
            id,
            EdgeSnapshot {
                value: edge.clone(),
                points,
            },
        );
        self.capture_node(graph, Some(edge.from), guard)?;
        self.capture_node(graph, Some(edge.to), guard)
    }
    pub(super) fn capture(
        graph: &ArenaGraph,
        receiver: OwnerGraphId,
        extra: &[Option<EdgeId>],
        guard: &RouteAtomicGuard<'_>,
    ) -> Result<Self, RouteStageError> {
        let mut snapshot = Self {
            receiver,
            cell_size: graph.owner_graph_cell_size(receiver).unwrap(),
            costs: graph.owner_graph_routing_costs(receiver).unwrap(),
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
            queue: Vec::new(),
        };
        let view = graph.owner_graph_view(receiver).unwrap();
        let edges = view.edges.header();
        for index in 0..edges.len() {
            snapshot.capture_edge(graph, edges.get(index).unwrap(), guard)?;
        }
        for node in view.nodes.iter_slots() {
            snapshot.capture_node(graph, node, guard)?;
            if let Some(node) = node {
                let edges = graph.nodes[node.0 as usize].edges.header().clone();
                for index in 0..edges.len() {
                    snapshot.capture_edge(graph, edges.get(index).unwrap(), guard)?;
                }
            }
        }
        for &edge in extra {
            snapshot.capture_edge(graph, edge, guard)?;
        }
        let mut index = 0;
        while index < snapshot.queue.len() {
            let node = snapshot.queue[index];
            index += 1;
            let owner = graph.topology_node_owner(node).unwrap_or(receiver);
            if graph.nodes[node.0 as usize].is_container {
                if let Some(children) = graph.containers.get_header(&Some(node)) {
                    for child in children.iter_slots() {
                        snapshot.capture_node(graph, child, guard)?;
                    }
                }
            }
            for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
                if kind == OwnerGroupKind::Cluster && !graph.owner_is_cluster_vessel(node) {
                    continue;
                }
                if let Some(group) = graph
                    .owner_graph_group_entry(owner, kind, Some(node))
                    .unwrap()
                    .flatten()
                {
                    for child in graph.owner_group_fields(group).unwrap().nodes.iter_slots() {
                        snapshot.capture_node(graph, child, guard)?;
                    }
                }
            }
        }
        guard.finish()?;
        Ok(snapshot)
    }
    pub(super) fn restore(self, graph: &mut ArenaGraph) {
        graph
            .set_owner_graph_cell_size(self.receiver, self.cell_size)
            .unwrap();
        graph
            .restore_owner_graph_routing_costs(self.receiver, self.costs)
            .unwrap();
        for (node, snapshot) in self.nodes {
            if let Some(point) = snapshot.point {
                graph.restore_owner_node_point(node, &point).unwrap();
            } else {
                graph.release_materialized_owner_node_point(node);
            }
            graph.nodes[node.0 as usize].position = snapshot.inline;
            graph.nodes[node.0 as usize].rect.origin = snapshot.origin;
            graph
                .set_owner_node_graph_reference(node, snapshot.owner)
                .unwrap();
        }
        for (edge, snapshot) in self.edges {
            for (index, (point, value)) in snapshot.points.into_iter().enumerate() {
                if let (Some(point), Some(value)) = (&point, value) {
                    point.set(value);
                }
                snapshot
                    .value
                    .route
                    .header()
                    .write_capacity_slot(index, point)
                    .unwrap();
            }
            graph.edge_records[edge.0 as usize].restore_value(snapshot.value);
        }
    }
}

pub(super) fn run_atomic_route_stage(
    graph: &mut ArenaGraph,
    receiver: Option<OwnerGraphId>,
    context: Option<&dyn limits::Context>,
    location: &'static str,
    extra: &[Option<EdgeId>],
    budget: &RouteStageWork,
    operation: impl FnOnce(&mut ArenaGraph, &RouteAtomicGuard<'_>) -> Result<(), ()>,
) -> Result<(), RouteStageError> {
    run_atomic_route_stage_typed(
        graph,
        receiver,
        context,
        location,
        extra,
        budget,
        |graph, guard| operation(graph, guard).map_err(|()| guard.take_failure()),
    )
}

pub(super) fn run_atomic_route_stage_typed(
    graph: &mut ArenaGraph,
    receiver: Option<OwnerGraphId>,
    context: Option<&dyn limits::Context>,
    location: &'static str,
    extra: &[Option<EdgeId>],
    budget: &RouteStageWork,
    operation: impl FnOnce(&mut ArenaGraph, &RouteAtomicGuard<'_>) -> Result<(), RouteStageError>,
) -> Result<(), RouteStageError> {
    graph.validate_owner_graph_topology(receiver, context, location)?;
    run_atomic_route_stage_after_preflight(
        graph,
        receiver.unwrap(),
        context,
        location,
        extra,
        budget,
        operation,
    )
}

/// standalone_router.go performs topology validation before selection checks.
/// Enter its atomic geometry/snapshot boundary without validating twice.
pub(super) fn run_atomic_route_stage_after_preflight(
    graph: &mut ArenaGraph,
    receiver: OwnerGraphId,
    context: Option<&dyn limits::Context>,
    location: &'static str,
    extra: &[Option<EdgeId>],
    budget: &RouteStageWork,
    operation: impl FnOnce(&mut ArenaGraph, &RouteAtomicGuard<'_>) -> Result<(), RouteStageError>,
) -> Result<(), RouteStageError> {
    let guard = RouteAtomicGuard::new(context, location, budget)?;
    validate_geometry(graph, receiver, extra, &guard)?;
    let snapshot = RouteMutationSnapshot::capture(graph, receiver, extra, &guard)?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        operation(graph, &guard)?;
        guard.finish()
    }));
    if !matches!(&result, Ok(Ok(()))) {
        snapshot.restore(graph);
    }
    match result {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// nudge_channels.go::channelInventoryIsClosed. Discovery uses the same live
/// reference walk and per-capacity-slot charges as the atomic stage snapshot.
pub(super) fn capture_inventory_is_closed(
    graph: &ArenaGraph,
    receiver: OwnerGraphId,
    guard: &RouteAtomicGuard<'_>,
) -> Result<bool, RouteStageError> {
    let snapshot = RouteMutationSnapshot::capture(graph, receiver, &[], guard)?;
    snapshot.inventory_is_closed(graph, guard)
}

impl RouteMutationSnapshot {
    pub(super) fn inventory_is_closed(
        &self,
        graph: &ArenaGraph,
        guard: &RouteAtomicGuard<'_>,
    ) -> Result<bool, RouteStageError> {
        let receiver = self.receiver;
        let view = graph.owner_graph_view(receiver).unwrap();
        let mut nodes = BTreeSet::new();
        let mut edges = BTreeSet::new();
        for node in view.nodes.iter_slots() {
            guard.step()?;
            nodes.insert(node);
        }
        for edge in view.edges.header().iter_slots() {
            guard.step()?;
            edges.insert(edge);
        }
        for node in self.nodes.keys() {
            guard.step()?;
            if !nodes.contains(&Some(*node)) {
                return Ok(false);
            }
        }
        for edge in self.edges.keys() {
            guard.step()?;
            if !edges.contains(&Some(*edge)) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, Size};
    use std::cell::Cell;
    struct Context {
        polls: Cell<usize>,
        cancel: Cell<bool>,
    }
    impl limits::Context for Context {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<limits::ContextError> {
            self.polls.set(self.polls.get() + 1);
            self.cancel.get().then_some(limits::ContextError::Cancelled)
        }
    }
    #[test]
    fn unsigned_route_work_polls_only_at_source_boundaries() {
        let context = Context {
            polls: Cell::new(0),
            cancel: Cell::new(false),
        };
        let budget = RouteStageWork::new(u64::MAX);
        let guard = RouteAtomicGuard::new(Some(&context), "SimplifyEdgeRoutes", &budget).unwrap();
        guard.add(1023).unwrap();
        assert_eq!(context.polls.get(), 1);
        context.cancel.set(true);
        assert_eq!(
            guard.step().unwrap_err().to_string(),
            "SimplifyEdgeRoutes: context canceled"
        );
        assert_eq!(budget.used(), 1024);
        assert_eq!(context.polls.get(), 2);
        assert!(guard.add(0).is_err());
        assert_eq!(budget.used(), 1024);
        assert!(guard.add(u64::MAX).is_err());
        assert_eq!(budget.used(), 1024);
        assert!(!budget.exhausted()); // cancellation wins over rejected work
        context.cancel.set(false);
        assert!(matches!(
            guard.add(u64::MAX),
            Err(RouteStageError::WorkLimit { .. })
        ));
        assert!(budget.exhausted());
        assert_eq!(budget.used(), 1024);
    }
    fn fixture() -> ArenaGraph {
        let mut input = Graph::default();
        let a = input.add_node(Node::new(
            "a",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let b = input.add_node(Node::new(
            "b",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.add_edge(Edge {
            source: a,
            target: b,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(a, Point { x: 0.0, y: 0.0 });
        graph.set_position(b, Point { x: 50.0, y: 50.0 });
        graph.edge_records[0]
            .route
            .replace(crate::internal::RoutePoints::make(
                &[
                    Point { x: 10.0, y: 5.0 },
                    Point { x: 55.0, y: 5.0 },
                    Point { x: 55.0, y: 50.0 },
                ],
                4,
            ));
        graph
    }
    #[test]
    fn atomic_failure_restores_aliases_geometry_ownership_and_costs() {
        for failure in ["limit", "finish", "panic"] {
            let mut graph = fixture();
            let original_route = graph.edge_records[0].route.clone();
            // Spare pointer capacity participates in capture and restoration.
            let spare = SharedOwnerPoint::new(Point { x: 88.0, y: 99.0 });
            original_route
                .header()
                .write_capacity_slot(3, Some(spare.clone()))
                .unwrap();
            graph.materialize_owner_node_point(NodeId(0)).unwrap();
            let node_point = graph.owner_node_point(NodeId(0)).unwrap();
            let original_owner = graph.topology_node_owner(NodeId(0));
            let original_costs = graph.raw_routing_costs();
            let original_cell = graph.cell_size;
            let context = Context {
                polls: Cell::new(0),
                cancel: Cell::new(false),
            };
            let budget = RouteStageWork::new(100_000);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_atomic_route_stage(
                    &mut graph,
                    Some(OwnerGraphId(0)),
                    Some(&context),
                    "SimplifyEdgeRoutes",
                    &[],
                    &budget,
                    |graph, guard| {
                        original_route.edit_point(0, |point| point.x = -500.0);
                        spare.set(Point {
                            x: -800.0,
                            y: -900.0,
                        });
                        original_route
                            .header()
                            .write_capacity_slot(3, None)
                            .unwrap();
                        graph.edge_records[0]
                            .route
                            .replace(crate::internal::RoutePoints::nil());
                        graph.edge_records[0].tala_id = 700;
                        graph
                            .write_owner_point(&node_point, Point { x: 300.0, y: 400.0 })
                            .unwrap();
                        graph
                            .set_owner_node_graph_reference(NodeId(0), None)
                            .unwrap();
                        graph.cell_size = 500.0;
                        graph.restore_routing_costs((100.0, 200.0, 300.0));
                        match failure {
                            "limit" => guard.reserve(u64::MAX),
                            "finish" => {
                                context.cancel.set(true);
                                Ok(())
                            }
                            _ => panic!("route operation panic"),
                        }
                    },
                )
            }));
            if failure == "panic" {
                assert!(result.is_err());
            } else {
                assert!(result.unwrap().is_err());
            }
            assert!(
                graph.edge_records[0]
                    .route
                    .slot(0)
                    .unwrap()
                    .same_allocation(&original_route.slot(0).unwrap())
            );
            assert_eq!(original_route.point(0), Point { x: 10.0, y: 5.0 });
            assert_eq!(spare.get(), Point { x: 88.0, y: 99.0 });
            assert!(
                original_route
                    .header()
                    .read_capacity_slot(3)
                    .flatten()
                    .unwrap()
                    .same_allocation(&spare)
            );
            assert!(node_point.same_allocation(&graph.owner_node_point(NodeId(0)).unwrap()));
            assert_eq!(node_point.get(), Some(Point { x: 0.0, y: 0.0 }));
            assert_eq!(graph.topology_node_owner(NodeId(0)), original_owner);
            assert_eq!(graph.cell_size, original_cell);
            assert_eq!(graph.raw_routing_costs(), original_costs);
            assert_ne!(graph.edge_records[0].tala_id, 700);
        }
    }
}
