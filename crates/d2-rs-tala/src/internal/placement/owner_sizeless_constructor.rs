// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Source sizeless_optimizer.go29–90: setup uses three independent current
//! Graph.Nodes ranges. This owner state is not yet the production dense pass.
//! Near eligibility reads the real shared map and guarded live ancestry.
//! Randomized multi-key Near order remains separately unqualified.

use super::go_rng::GoRng;
use super::limits::{Context, OptimizationError, OptimizationWorkGuard};
use super::occupied::OccupiedPointMap;
use super::owner_near_eligibility::{OwnerNearEligibilityError, can_optimize_owner_node_guarded};
use super::owner_score_session::OwnerOptimizerMutationSnapshot;
use super::{ArenaGraph, NodeId, OwnerGraphId, OwnerMemberHeader, OwnerTopologyError};
use d2_rs_core::stable_sort::StableSort;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

const MAX_ENGINE_NODES: usize = 10_000;
const MAX_ENGINE_EDGES: usize = 50_000;

/// A caller-supplied stream reference, rather than a clone of its RNG state.
/// Setup stores this alias, accepts nil and never draws from it.
#[derive(Clone)]
pub(super) struct OwnerOptimizerRng(Rc<RefCell<GoRng>>);

impl OwnerOptimizerRng {
    pub(super) fn new(rng: GoRng) -> Self {
        Self(Rc::new(RefCell::new(rng)))
    }

    pub(super) fn stream(&self) -> &RefCell<GoRng> {
        &self.0
    }

    pub(super) fn into_rng(self) -> GoRng {
        Rc::try_unwrap(self.0)
            .expect("optimizer stream aliases released before handoff")
            .into_inner()
    }

    pub(super) fn same_stream(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn reference_address(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }
}

#[derive(Debug)]
pub(super) enum OwnerSizelessConstructorError {
    Source(OptimizationError),
    Topology(OwnerTopologyError),
    UnboundDebugIdentity(NodeId),
}

impl From<OptimizationError> for OwnerSizelessConstructorError {
    fn from(error: OptimizationError) -> Self {
        Self::Source(error)
    }
}

impl From<OwnerTopologyError> for OwnerSizelessConstructorError {
    fn from(error: OwnerTopologyError) -> Self {
        Self::Topology(error)
    }
}

impl From<OwnerNearEligibilityError> for OwnerSizelessConstructorError {
    fn from(error: OwnerNearEligibilityError) -> Self {
        match error {
            OwnerNearEligibilityError::Optimization(error) => Self::Source(error),
            OwnerNearEligibilityError::Topology(error) => Self::Topology(error),
            OwnerNearEligibilityError::UnboundDebugId(node) => Self::UnboundDebugIdentity(node),
        }
    }
}

impl fmt::Display for OwnerSizelessConstructorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => error.fmt(formatter),
            Self::Topology(error) => error.fmt(formatter),
            Self::UnboundDebugIdentity(node) => {
                write!(formatter, "owner optimizer DebugID is unbound for {node:?}")
            }
        }
    }
}

impl std::error::Error for OwnerSizelessConstructorError {}

pub(super) struct OwnerSizelessOptimizer {
    receiver: OwnerGraphId,
    nodes: OwnerMemberHeader,
    occupied: RefCell<OccupiedPointMap>,
    rng: Option<OwnerOptimizerRng>,
    mutation_scratch: RefCell<OwnerOptimizerMutationSnapshot>,
    #[cfg(feature = "diagnostic-traces")]
    setup_work_used: u64,
}

fn source_error(message: impl Into<String>) -> OwnerSizelessConstructorError {
    OptimizationError::Invalid(message.into()).into()
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

fn go_fixed_float(value: f64) -> String {
    if value.is_nan() {
        "NaN".into()
    } else if value == f64::INFINITY {
        "+Inf".into()
    } else if value == f64::NEG_INFINITY {
        "-Inf".into()
    } else {
        format!("{value:.6}")
    }
}

impl OwnerSizelessOptimizer {
    pub(super) fn new(
        owner: &mut ArenaGraph,
        receiver: Option<OwnerGraphId>,
        rng: Option<OwnerOptimizerRng>,
        context: Option<&dyn Context>,
    ) -> Result<Self, OwnerSizelessConstructorError> {
        let receiver =
            receiver.ok_or_else(|| source_error("TALA sizelessOptimizer requires a graph"))?;
        let initial = owner.owner_graph_member_header(receiver)?;
        if initial.len() > MAX_ENGINE_NODES {
            return Err(source_error(format!(
                "TALA sizelessOptimizer node count exceeds limit {MAX_ENGINE_NODES}"
            )));
        }
        let edges = owner
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?
            .edges
            .len();
        if edges > MAX_ENGINE_EDGES {
            return Err(source_error(format!(
                "TALA sizelessOptimizer edge count exceeds limit {MAX_ENGINE_EDGES}"
            )));
        }
        #[cfg(feature = "diagnostic-traces")]
        let mut guard = OptimizationWorkGuard::new_with_owner(
            context,
            "sizelessOptimizer.setup",
            super::limits::MAX_OPTIMIZATION_WORK_UNITS,
            owner,
        )?;
        #[cfg(not(feature = "diagnostic-traces"))]
        let mut guard = OptimizationWorkGuard::new(
            context,
            "sizelessOptimizer.setup",
            super::limits::MAX_OPTIMIZATION_WORK_UNITS,
        )?;

        // The source range is bound after NewGuard, independently of the
        // precheck. Each nullable slot is loaded before its accepted Step.
        let validation = owner.owner_graph_member_header(receiver)?;
        for index in 0..validation.len() {
            let node = validation.get(index).expect("retained member logical slot");
            step(owner, &mut guard)?;
            let node = node.ok_or_else(|| {
                source_error("TALA sizelessOptimizer found a node without a graph")
            })?;
            let node_graph = owner.topology_node_owner(node).ok_or_else(|| {
                source_error("TALA sizelessOptimizer found a node without a graph")
            })?;
            let live = owner
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?;
            if owner.owner_node_incident_count(node) > MAX_ENGINE_EDGES
                || live.nears.len() > MAX_ENGINE_NODES
            {
                // D2ID presence is retained for canonical input nodes. Runtime
                // aggregate DebugID needs the source Cluster/Sequence objects.
                if owner.topology_source_input_node(node).is_none() {
                    return Err(OwnerSizelessConstructorError::UnboundDebugIdentity(node));
                }
                return Err(source_error(format!(
                    "TALA sizelessOptimizer node {} adjacency references exceed engine limits",
                    live.external_id
                )));
            }
            let node_cell_size = owner.owner_graph_cell_size(node_graph)?;
            let graph_cell_size = owner.owner_graph_cell_size(receiver)?;
            if node_cell_size != graph_cell_size {
                return Err(source_error(format!(
                    "mismatch of cell size, graph={}, node={}",
                    go_fixed_float(graph_cell_size),
                    go_fixed_float(node_cell_size)
                )));
            }
        }

        // Allocation uses the CURRENT member length, after validation polls.
        let capacity = owner.owner_graph_member_header(receiver)?.len();
        let nodes = owner.allocate_owner_member_header(&[], capacity)?;
        let mut optimizer = Self {
            receiver,
            nodes,
            occupied: RefCell::new(OccupiedPointMap::default()),
            rng,
            mutation_scratch: RefCell::new(OwnerOptimizerMutationSnapshot::default()),
            #[cfg(feature = "diagnostic-traces")]
            setup_work_used: 0,
        };
        optimizer.reset_occupied(owner)?;

        // Source selection binds a third header, after the uncharged reset.
        let selection = owner.owner_graph_member_header(receiver)?;
        for index in 0..selection.len() {
            let node = selection.get(index).expect("retained member logical slot");
            step(owner, &mut guard)?;
            let can_optimize =
                can_optimize_owner_node_guarded(owner, node, Some(receiver), &mut guard)?;
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_enabled("TALA_TRACE_SIZELESS_MEMBERS") {
                if let Some(node) = node {
                    let name = |node: Option<super::NodeId>| {
                        node.map_or_else(
                            || "nil".into(),
                            |node| {
                                owner
                                    .topology_node(node)
                                    .expect("actual member node")
                                    .external_id
                                    .clone()
                            },
                        )
                    };
                    let live = owner.topology_node(node).expect("actual optimizer member");
                    let mut nears = live
                        .nears
                        .raw_keys()
                        .into_iter()
                        .map(name)
                        .collect::<Vec<_>>();
                    nears.stable_sort();
                    let edges = owner.owner_node_incident_header(node)?.iter_slots().map(|edge| {
                        let edge = edge.expect("actual member edge");
                        let from = owner.owner_edge_endpoint(edge, false).expect("actual endpoint");
                        let to = owner.owner_edge_endpoint(edge, true).expect("actual endpoint");
                        serde_json::json!({"from":name(from),"to":name(to),"from_positioned":from.is_some_and(|node| owner.position(node).is_some()),"to_positioned":to.is_some_and(|node| owner.position(node).is_some())})
                    }).collect::<Vec<_>>();
                    eprintln!(
                        "{}",
                        serde_json::json!({"event":"sizeless-member","node_id":name(Some(node)),"container":name(live.container),"selected":can_optimize,"edges":edges,"nears":nears,"in_tree":owner.owner_graph_node_to_tree_entry(receiver, Some(node))?.is_some()})
                    );
                }
            }
            if can_optimize {
                optimizer.nodes = owner.append_owner_member_header(&optimizer.nodes, &[node])?;
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        guard.finish_with_owner(owner)?;
        #[cfg(not(feature = "diagnostic-traces"))]
        guard.finish()?;
        #[cfg(feature = "diagnostic-traces")]
        {
            optimizer.setup_work_used = guard.used();
        }
        Ok(optimizer)
    }

    /// Source82–90 replaces the occupied map, even when current membership is
    /// nil/empty. This range has no work charge and no context poll.
    pub(super) fn reset_occupied(
        &mut self,
        owner: &ArenaGraph,
    ) -> Result<(), OwnerSizelessConstructorError> {
        let members = owner.owner_graph_member_header(self.receiver)?;
        let map = OccupiedPointMap::new();
        *self.occupied.borrow_mut() = map.clone();
        for index in 0..members.len() {
            // Go dereferences n.TopLeft directly here; nil is a runtime panic,
            // not the validation/selection error from either charged range.
            let node = members
                .get(index)
                .expect("retained member logical slot")
                .expect("runtime error: invalid memory address or nil pointer dereference");
            owner
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?;
            if let Some(point) = owner.position(node) {
                map.insert(point, Some(node));
            }
        }
        Ok(())
    }

    pub(super) fn nodes(&self) -> &OwnerMemberHeader {
        &self.nodes
    }

    pub(super) fn selected_graph(&self) -> OwnerGraphId {
        self.receiver
    }

    pub(super) fn occupied(&self) -> &RefCell<OccupiedPointMap> {
        &self.occupied
    }

    pub(super) fn take_mutation_scratch(&self) -> OwnerOptimizerMutationSnapshot {
        self.mutation_scratch.take()
    }
    pub(super) fn retain_mutation_scratch(&self, snapshot: OwnerOptimizerMutationSnapshot) {
        *self.mutation_scratch.borrow_mut() = snapshot;
    }
    pub(super) fn rng(&self) -> Option<&OwnerOptimizerRng> {
        self.rng.as_ref()
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn setup_work_used(&self) -> u64 {
        self.setup_work_used
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn receiver(&self) -> OwnerGraphId {
        self.receiver
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, Point, Size};
    #[cfg(feature = "diagnostic-traces")]
    use std::cell::Cell;

    struct QuietContext;
    impl Context for QuietContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<super::super::limits::ContextError> {
            None
        }
    }

    fn fixture() -> (ArenaGraph, OwnerGraphId, NodeId, NodeId) {
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "C",
            Size {
                width: 10.0,
                height: 20.0,
            },
        ));
        let second = input.add_node(Node::new(
            "D",
            Size {
                width: 30.0,
                height: 40.0,
            },
        ));
        let edge = input.add_edge(Edge {
            source: first,
            target: second,
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.containers.clear();
        owner.cell_size = 28.0;
        owner.set_position(first, Point { x: 1.25, y: 2.5 });
        owner.set_position(second, Point { x: 3.75, y: 4.5 });
        owner.initialize_shared_topology();
        let selected = owner
            .create_owner_graph_view(vec![first, second], vec![edge])
            .unwrap();
        owner.set_owner_graph_cell_size(selected, 28.0).unwrap();
        (owner, selected, first, second)
    }

    #[test]
    fn constructor_uses_selected_tree_key_presence_including_nil_value_and_keeps_rng_alias() {
        let (mut owner, selected, first, second) = fixture();
        assert_eq!(owner.topology_node_owner(first), Some(OwnerGraphId(0)));
        for allocated in [false, true] {
            let map = owner.allocate_owner_node_to_tree_map(allocated);
            owner
                .set_owner_graph_node_to_tree_map(selected, map)
                .unwrap();
            let optimizer =
                OwnerSizelessOptimizer::new(&mut owner, Some(selected), None, Some(&QuietContext))
                    .unwrap();
            assert_eq!(
                optimizer.nodes().visible_slots(),
                vec![Some(first), Some(second)]
            );
            assert!(optimizer.rng().is_none());
            assert!(!optimizer.occupied().borrow().is_nil());
            assert_eq!(
                optimizer
                    .occupied()
                    .borrow()
                    .lookup(Point { x: 1.25, y: 2.5 }),
                Some(Some(first))
            );
        }
        let map = owner.allocate_owner_node_to_tree_map(true);
        owner
            .write_owner_node_to_tree_entry(&map, Some(first), None)
            .unwrap();
        owner
            .set_owner_graph_node_to_tree_map(selected, map)
            .unwrap();
        let rng = OwnerOptimizerRng::new(GoRng::new_untraced(1729));
        let mut expected_rng = rng.stream().borrow().clone_without_trace();
        let optimizer = OwnerSizelessOptimizer::new(
            &mut owner,
            Some(selected),
            Some(rng.clone()),
            Some(&QuietContext),
        )
        .unwrap();
        assert_eq!(optimizer.nodes().visible_slots(), vec![Some(second)]);
        assert_eq!(optimizer.nodes().capacity(), 2);
        assert_eq!(
            owner
                .owner_graph_node_to_tree_entry(OwnerGraphId(0), Some(first))
                .unwrap(),
            None
        );
        assert!(optimizer.rng().unwrap().same_stream(&rng));
        assert_eq!(
            rng.stream().borrow_mut().float64().to_bits(),
            expected_rng.float64().to_bits()
        );
        assert_eq!(
            optimizer
                .rng()
                .unwrap()
                .stream()
                .borrow_mut()
                .float64()
                .to_bits(),
            expected_rng.float64().to_bits()
        );
        #[cfg(feature = "diagnostic-traces")]
        assert_eq!(optimizer.setup_work_used, 4);
    }

    #[test]
    fn constructor_source_prechecks_precede_context_and_cell_size_errors_preserve_go_float_text() {
        let (mut owner, selected, first, _) = fixture();
        let nil_error = OwnerSizelessOptimizer::new(&mut owner, None, None, None)
            .err()
            .unwrap();
        assert_eq!(
            nil_error.to_string(),
            "TALA sizelessOptimizer requires a graph"
        );
        let huge = owner
            .allocate_owner_member_header(&vec![Some(first); 10_001], 10_001)
            .unwrap();
        owner.set_owner_graph_member_header(selected, huge).unwrap();
        let count_error = OwnerSizelessOptimizer::new(&mut owner, Some(selected), None, None)
            .err()
            .unwrap();
        assert_eq!(
            count_error.to_string(),
            "TALA sizelessOptimizer node count exceeds limit 10000"
        );
        let members = owner
            .allocate_owner_member_header(&[Some(first)], 1)
            .unwrap();
        owner
            .set_owner_graph_member_header(selected, members)
            .unwrap();
        for (graph_size, node_size, expected) in [
            (
                1.25,
                2.5,
                "mismatch of cell size, graph=1.250000, node=2.500000",
            ),
            (
                f64::INFINITY,
                f64::NEG_INFINITY,
                "mismatch of cell size, graph=+Inf, node=-Inf",
            ),
            (
                -0.0,
                f64::NAN,
                "mismatch of cell size, graph=-0.000000, node=NaN",
            ),
        ] {
            owner
                .set_owner_graph_cell_size(selected, graph_size)
                .unwrap();
            owner.cell_size = node_size;
            let error =
                OwnerSizelessOptimizer::new(&mut owner, Some(selected), None, Some(&QuietContext))
                    .err()
                    .unwrap();
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn constructor_near_ancestry_error_precedes_fixed_node_exclusion() {
        let (mut owner, selected, first, second) = fixture();
        let members = owner
            .allocate_owner_member_header(&[Some(first)], 1)
            .unwrap();
        owner
            .set_owner_graph_member_header(selected, members)
            .unwrap();
        owner.nodes[first.0 as usize].edges.clear();
        owner.nodes[first.0 as usize].nears =
            super::super::node_near_map::NodeNearMap::from_raw_keys([Some(second)]);
        owner.nodes[second.0 as usize].container = Some(second);
        owner.nodes[first.0 as usize].fixed_top_left = Some(Point { x: 9.0, y: 10.0 });
        let error =
            OwnerSizelessOptimizer::new(&mut owner, Some(selected), None, Some(&QuietContext))
                .err()
                .unwrap();
        assert_eq!(
            error.to_string(),
            "TALA sizelessOptimizer.setup found a cycle in node ancestry at D"
        );
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn constructor_retains_validation_range_then_reads_current_reset_selection_and_final_geometry()
    {
        struct ReplacingContext {
            polls: Cell<u64>,
            selected: OwnerGraphId,
            second: NodeId,
            rng: OwnerOptimizerRng,
        }
        impl Context for ReplacingContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<super::super::limits::ContextError> {
                panic!("constructor should poll with the actual mutable owner")
            }
            fn error_with_owner(
                &self,
                owner: &mut ArenaGraph,
            ) -> Option<super::super::limits::ContextError> {
                let poll = self.polls.get() + 1;
                self.polls.set(poll);
                if poll == 2 {
                    let replacement = owner
                        .allocate_owner_member_header(&[Some(self.second)], 5)
                        .unwrap();
                    owner
                        .set_owner_graph_member_header(self.selected, replacement)
                        .unwrap();
                    let _ = self.rng.stream().borrow_mut().float64();
                } else if poll == 3 {
                    owner.set_position(self.second, Point { x: 33.25, y: 44.5 });
                }
                None
            }
        }
        let (mut owner, selected, first, second) = fixture();
        let members = owner
            .allocate_owner_member_header(&vec![Some(first); 65], 70)
            .unwrap();
        owner
            .set_owner_graph_member_header(selected, members.clone())
            .unwrap();
        let rng = OwnerOptimizerRng::new(GoRng::new_untraced(1729));
        let mut expected = rng.stream().borrow().clone_without_trace();
        let _ = expected.float64();
        let context = ReplacingContext {
            polls: Cell::new(0),
            selected,
            second,
            rng: rng.clone(),
        };
        let optimizer = OwnerSizelessOptimizer::new(
            &mut owner,
            Some(selected),
            Some(rng.clone()),
            Some(&context),
        )
        .unwrap();
        assert_eq!(context.polls.get(), 3);
        assert_eq!(optimizer.setup_work_used, 66);
        assert_eq!(optimizer.nodes().visible_slots(), vec![Some(second)]);
        assert_eq!(optimizer.nodes().capacity(), 1);
        assert_eq!(members.visible_slots(), vec![Some(first); 65]);
        assert_eq!(
            owner
                .owner_graph_member_header(selected)
                .unwrap()
                .visible_slots(),
            vec![Some(second)]
        );
        assert_eq!(
            optimizer
                .occupied()
                .borrow()
                .lookup(Point { x: 3.75, y: 4.5 }),
            Some(Some(second))
        );
        assert_eq!(
            optimizer
                .occupied()
                .borrow()
                .lookup(Point { x: 33.25, y: 44.5 }),
            None
        );
        assert_eq!(owner.position(second), Some(Point { x: 33.25, y: 44.5 }));
        assert_eq!(
            rng.stream().borrow_mut().float64().to_bits(),
            expected.float64().to_bits()
        );
        assert!(optimizer.rng().unwrap().same_stream(&rng));
    }
}
