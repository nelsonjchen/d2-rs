// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Actual owner eligibility: sizeless_optimizer.go598–625 and
//! optimization_guard.go23–50. The Near map cursor preserves live map/header
//! semantics, but not Go1.27 randomized multi-key ordering or work prefixes.
//! Missing real group vessels remain explicit unsupported operands.

use super::limits::{OptimizationError, OptimizationWorkGuard};
#[cfg(test)]
use super::shared_topology::TopologyVessel;
use super::{ArenaGraph, NodeId, OwnerGraphId, OwnerTopologyError};
use std::collections::BTreeSet;
use std::fmt;

const MAX_ENGINE_NODES: usize = 10_000;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum OwnerNearEligibilityError {
    Optimization(OptimizationError),
    Topology(OwnerTopologyError),
    UnboundDebugId(NodeId),
}

impl From<OptimizationError> for OwnerNearEligibilityError {
    fn from(value: OptimizationError) -> Self {
        Self::Optimization(value)
    }
}

// canOptimizeNodeGuarded returns guard errors with their original causes.
impl From<OwnerNearEligibilityError> for OptimizationError {
    fn from(value: OwnerNearEligibilityError) -> Self {
        match value {
            OwnerNearEligibilityError::Optimization(error) => error,
            other => Self::Invalid(other.to_string()),
        }
    }
}

impl From<OwnerTopologyError> for OwnerNearEligibilityError {
    fn from(value: OwnerTopologyError) -> Self {
        Self::Topology(value)
    }
}

impl fmt::Display for OwnerNearEligibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Optimization(error) => error.fmt(f),
            Self::Topology(error) => write!(f, "unbound owner topology: {error:?}"),
            Self::UnboundDebugId(node) => write!(f, "unbound source node DebugID: {node:?}"),
        }
    }
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

/// The current pointer is loaded before Step; its parent fields are read only
/// after the real callback. The ancestor argument is frozen for this walk.
pub(super) fn optimizer_owner_is_descendant_of(
    owner: &mut ArenaGraph,
    descendant: Option<NodeId>,
    ancestor: Option<NodeId>,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<bool, OwnerNearEligibilityError> {
    let mut seen = BTreeSet::new();
    let mut current = descendant;
    loop {
        step(owner, guard)?;
        if current == ancestor {
            return Ok(true);
        }
        let Some(node) = current else {
            return Ok(ancestor.is_none());
        };
        if seen.contains(&node) {
            // Auxiliary runtime group DebugID requires source metadata that
            // cannot be fabricated from projected vessel/member IDs.
            if owner.topology_source_input_node(node).is_none() {
                return Err(OwnerNearEligibilityError::UnboundDebugId(node));
            }
            let live = owner
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?;
            return Err(OptimizationError::Invalid(format!(
                "TALA {} found a cycle in node ancestry at {}",
                guard.location(),
                live.external_id
            ))
            .into());
        }
        if seen.len() >= MAX_ENGINE_NODES {
            return Err(OptimizationError::Invalid(format!(
                "TALA {} ancestry exceeds node limit {}",
                guard.location(),
                MAX_ENGINE_NODES
            ))
            .into());
        }
        seen.insert(node);
        let live = owner
            .topology_node(node)
            .ok_or(OwnerTopologyError::UnknownNode(node))?;
        current = if let Some(container) = live.container {
            Some(container)
        } else if let Some(vessel) =
            owner.owner_node_group_vessel(node, super::owner_groups::OwnerGroupKind::Cluster)?
        {
            vessel
        } else if let Some(vessel) =
            owner.owner_node_group_vessel(node, super::owner_groups::OwnerGroupKind::Sequence)?
        {
            vessel
        } else {
            None
        };
    }
}

pub(super) fn can_optimize_owner_node_guarded(
    owner: &mut ArenaGraph,
    node: Option<NodeId>,
    receiver: Option<OwnerGraphId>,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<bool, OwnerNearEligibilityError> {
    let (Some(node), Some(receiver)) = (node, receiver) else {
        return Err(OptimizationError::Invalid(format!(
            "TALA {} found a nil optimizer node",
            guard.location()
        ))
        .into());
    };
    let live = owner
        .topology_node(node)
        .ok_or(OwnerTopologyError::UnknownNode(node))?;
    if owner.owner_node_incident_count(node) == 0 {
        let mut cursor = live.nears.live_cursor();
        let mut usable_near = false;
        for near in &mut cursor {
            // Near is the actual yielded local, even if the callback deletes
            // it or replaces node.Nears. node.Container is read AFTER Step.
            step(owner, guard)?;
            let ancestor = owner
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?
                .container;
            if optimizer_owner_is_descendant_of(owner, near, ancestor, guard)? {
                usable_near = true;
                break;
            }
        }
        if !usable_near {
            return Ok(false);
        }
    }
    if owner
        .topology_node(node)
        .ok_or(OwnerTopologyError::UnknownNode(node))?
        .fixed_top_left
        .is_some()
    {
        return Ok(false);
    }
    Ok(owner
        .owner_graph_node_to_tree_entry(receiver, Some(node))?
        .is_none())
}

#[cfg(test)]
mod tests {
    use super::super::limits::{Context, ContextError};
    use super::super::node_near_map::NodeNearMap;
    use super::*;
    use crate::{Graph, Node, Point, Size};

    struct Quiet;
    impl Context for Quiet {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            None
        }
    }

    fn fixture() -> (ArenaGraph, OwnerGraphId, [NodeId; 3]) {
        let mut input = Graph::default();
        let ids = ["C", "D", "L"].map(|id| {
            input.add_node(Node::new(
                id,
                Size {
                    width: 10.0,
                    height: 10.0,
                },
            ))
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        let receiver = owner.create_owner_graph_view(ids.to_vec(), vec![]).unwrap();
        (owner, receiver, ids)
    }

    #[test]
    fn owner_near_nil_key_work_precedes_fixed_and_selected_present_nil_tree() {
        let (mut owner, receiver, [node, _, _]) = fixture();
        owner.nodes[node.0 as usize].nears = NodeNearMap::from_raw_keys([None]);
        owner.nodes[node.0 as usize].fixed_top_left = Some(Point { x: 1.0, y: 2.0 });
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "near", 100).unwrap();
        assert!(
            !can_optimize_owner_node_guarded(&mut owner, Some(node), Some(receiver), &mut guard)
                .unwrap()
        );
        assert_eq!(guard.used(), 2);
        owner.nodes[node.0 as usize].fixed_top_left = None;
        let map = owner.allocate_owner_node_to_tree_map(true);
        owner
            .write_owner_node_to_tree_entry(&map, Some(node), None)
            .unwrap();
        owner
            .set_owner_graph_node_to_tree_map(receiver, map)
            .unwrap();
        assert!(
            !can_optimize_owner_node_guarded(&mut owner, Some(node), Some(receiver), &mut guard)
                .unwrap()
        );
        assert_eq!(guard.used(), 4);
        let empty = owner.allocate_owner_node_to_tree_map(false);
        owner
            .set_owner_graph_node_to_tree_map(receiver, empty)
            .unwrap();
        assert!(
            can_optimize_owner_node_guarded(&mut owner, Some(node), Some(receiver), &mut guard)
                .unwrap()
        );
        assert_eq!(guard.used(), 6);
    }

    #[test]
    fn owner_ancestry_walks_nil_ancestor_and_detects_cycle_before_nil_shortcut() {
        let (mut owner, _, [first, second, _]) = fixture();
        owner.nodes[first.0 as usize].container = Some(second);
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "ancestry", 100).unwrap();
        assert!(
            optimizer_owner_is_descendant_of(&mut owner, Some(first), None, &mut guard).unwrap()
        );
        assert_eq!(guard.used(), 3);
        owner.nodes[first.0 as usize].container = Some(first);
        let error = optimizer_owner_is_descendant_of(&mut owner, Some(first), None, &mut guard)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "TALA ancestry found a cycle in node ancestry at C"
        );
        assert_eq!(guard.used(), 5);
        assert!(optimizer_owner_is_descendant_of(&mut owner, None, None, &mut guard).unwrap());
        assert_eq!(guard.used(), 6);
    }

    #[test]
    fn owner_ancestry_preserves_container_priority_and_reports_unbound_group_vessel() {
        let (mut owner, _, [first, second, _]) = fixture();
        owner.nodes[first.0 as usize].container = Some(second);
        owner.nodes[first.0 as usize].cluster = Some(99);
        owner.nodes[first.0 as usize].sequence = Some(98);
        let mut guard = OptimizationWorkGuard::new(Some(&Quiet), "ancestry", 100).unwrap();
        assert!(
            optimizer_owner_is_descendant_of(&mut owner, Some(first), Some(second), &mut guard)
                .unwrap()
        );
        assert_eq!(guard.used(), 2);
        owner.nodes[first.0 as usize].container = None;
        assert_eq!(
            optimizer_owner_is_descendant_of(&mut owner, Some(first), None, &mut guard),
            Err(OwnerNearEligibilityError::Topology(
                OwnerTopologyError::UnboundVessel(TopologyVessel::Cluster(99))
            ))
        );
        assert_eq!(guard.used(), 3);
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn owner_near_actual_outer_poll_keeps_loaded_key_and_reads_new_ancestor() {
        use std::cell::Cell;
        struct Mutate {
            polls: Cell<usize>,
            node: NodeId,
            near: NodeId,
            parent: NodeId,
        }
        impl Context for Mutate {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                panic!("immutable poll")
            }
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
                let next = self.polls.get() + 1;
                self.polls.set(next);
                if next == 2 {
                    owner.nodes[self.node.0 as usize]
                        .nears
                        .delete(Some(self.near));
                    owner.nodes[self.node.0 as usize].nears = NodeNearMap::nil();
                    owner.nodes[self.node.0 as usize].container = Some(self.parent);
                    owner.nodes[self.near.0 as usize].container = Some(self.parent);
                }
                None
            }
        }
        let (mut owner, receiver, [node, near, parent]) = fixture();
        owner.nodes[node.0 as usize].nears = NodeNearMap::from_raw_keys([Some(near)]);
        let original = owner.nodes[node.0 as usize].nears.retained();
        let context = Mutate {
            polls: Cell::new(0),
            node,
            near,
            parent,
        };
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&context), "near", 100, &mut owner).unwrap();
        guard.add_with_owner(63, &mut owner).unwrap();
        assert!(
            can_optimize_owner_node_guarded(&mut owner, Some(node), Some(receiver), &mut guard)
                .unwrap()
        );
        assert_eq!((guard.used(), context.polls.get()), (66, 2));
        assert!(owner.nodes[node.0 as usize].nears.is_nil());
        assert!(!original.is_nil() && original.is_empty());
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn owner_near_actual_inner_poll_freezes_ancestor_reads_live_parent_and_cancel_precedes_equal() {
        use std::cell::Cell;
        struct Mutate {
            polls: Cell<usize>,
            node: NodeId,
            near: NodeId,
            parent: NodeId,
            cancel: bool,
        }
        impl Context for Mutate {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                panic!("immutable poll")
            }
            fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
                let next = self.polls.get() + 1;
                self.polls.set(next);
                if next == 2 {
                    owner.nodes[self.node.0 as usize].container = None;
                    owner.nodes[self.near.0 as usize].container = Some(self.parent);
                    if self.cancel {
                        return Some(ContextError::Cancelled);
                    }
                }
                None
            }
        }
        let (mut owner, receiver, [node, near, parent]) = fixture();
        owner.nodes[node.0 as usize].nears = NodeNearMap::from_raw_keys([Some(near)]);
        owner.nodes[node.0 as usize].container = Some(parent);
        let context = Mutate {
            polls: Cell::new(0),
            node,
            near,
            parent,
            cancel: false,
        };
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&context), "near", 100, &mut owner).unwrap();
        guard.add_with_owner(62, &mut owner).unwrap();
        assert!(
            can_optimize_owner_node_guarded(&mut owner, Some(node), Some(receiver), &mut guard)
                .unwrap()
        );
        assert_eq!((guard.used(), context.polls.get()), (65, 2));
        let canceled = Mutate {
            polls: Cell::new(0),
            node,
            near,
            parent,
            cancel: true,
        };
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&canceled), "near", 100, &mut owner)
                .unwrap();
        guard.add_with_owner(63, &mut owner).unwrap();
        let error =
            optimizer_owner_is_descendant_of(&mut owner, None, None, &mut guard).unwrap_err();
        assert_eq!(error.to_string(), "near: context canceled");
        assert_eq!((guard.used(), canceled.polls.get()), (64, 2));
    }
}
