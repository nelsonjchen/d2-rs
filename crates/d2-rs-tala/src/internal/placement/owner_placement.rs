// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Source optimization_guard.go73–92 and sizeless_optimizer.go279–400.
//! These helpers operate on the real owner, retained members and occupied map.

use super::limits::{Context, OptimizationError, OptimizationWorkGuard};
use super::owner_sizeless_constructor::OwnerSizelessOptimizer;
#[cfg(test)]
use super::shared_topology::TopologyVessel;
use super::{ArenaGraph, NodeId, OwnerGraphId, OwnerTopologyError, Point};
use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub(super) struct OwnerPlacementPoint(Rc<Cell<Point>>);

impl OwnerPlacementPoint {
    pub(super) fn new(point: Point) -> Self {
        Self(Rc::new(Cell::new(point)))
    }

    pub(super) fn get(&self) -> Point {
        self.0.get()
    }

    pub(super) fn set(&self, point: Point) {
        self.0.set(point);
    }

    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum OwnerPlacementError {
    Optimization(OptimizationError),
    Topology(OwnerTopologyError),
    UnboundDebugId(NodeId),
}

impl From<OptimizationError> for OwnerPlacementError {
    fn from(value: OptimizationError) -> Self {
        Self::Optimization(value)
    }
}

// Go optimizerMedianToNeighbors returns guard errors without replacing their causes.
impl From<OwnerPlacementError> for OptimizationError {
    fn from(value: OwnerPlacementError) -> Self {
        match value {
            OwnerPlacementError::Optimization(error) => error,
            other => Self::Invalid(other.to_string()),
        }
    }
}

impl From<OwnerTopologyError> for OwnerPlacementError {
    fn from(value: OwnerTopologyError) -> Self {
        Self::Topology(value)
    }
}

impl fmt::Display for OwnerPlacementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Optimization(error) => error.fmt(f),
            Self::Topology(error) => write!(f, "unbound owner topology: {error:?}"),
            Self::UnboundDebugId(node) => write!(f, "unbound source node DebugID: {node:?}"),
        }
    }
}

fn invalid(guard: &OptimizationWorkGuard<'_>, message: &str) -> OwnerPlacementError {
    OptimizationError::Invalid(format!("TALA {} {message}", guard.location())).into()
}

fn step(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    return guard.step_with_owner(owner);
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.step()
    }
}

fn node_graph(
    owner: &ArenaGraph,
    node: NodeId,
) -> Result<Option<OwnerGraphId>, OwnerTopologyError> {
    owner
        .topology_node(node)
        .ok_or(OwnerTopologyError::UnknownNode(node))?;
    Ok(owner.topology_node_owner(node))
}

/// node.go2344–2352: active group pointers precede the direct container.
/// A nil Vessel is dereferenced by isActive; ancestry's nil terminator is a
/// different operation and must not turn this dereference into a fallback.
pub(super) fn owner_owning_container(
    owner: &ArenaGraph,
    node: NodeId,
) -> Result<Option<NodeId>, OwnerTopologyError> {
    let live = owner
        .topology_node(node)
        .ok_or(OwnerTopologyError::UnknownNode(node))?;
    for kind in [
        super::owner_groups::OwnerGroupKind::Cluster,
        super::owner_groups::OwnerGroupKind::Sequence,
    ] {
        let Some(reference) = owner.owner_node_group_vessel(node, kind)? else {
            continue;
        };
        let vessel_node =
            reference.expect("runtime error: invalid memory address or nil pointer dereference");
        if node_graph(owner, vessel_node)?.is_some() {
            return Ok(owner
                .topology_node(vessel_node)
                .ok_or(OwnerTopologyError::UnknownNode(vessel_node))?
                .container);
        }
    }
    Ok(live.container)
}

pub(super) fn optimizer_owner_fixed_origin(
    owner: &mut ArenaGraph,
    receiver: Option<OwnerGraphId>,
    container: Option<NodeId>,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<Option<Point>, OwnerPlacementError> {
    let receiver =
        receiver.ok_or_else(|| invalid(guard, "fixed-origin lookup requires a graph"))?;
    let members = owner.owner_graph_member_header(receiver)?;
    if members.len() > 10_000 {
        return Err(invalid(
            guard,
            "fixed-origin node count exceeds limit 10000",
        ));
    }
    for index in 0..members.len() {
        let node = members.get(index).expect("retained member slot");
        step(owner, guard)?;
        let Some(node) = node else { continue };
        if owner_owning_container(owner, node)? != container {
            continue;
        }
        let live = owner
            .topology_node(node)
            .ok_or(OwnerTopologyError::UnknownNode(node))?;
        if let (Some(position), Some(fixed)) = (owner.position(node), live.fixed_top_left) {
            return Ok(Some(Point {
                x: position.x - fixed.x,
                y: position.y - fixed.y,
            }));
        }
    }
    Ok(None)
}

fn rounded_fixed_origin(
    owner: &mut ArenaGraph,
    node: NodeId,
    receiver: OwnerGraphId,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<Option<Point>, OwnerPlacementError> {
    let container = owner_owning_container(owner, node)?;
    let origin = optimizer_owner_fixed_origin(owner, Some(receiver), container, guard)?;
    let Some(origin) = origin else {
        return Ok(None);
    };
    // node.Graph is read again after fixed-origin's callbacks. It can differ
    // from the graph pointer passed into that lookup.
    let current = node_graph(owner, node)?
        .expect("runtime error: invalid memory address or nil pointer dereference");
    let cell = owner.owner_graph_cell_size(current)?;
    if cell <= 0.0 || !cell.is_finite() {
        return Err(invalid(
            guard,
            "fixed-origin placement requires a finite positive cell size",
        ));
    }
    Ok(Some(Point {
        x: (origin.x / cell).round(),
        y: (origin.y / cell).round(),
    }))
}

fn past_origin(origin: Option<Point>, point: Point) -> bool {
    origin.is_some_and(|origin| point.x < origin.x || point.y < origin.y)
}

// fmt's default float verb uses shortest %g, with exponent notation below
// -4 or at/above 6. The scientific mantissa is generated without rounding
// through a decimal/logarithm intermediate.
pub(super) fn go_general_float(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value == f64::INFINITY {
        return "+Inf".into();
    }
    if value == f64::NEG_INFINITY {
        return "-Inf".into();
    }
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("scientific exponent");
    let exponent: i32 = exponent.parse().expect("decimal exponent");
    if !(-4..6).contains(&exponent) {
        format!("{mantissa}e{exponent:+03}")
    } else {
        value.to_string()
    }
}

impl OwnerSizelessOptimizer {
    pub(super) fn find_closest_unoccupied_distance(
        &self,
        owner: &mut ArenaGraph,
        context: Option<&dyn Context>,
        node: Option<NodeId>,
        median: Option<&OwnerPlacementPoint>,
    ) -> Result<f64, OwnerPlacementError> {
        #[cfg(feature = "diagnostic-traces")]
        let mut guard = OptimizationWorkGuard::new_with_owner(
            context,
            "sizelessOptimizer.optimize",
            super::limits::MAX_OPTIMIZATION_WORK_UNITS,
            owner,
        )?;
        #[cfg(not(feature = "diagnostic-traces"))]
        let mut guard = OptimizationWorkGuard::new(
            context,
            "sizelessOptimizer.optimize",
            super::limits::MAX_OPTIMIZATION_WORK_UNITS,
        )?;
        self.find_closest_unoccupied_distance_guarded(owner, node, median, &mut guard)
    }

    pub(super) fn find_closest_unoccupied_distance_guarded(
        &self,
        owner: &mut ArenaGraph,
        node: Option<NodeId>,
        median: Option<&OwnerPlacementPoint>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<f64, OwnerPlacementError> {
        let (Some(node), Some(median)) = (node, median) else {
            return Err(invalid(
                guard,
                "placement search requires a node, graph, and median",
            ));
        };
        let receiver = node_graph(owner, node)?
            .ok_or_else(|| invalid(guard, "placement search requires a node, graph, and median"))?;
        let origin = rounded_fixed_origin(owner, node, receiver, guard)?;
        for distance in 0..=100 {
            step(owner, guard)?;
            let mut x = distance as f64;
            let mut y = 0.0;
            while x >= -(distance as f64) {
                step(owner, guard)?;
                if y != 0.0 {
                    let median = median.get();
                    let point = Point {
                        x: median.x + x,
                        y: median.y - y,
                    };
                    if !past_origin(origin, point)
                        && self.occupied().borrow().lookup(point).is_none()
                    {
                        return Ok(distance as f64);
                    }
                }
                let median = median.get();
                let point = Point {
                    x: median.x + x,
                    y: median.y + y,
                };
                if !past_origin(origin, point) && self.occupied().borrow().lookup(point).is_none() {
                    return Ok(distance as f64);
                }
                if x > 0.0 {
                    y += 1.0
                } else {
                    y -= 1.0
                }
                x -= 1.0;
            }
        }
        let point = median.get();
        Err(OptimizationError::Invalid(format!(
            "no unoccupied points within d=100 of p=&{{{} {}}}",
            go_general_float(point.x),
            go_general_float(point.y)
        ))
        .into())
    }

    pub(super) fn placement_points_guarded(
        &self,
        owner: &mut ArenaGraph,
        node: Option<NodeId>,
        median: Option<&OwnerPlacementPoint>,
        minimum_distance: f64,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<Vec<OwnerPlacementPoint>, OwnerPlacementError> {
        let (Some(node), Some(median)) = (node, median) else {
            return Err(invalid(
                guard,
                "placement generation requires a node, graph, and median",
            ));
        };
        let receiver = node_graph(owner, node)?.ok_or_else(|| {
            invalid(
                guard,
                "placement generation requires a node, graph, and median",
            )
        })?;
        if !(0.0..=100.0).contains(&minimum_distance) || !minimum_distance.is_finite() {
            return Err(invalid(
                guard,
                "placement distance must be finite and within [0, 100]",
            ));
        }
        let distance = minimum_distance + 1.0;
        let capacity =
            (distance * distance + (distance + 1.0) * (distance + 1.0)).ceil() as usize + 1;
        // The source's million-candidate cap is unreachable after the same
        // finite [0,100] precheck: the largest capacity is 20,606.
        let mut points = Vec::with_capacity(capacity);
        let origin = rounded_fixed_origin(owner, node, receiver, guard)?;
        let mut x = distance;
        while x >= -distance {
            step(owner, guard)?;
            let mut y = 0.0;
            while y <= distance - x.abs() {
                step(owner, guard)?;
                if y != 0.0 {
                    let median = median.get();
                    let point = Point {
                        x: median.x + x,
                        y: median.y - y,
                    };
                    if !past_origin(origin, point)
                        && self.occupied().borrow().lookup(point).is_none()
                    {
                        points.push(OwnerPlacementPoint::new(point));
                    }
                }
                let median = median.get();
                let point = Point {
                    x: median.x + x,
                    y: median.y + y,
                };
                if !past_origin(origin, point) && self.occupied().borrow().lookup(point).is_none() {
                    points.push(OwnerPlacementPoint::new(point));
                }
                y += 1.0;
            }
            x -= 1.0;
        }
        Ok(points)
    }
}

#[cfg(test)]
mod tests {
    use super::super::limits::ContextError;
    use super::super::occupied::OccupiedPointMap;
    use super::super::owner_near_eligibility::optimizer_owner_is_descendant_of;
    use super::*;
    use crate::{Edge, Graph, Node, Size};

    struct Quiet;
    impl Context for Quiet {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            None
        }
    }

    fn fixture() -> (ArenaGraph, OwnerSizelessOptimizer, [NodeId; 2]) {
        let mut input = Graph::default();
        let nodes = ["A", "B"].map(|id| {
            input.add_node(Node::new(
                id,
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ))
        });
        input.add_edge(Edge {
            source: nodes[0],
            target: nodes[1],
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.cell_size = 10.0;
        owner.set_position(nodes[0], Point::default());
        owner.set_position(nodes[1], Point { x: 10.0, y: 0.0 });
        owner.initialize_shared_topology();
        let optimizer =
            OwnerSizelessOptimizer::new(&mut owner, Some(OwnerGraphId(0)), None, Some(&Quiet))
                .unwrap();
        *optimizer.occupied().borrow_mut() = OccupiedPointMap::new();
        (owner, optimizer, nodes)
    }

    #[test]
    fn owner_placement_actual_source_controls() {
        for name in [
            "search-empty",
            "search-occupied",
            "points-integer",
            "points-fractional",
            "search-foreign-fixed",
            "points-limit",
            "search-exhausted",
            "search-exhausted-small",
            "search-exhausted-large",
            "search-exhausted-nan",
        ] {
            let (mut owner, optimizer, [node, fixed]) = fixture();
            let median = OwnerPlacementPoint::new(Point::default());
            let mut minimum = 0.0;
            let mut budget = 100_000;
            if matches!(name, "search-occupied" | "points-integer") {
                optimizer.occupied().borrow().insert(median.get(), None);
            }
            if name == "points-fractional" {
                minimum = 0.5
            }
            if name == "points-limit" {
                budget = 7
            }
            if name == "search-foreign-fixed" {
                let foreign = owner.create_owner_graph_view(vec![fixed], vec![]).unwrap();
                owner.set_owner_graph_cell_size(foreign, 20.0).unwrap();
                owner
                    .set_owner_node_graph_reference(node, Some(foreign))
                    .unwrap();
                owner.nodes[fixed.0 as usize].fixed_top_left = Some(Point { x: 0.0, y: -20.0 });
            }
            if name.starts_with("search-exhausted") {
                owner.nodes[fixed.0 as usize].fixed_top_left = Some(Point { x: 0.0, y: -1e30 });
                let x = match name {
                    "search-exhausted-small" => 1e-7,
                    "search-exhausted-large" => 1e6,
                    "search-exhausted-nan" => f64::from_bits(0x7ff8000000000001),
                    _ => 0.0,
                };
                median.set(Point { x, y: 0.0 });
            }
            let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "placement", budget).unwrap();
            let mut distance: Option<String> = None;
            let mut point_bits: Option<Vec<[String; 2]>> = None;
            let mut capacity = 0;
            let mut distinct = true;
            let error = if name.starts_with("points-") {
                match optimizer.placement_points_guarded(
                    &mut owner,
                    Some(node),
                    Some(&median),
                    minimum,
                    &mut guard,
                ) {
                    Ok(points) => {
                        capacity = points.capacity();
                        point_bits = Some(
                            points
                                .iter()
                                .map(|p| {
                                    let p = p.get();
                                    [
                                        format!("{:016x}", p.x.to_bits()),
                                        format!("{:016x}", p.y.to_bits()),
                                    ]
                                })
                                .collect(),
                        );
                        for (index, point) in points.iter().enumerate() {
                            distinct &= !point.same_allocation(&median)
                                && points[..index]
                                    .iter()
                                    .all(|prior| !point.same_allocation(prior));
                        }
                        None
                    }
                    Err(error) => Some(error.to_string()),
                }
            } else {
                match optimizer.find_closest_unoccupied_distance_guarded(
                    &mut owner,
                    Some(node),
                    Some(&median),
                    &mut guard,
                ) {
                    Ok(result) => {
                        distance = Some(format!("{:016x}", result.to_bits()));
                        None
                    }
                    Err(error) => Some(error.to_string()),
                }
            };
            println!(
                "OWNER_PLACEMENT_CONTROL {}",
                serde_json::json!({"case": name, "distance_bits": distance, "point_bits": point_bits, "used": guard.used(), "error": error, "capacity": capacity, "allocation_distinct": distinct})
            );
        }
    }

    #[test]
    fn guarded_search_and_points_preserve_diamond_order_work_and_nil_occupied_values() {
        let (mut owner, optimizer, [node, _]) = fixture();
        let median = OwnerPlacementPoint::new(Point::default());
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "placement", 100).unwrap();
        assert_eq!(
            optimizer
                .find_closest_unoccupied_distance_guarded(
                    &mut owner,
                    Some(node),
                    Some(&median),
                    &mut guard
                )
                .unwrap(),
            0.0
        );
        assert_eq!(guard.used(), 4);
        optimizer.occupied().borrow().insert(Point::default(), None);
        let points = optimizer
            .placement_points_guarded(&mut owner, Some(node), Some(&median), 0.0, &mut guard)
            .unwrap();
        assert_eq!(
            points
                .iter()
                .map(OwnerPlacementPoint::get)
                .collect::<Vec<_>>(),
            vec![
                Point { x: 1.0, y: 0.0 },
                Point { x: 0.0, y: -1.0 },
                Point { x: 0.0, y: 1.0 },
                Point { x: -1.0, y: 0.0 }
            ]
        );
        assert_eq!(guard.used(), 13);
        assert!(!points[0].same_allocation(&points[1]));
        let alias = points[0].clone();
        alias.set(Point { x: 99.0, y: 88.0 });
        assert_eq!(points[0].get(), alias.get());
        assert_eq!(points[1].get(), Point { x: 0.0, y: -1.0 });
    }

    #[test]
    fn guarded_points_keep_fractional_distance_and_stop_at_the_first_rejected_charge() {
        let (mut owner, optimizer, [node, _]) = fixture();
        let median = OwnerPlacementPoint::new(Point::default());
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "placement", 100).unwrap();
        let points = optimizer
            .placement_points_guarded(&mut owner, Some(node), Some(&median), 0.5, &mut guard)
            .unwrap();
        assert_eq!(points.capacity(), 10);
        assert_eq!(
            points
                .iter()
                .map(OwnerPlacementPoint::get)
                .collect::<Vec<_>>(),
            vec![
                Point { x: 1.5, y: 0.0 },
                Point { x: 0.5, y: 0.0 },
                Point { x: 0.5, y: -1.0 },
                Point { x: 0.5, y: 1.0 },
                Point { x: -0.5, y: 0.0 },
                Point { x: -0.5, y: -1.0 },
                Point { x: -0.5, y: 1.0 },
                Point { x: -1.5, y: 0.0 }
            ]
        );
        assert_eq!(guard.used(), 12);
        let mut limited = OptimizationWorkGuard::new(Some(&Quiet), "placement", 7).unwrap();
        let error = optimizer
            .placement_points_guarded(&mut owner, Some(node), Some(&median), 0.0, &mut limited)
            .unwrap_err();
        assert!(matches!(
            error,
            OwnerPlacementError::Optimization(OptimizationError::ResourceLimit { .. })
        ));
        assert_eq!(limited.used(), 7);
        assert_eq!(optimizer.occupied().borrow().len(), 0);
    }

    #[test]
    fn guarded_fixed_origin_and_search_use_foreign_members_and_live_node_cell() {
        let (mut owner, optimizer, [node, fixed]) = fixture();
        owner.nodes[fixed.0 as usize].fixed_top_left = Some(Point { x: 0.0, y: -20.0 });
        let foreign = owner.create_owner_graph_view(vec![fixed], vec![]).unwrap();
        owner.set_owner_graph_cell_size(foreign, 20.0).unwrap();
        owner
            .set_owner_node_graph_reference(node, Some(foreign))
            .unwrap();
        let median = OwnerPlacementPoint::new(Point::default());
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "placement", 100).unwrap();
        assert_eq!(
            optimizer
                .find_closest_unoccupied_distance_guarded(
                    &mut owner,
                    Some(node),
                    Some(&median),
                    &mut guard
                )
                .unwrap(),
            2.0
        );
        // The actual Go control charges one foreign member plus nine search
        // iterations, independently of the optimizer's two root members.
        assert_eq!(guard.used(), 10);
        let points = optimizer
            .placement_points_guarded(&mut owner, Some(node), Some(&median), 0.0, &mut guard)
            .unwrap();
        assert!(points.is_empty());
        owner.nodes[fixed.0 as usize].fixed_top_left = None;
        owner.set_owner_graph_cell_size(foreign, f64::NAN).unwrap();
        assert_eq!(
            optimizer
                .find_closest_unoccupied_distance_guarded(
                    &mut owner,
                    Some(node),
                    Some(&median),
                    &mut guard
                )
                .unwrap(),
            0.0
        );
    }

    #[test]
    fn guarded_placement_validation_precedes_work_but_public_search_polls_context_first() {
        let (mut owner, optimizer, [node, _]) = fixture();
        let median = OwnerPlacementPoint::new(Point::default());
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "placement", 100).unwrap();
        for distance in [-1.0, 100.1, f64::NAN, f64::INFINITY] {
            assert_eq!(
                optimizer
                    .placement_points_guarded(
                        &mut owner,
                        Some(node),
                        Some(&median),
                        distance,
                        &mut guard
                    )
                    .unwrap_err()
                    .to_string(),
                "TALA placement placement distance must be finite and within [0, 100]"
            );
        }
        assert_eq!(guard.used(), 0);
        assert_eq!(
            optimizer
                .placement_points_guarded(&mut owner, None, Some(&median), -1.0, &mut guard)
                .unwrap_err()
                .to_string(),
            "TALA placement placement generation requires a node, graph, and median"
        );
        assert_eq!(
            optimizer
                .find_closest_unoccupied_distance(&mut owner, None, None, None)
                .unwrap_err()
                .to_string(),
            "TALA sizelessOptimizer.optimize requires a context"
        );
        owner.set_owner_node_graph_reference(node, None).unwrap();
        assert_eq!(
            optimizer
                .find_closest_unoccupied_distance_guarded(
                    &mut owner,
                    Some(node),
                    Some(&median),
                    &mut guard
                )
                .unwrap_err()
                .to_string(),
            "TALA placement placement search requires a node, graph, and median"
        );
        assert_eq!(guard.used(), 0);
    }

    #[test]
    fn nullable_group_vessel_terminates_ancestry_but_panics_in_active_container_lookup() {
        let (mut owner, _, [node, other]) = fixture();
        owner.sequences.push(super::super::SequenceState {
            members: vec![node],
            vessel_tala_id: -1,
            container: None,
            has_edge_abductions: false,
        });
        owner.nodes[node.0 as usize].sequence = Some(0);
        let vessel = TopologyVessel::Sequence(0);
        assert_eq!(owner.topology_vessel_reference(vessel), None);
        owner.set_topology_vessel_reference(vessel, None).unwrap();
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "ancestry", 100).unwrap();
        assert!(
            optimizer_owner_is_descendant_of(&mut owner, Some(node), None, &mut guard).unwrap()
        );
        assert_eq!(guard.used(), 2);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| owner_owning_container(
                &owner, node
            )))
            .is_err()
        );
        let snapshot = owner.owner_topology_snapshot();
        owner
            .set_topology_vessel_reference(vessel, Some(other))
            .unwrap();
        owner.nodes[other.0 as usize].container = Some(node);
        assert_eq!(owner_owning_container(&owner, node).unwrap(), Some(node));
        owner.set_owner_node_graph_reference(other, None).unwrap();
        assert_eq!(owner_owning_container(&owner, node).unwrap(), None);
        owner.restore_owner_topology_snapshot(snapshot);
        assert_eq!(owner.topology_vessel_reference(vessel), Some(None));
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn fixed_origin_retains_header_and_loaded_node_across_actual_owner_poll() {
        struct Replace {
            polls: Cell<usize>,
            receiver: OwnerGraphId,
            first: NodeId,
            second: NodeId,
        }
        impl Context for Replace {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                None
            }
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
                self.polls.set(self.polls.get() + 1);
                if self.polls.get() == 2 {
                    let retained = owner.owner_graph_member_header(self.receiver).unwrap();
                    owner
                        .write_owner_member_capacity_slot(&retained, 0, Some(self.second))
                        .unwrap();
                    let replacement = owner
                        .allocate_owner_member_header(&[Some(self.second)], 1)
                        .unwrap();
                    owner
                        .set_owner_graph_member_header(self.receiver, replacement)
                        .unwrap();
                    owner.nodes[self.first.0 as usize].fixed_top_left =
                        Some(Point { x: -12.0, y: -13.0 });
                }
                None
            }
        }
        let (mut owner, _, [first, second]) = fixture();
        let context = Replace {
            polls: Cell::new(0),
            receiver: OwnerGraphId(0),
            first,
            second,
        };
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&context), "origin", 100, &mut owner)
                .unwrap();
        guard.add_with_owner(63, &mut owner).unwrap();
        assert_eq!(
            optimizer_owner_fixed_origin(&mut owner, Some(OwnerGraphId(0)), None, &mut guard)
                .unwrap(),
            Some(Point { x: 12.0, y: 13.0 })
        );
        assert_eq!((guard.used(), context.polls.get()), (64, 2));
        assert_eq!(
            owner
                .owner_graph_member_header(OwnerGraphId(0))
                .unwrap()
                .visible_slots(),
            vec![Some(second)]
        );
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn placement_reads_mutated_median_and_occupied_map_at_actual_poll_and_returns_cancellation() {
        struct Mutate<'a> {
            polls: Cell<usize>,
            median: OwnerPlacementPoint,
            optimizer: &'a OwnerSizelessOptimizer,
            cancel: bool,
        }
        impl Context for Mutate<'_> {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                None
            }
            fn error_with_owner(&self, _: &mut ArenaGraph) -> Option<ContextError> {
                self.polls.set(self.polls.get() + 1);
                if self.polls.get() == 2 {
                    self.median.set(Point { x: 25.0, y: 30.0 });
                    self.optimizer
                        .occupied()
                        .borrow()
                        .insert(self.median.get(), None);
                    if self.cancel {
                        return Some(ContextError::Cancelled);
                    }
                }
                None
            }
        }
        let (mut owner, optimizer, [node, _]) = fixture();
        let median = OwnerPlacementPoint::new(Point::default());
        let context = Mutate {
            polls: Cell::new(0),
            median: median.clone(),
            optimizer: &optimizer,
            cancel: false,
        };
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&context), "placement", 100, &mut owner)
                .unwrap();
        guard.add_with_owner(60, &mut owner).unwrap();
        assert_eq!(
            optimizer
                .find_closest_unoccupied_distance_guarded(
                    &mut owner,
                    Some(node),
                    Some(&median),
                    &mut guard
                )
                .unwrap(),
            1.0
        );
        assert_eq!((guard.used(), context.polls.get()), (66, 2));
        let canceled = Mutate {
            polls: Cell::new(0),
            median: median.clone(),
            optimizer: &optimizer,
            cancel: true,
        };
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&canceled), "placement", 100, &mut owner)
                .unwrap();
        guard.add_with_owner(60, &mut owner).unwrap();
        assert_eq!(
            optimizer
                .find_closest_unoccupied_distance_guarded(
                    &mut owner,
                    Some(node),
                    Some(&median),
                    &mut guard
                )
                .unwrap_err()
                .to_string(),
            "placement: context canceled"
        );
        assert_eq!((guard.used(), canceled.polls.get()), (64, 2));
    }
}
