// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Source optimizer positional transactions on the actual exclusive owner.
use super::limits::{OptimizationError, OptimizationWorkGuard};
use super::shared_topology::OwnerNodePointSnapshot;
use super::{ArenaGraph, NodeId, OwnerGraphId, OwnerTopologyError, Point};
use std::collections::BTreeSet;
#[cfg(test)]
#[path = "owner_swap_motion_tests.rs"]
mod tests;

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
fn invalid(guard: &OptimizationWorkGuard<'_>, message: &str) -> OptimizationError {
    OptimizationError::Invalid(format!("TALA {} {message}", guard.location()))
}

struct PositionSnapshot {
    node: NodeId,
    point: Option<OwnerNodePointSnapshot>,
    legacy: Option<Point>,
    origin: Point,
}
impl PositionSnapshot {
    fn capture(owner: &ArenaGraph, node: NodeId) -> Result<Self, OptimizationError> {
        let live = owner.topology_node(node).ok_or_else(|| {
            OptimizationError::Invalid(OwnerTopologyError::UnknownNode(node).to_string())
        })?;
        let point = match owner.owner_node_point(node) {
            Ok(point) => Some(point.snapshot()),
            Err(OwnerTopologyError::UnboundNodePoint(_)) => None,
            Err(error) => return Err(OptimizationError::Invalid(error.to_string())),
        };
        Ok(Self {
            node,
            point,
            legacy: owner.position(node),
            origin: live.rect.origin,
        })
    }
    fn restore(&self, owner: &mut ArenaGraph) {
        if let Some(point) = &self.point {
            owner
                .restore_owner_node_point(self.node, point)
                .expect("captured point scope");
        } else {
            owner.nodes[self.node.0 as usize].position = self.legacy;
        }
        owner.nodes[self.node.0 as usize].rect.origin = self.origin;
    }
}
// Own the rollback through ordinary returns and Rust unwinding; no cloned
// arena or synthetic allocation identities substitute for original pointers.
struct PositionScope<'a> {
    owner: &'a mut ArenaGraph,
    snapshots: Vec<PositionSnapshot>,
    complete: bool,
}
impl Drop for PositionScope<'_> {
    fn drop(&mut self) {
        if !self.complete {
            for snapshot in &self.snapshots {
                snapshot.restore(self.owner);
            }
        }
    }
}

impl ArenaGraph {
    fn optimizer_leaf(&self, node: NodeId) -> Result<bool, OptimizationError> {
        let live = self.topology_node(node).ok_or_else(|| {
            OptimizationError::Invalid(OwnerTopologyError::UnknownNode(node).to_string())
        })?;
        let Some(graph) = self.topology_node_owner(node) else {
            return Ok(false);
        };
        Ok(!live.is_container
            && !self.owner_is_cluster_vessel(node)
            && self
                .owner_graph_group_entry(
                    graph,
                    super::owner_groups::OwnerGroupKind::Sequence,
                    Some(node),
                )
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?
                .flatten()
                .is_none())
    }
    fn optimizer_descendants(
        &mut self,
        node: Option<NodeId>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<Vec<NodeId>, OptimizationError> {
        let node = node.ok_or_else(|| invalid(guard, "movement requires a node with a graph"))?;
        let graph = self
            .topology_node_owner(node)
            .ok_or_else(|| invalid(guard, "movement requires a node with a graph"))?;
        let mut seen = BTreeSet::from([node]);
        let mut stack = Vec::new();
        let mut descendants = Vec::new();
        self.owner_optimizer_push_children(node, graph, guard, &mut seen, &mut stack)?;
        while !stack.is_empty() {
            step(self, guard)?;
            let current = stack.pop().unwrap();
            descendants.push(current);
            self.owner_optimizer_push_children(current, graph, guard, &mut seen, &mut stack)?;
        }
        Ok(descendants)
    }
    fn optimizer_capture_positions(
        &mut self,
        nodes: &[Option<NodeId>],
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<Vec<PositionSnapshot>, OptimizationError> {
        if nodes.len() <= 2 {
            let mut leaves = true;
            for &node in nodes {
                let node = node.ok_or_else(|| invalid(guard, "found a nil node"))?;
                if !self.optimizer_leaf(node)? {
                    leaves = false;
                    break;
                }
            }
            if leaves {
                let mut snapshots: Vec<PositionSnapshot> = Vec::with_capacity(nodes.len());
                for &node in nodes {
                    let node = node.unwrap();
                    if snapshots.iter().any(|s| s.node == node) {
                        continue;
                    }
                    step(self, guard)?;
                    snapshots.push(PositionSnapshot::capture(self, node)?);
                }
                return Ok(snapshots);
            }
        }
        if nodes.len() == 1 {
            let descendants = self.optimizer_descendants(nodes[0], guard)?;
            let mut snapshots = Vec::with_capacity(descendants.len() + 1);
            for node in std::iter::once(nodes[0].unwrap()).chain(descendants) {
                step(self, guard)?;
                snapshots.push(PositionSnapshot::capture(self, node)?);
            }
            return Ok(snapshots);
        }
        let mut seen = BTreeSet::new();
        let mut snapshots = Vec::with_capacity(nodes.len());
        for &node in nodes {
            let node = node.ok_or_else(|| invalid(guard, "found a nil node"))?;
            let descendants = self.optimizer_descendants(Some(node), guard)?;
            for current in std::iter::once(node).chain(descendants) {
                if seen.contains(&current) {
                    continue;
                }
                if seen.len() >= 10000 {
                    return Err(invalid(guard, "position snapshot exceeds node limit 10000"));
                }
                step(self, guard)?;
                seen.insert(current);
                snapshots.push(PositionSnapshot::capture(self, current)?);
            }
        }
        Ok(snapshots)
    }
    pub(super) fn optimizer_move_node_abs(
        &mut self,
        node: Option<NodeId>,
        point: Point,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OptimizationError> {
        let node = node.ok_or_else(|| invalid(guard, "cannot move an unpositioned node"))?;
        let current = self
            .position(node)
            .ok_or_else(|| invalid(guard, "cannot move an unpositioned node"))?;
        if current == point {
            return step(self, guard);
        }
        if self.optimizer_leaf(node)? {
            self.translate_owner_point(node, point.x - current.x, point.y - current.y);
            return Ok(());
        }
        let descendants = self.optimizer_descendants(Some(node), guard)?;
        let mut snapshots = Vec::with_capacity(descendants.len() + 1);
        snapshots.push(PositionSnapshot::capture(self, node)?);
        for &child in &descendants {
            step(self, guard)?;
            if self.position(child).is_none() {
                return Err(invalid(guard, "cannot move an unpositioned descendant"));
            }
            snapshots.push(PositionSnapshot::capture(self, child)?);
        }
        let mut scope = PositionScope {
            owner: self,
            snapshots,
            complete: false,
        };
        let current = scope
            .owner
            .position(node)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let delta = Point {
            x: point.x - current.x,
            y: point.y - current.y,
        };
        scope.owner.translate_owner_point(node, delta.x, delta.y);
        for child in descendants {
            step(scope.owner, guard)?;
            scope.owner.translate_owner_point(child, delta.x, delta.y);
        }
        scope.complete = true;
        Ok(())
    }
    pub(super) fn optimizer_swap_positions(
        &mut self,
        a: Option<NodeId>,
        b: Option<NodeId>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OptimizationError> {
        let (Some(a), Some(b)) = (a, b) else {
            return Err(invalid(guard, "cannot swap unpositioned nodes"));
        };
        if self.position(a).is_none() || self.position(b).is_none() {
            return Err(invalid(guard, "cannot swap unpositioned nodes"));
        }
        let snapshots = self.optimizer_capture_positions(&[Some(a), Some(b)], guard)?;
        let mut scope = PositionScope {
            owner: self,
            snapshots,
            complete: false,
        };
        let old_a = scope
            .owner
            .position(a)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let old_b = scope
            .owner
            .position(b)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        scope.owner.optimizer_move_node_abs(Some(a), old_b, guard)?;
        scope.owner.optimizer_move_node_abs(Some(b), old_a, guard)?;
        scope.complete = true;
        Ok(())
    }
    pub(super) fn with_optimizer_positions_swapped<R>(
        &mut self,
        a: Option<NodeId>,
        b: Option<NodeId>,
        guard: &mut OptimizationWorkGuard<'_>,
        callback: impl FnOnce(
            &mut ArenaGraph,
            &mut OptimizationWorkGuard<'_>,
        ) -> Result<R, OptimizationError>,
    ) -> Result<R, OptimizationError> {
        let before = guard.used();
        let snapshots = self.optimizer_capture_positions(&[a, b], guard)?;
        let work = guard.used() - before;
        let scope = PositionScope {
            owner: self,
            snapshots,
            complete: false,
        };
        let (Some(a), Some(b)) = (a, b) else {
            return Err(invalid(guard, "cannot swap unpositioned nodes"));
        };
        if scope.owner.position(a).is_none() || scope.owner.position(b).is_none() {
            return Err(invalid(guard, "cannot swap unpositioned nodes"));
        }
        for _ in 0..work {
            step(scope.owner, guard)?;
        }
        let old_a = scope
            .owner
            .position(a)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let old_b = scope
            .owner
            .position(b)
            .expect("runtime error: invalid memory address or nil pointer dereference");
        scope.owner.optimizer_move_node_abs(Some(a), old_b, guard)?;
        scope.owner.optimizer_move_node_abs(Some(b), old_a, guard)?;
        callback(scope.owner, guard)
    }
    pub(super) fn owner_optimizer_push_children(
        &mut self,
        parent: NodeId,
        receiver: OwnerGraphId,
        guard: &mut OptimizationWorkGuard<'_>,
        seen: &mut BTreeSet<NodeId>,
        stack: &mut Vec<NodeId>,
    ) -> Result<(), OptimizationError> {
        self.topology_node(parent).ok_or_else(|| {
            OptimizationError::Invalid(OwnerTopologyError::UnknownNode(parent).to_string())
        })?;
        use super::owner_groups::OwnerGroupKind;
        for kind in [OwnerGroupKind::Sequence, OwnerGroupKind::Cluster] {
            // The private flag is read only after sequence range callbacks.
            if kind == OwnerGroupKind::Cluster && !self.owner_is_cluster_vessel(parent) {
                continue;
            }
            let group = self
                .owner_graph_group_entry(receiver, kind, Some(parent))
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?
                .flatten();
            let Some(group) = group else { continue };
            let children = self
                .owner_group_fields(group)
                .map_err(|e| OptimizationError::Invalid(e.to_string()))?
                .nodes;
            if children.len() > 10_000 {
                let name = match kind {
                    OwnerGroupKind::Sequence => "sequence",
                    OwnerGroupKind::Cluster => "cluster",
                };
                return Err(OptimizationError::Invalid(format!(
                    "TALA {} {name} child references exceed node limit 10000",
                    guard.location()
                )));
            }
            for index in (0..children.len()).rev() {
                let child = children.get(index).expect("retained backward group slot");
                self.owner_optimizer_push_child(child, guard, seen, stack)?;
            }
        }
        if !self.nodes[parent.0 as usize].is_container {
            return Ok(());
        }
        let map = self.owner_container_map(receiver).ok_or_else(|| {
            OptimizationError::Invalid(OwnerTopologyError::UnknownGraph(receiver).to_string())
        })?;
        // optimization_guard.go653–658 / slices.Backward retains one header,
        // then loads each live backing slot before push's Step. The load drops
        // its backing lock before a context can mutate a future slot. Replacing
        // the map entry cannot retarget this already-retained header.
        let Some(children) = map.get(Some(parent)) else {
            return Ok(());
        };
        const MAX_ENGINE_NODES: usize = 10_000;
        if children.len() > MAX_ENGINE_NODES {
            return Err(OptimizationError::Invalid(format!(
                "TALA {} container child references exceed node limit {}",
                guard.location(),
                MAX_ENGINE_NODES
            )));
        }
        for index in (0..children.len()).rev() {
            let child = children.get(index).expect("live logical header slot");
            self.owner_optimizer_push_child(child, guard, seen, stack)?;
        }
        Ok(())
    }

    fn owner_optimizer_push_child(
        &mut self,
        child: Option<NodeId>,
        guard: &mut OptimizationWorkGuard<'_>,
        seen: &mut BTreeSet<NodeId>,
        stack: &mut Vec<NodeId>,
    ) -> Result<(), OptimizationError> {
        step(self, guard)?;
        let child = child.ok_or_else(|| {
            OptimizationError::Invalid(format!("TALA {} found a nil descendant", guard.location()))
        })?;
        if seen.contains(&child) {
            return Ok(());
        }
        if seen.len() >= 10_000 {
            return Err(OptimizationError::Invalid(format!(
                "TALA {} descendant count exceeds limit {}",
                guard.location(),
                10_000
            )));
        }
        seen.insert(child);
        stack.push(child);
        Ok(())
    }
}
