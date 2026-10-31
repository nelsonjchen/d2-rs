// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! compaction.go / types.go: arm rollback only after the complete live
//! position snapshot. Validation and interrupted capture are outside the defer.
use super::shared_topology::{OwnerMemberHeader, OwnerNodePointSnapshot};
use super::*;

pub(super) struct CapturedCompactionPositions {
    positions: Vec<(NodeId, Option<Point>, Point, Option<OwnerNodePointSnapshot>)>,
}
fn invalid(error: impl std::fmt::Display) -> limits::Error {
    limits::Error::Invalid(error.to_string())
}
impl CapturedCompactionPositions {
    pub(super) fn capture(
        graph: &mut ArenaGraph,
        roots: OwnerMemberHeader,
        context: &dyn limits::Context,
    ) -> Result<Self, limits::Error> {
        // Function arguments in Go retain Graph.Nodes before NewWorkGuard
        // polls the context. Keep that same live backing and logical length.
        let mut guard =
            limits::WorkGuard::new(Some(context), "Compaction", limits::MAX_ENGINE_WORK_UNITS)?;
        let mut seen = BTreeSet::new();
        let mut positions = Vec::new();
        let mut pending = Vec::new();
        fn enqueue(
            graph: &mut ArenaGraph,
            node: Option<NodeId>,
            receiver: Option<OwnerGraphId>,
            seen: &mut BTreeSet<NodeId>,
            positions: &mut Vec<(NodeId, Option<Point>, Point, Option<OwnerNodePointSnapshot>)>,
            pending: &mut Vec<(NodeId, Option<OwnerGraphId>)>,
            guard: &mut limits::WorkGuard<'_>,
        ) -> Result<(), limits::Error> {
            let Some(node) = node else { return Ok(()) };
            if seen.contains(&node) {
                return Ok(());
            }
            if seen.len() >= 10_000 {
                return Err(limits::Error::Invalid(
                    "TALA Compaction unique node count exceeds limit 10000".into(),
                ));
            }
            guard.step()?;
            seen.insert(node);
            let live = graph
                .topology_node(node)
                .ok_or_else(|| invalid(OwnerTopologyError::UnknownNode(node)))?;
            let point = match graph.owner_node_point(node) {
                Ok(point) => Some(point.snapshot()),
                Err(OwnerTopologyError::UnboundNodePoint(_)) => None,
                Err(error) => return Err(invalid(error)),
            };
            positions.push((node, graph.position(node), live.rect.origin, point));
            pending.push((node, receiver));
            Ok(())
        }
        for index in 0..roots.len() {
            let node = roots.get(index).expect("retained root slot");
            let owner = node.and_then(|node| graph.topology_node_owner(node));
            enqueue(
                graph,
                node,
                owner,
                &mut seen,
                &mut positions,
                &mut pending,
                &mut guard,
            )?;
        }
        let mut index = 0;
        while index < pending.len() {
            let (node, receiver) = pending[index];
            index += 1;
            let Some(receiver) = receiver else { continue };
            if graph.nodes[node.0 as usize].is_container {
                let map = graph
                    .owner_container_map(receiver)
                    .ok_or_else(|| invalid(OwnerTopologyError::UnknownGraph(receiver)))?;
                if let Some(children) = map.get(Some(node)) {
                    for i in 0..children.len() {
                        let child = children.get(i).expect("retained child slot");
                        enqueue(
                            graph,
                            child,
                            Some(receiver),
                            &mut seen,
                            &mut positions,
                            &mut pending,
                            &mut guard,
                        )?;
                    }
                }
            }
            for kind in [
                owner_groups::OwnerGroupKind::Cluster,
                owner_groups::OwnerGroupKind::Sequence,
            ] {
                if kind == owner_groups::OwnerGroupKind::Cluster
                    && !graph.owner_is_cluster_vessel(node)
                {
                    continue;
                }
                if let Some(group) = graph
                    .owner_graph_group_entry(receiver, kind, Some(node))
                    .map_err(invalid)?
                    .flatten()
                {
                    let children = graph.owner_group_fields(group).map_err(invalid)?.nodes;
                    for i in 0..children.len() {
                        let child = children.get(i).expect("retained group slot");
                        enqueue(
                            graph,
                            child,
                            Some(receiver),
                            &mut seen,
                            &mut positions,
                            &mut pending,
                            &mut guard,
                        )?;
                    }
                }
            }
        }
        guard.finish()?;
        Ok(Self { positions })
    }
    pub(super) fn restore(&self, graph: &mut ArenaGraph) {
        for (node, position, origin, point) in &self.positions {
            if let Some(point) = point {
                graph
                    .restore_owner_node_point(*node, point)
                    .expect("captured point scope");
            } else {
                graph.nodes[node.0 as usize].position = *position;
            }
            graph.nodes[node.0 as usize].rect.origin = *origin;
        }
    }
}

// Outer compaction additionally restores the receiver's routing costs;
// selected-root trial snapshots restore only points, exactly as types.go does.
pub(super) struct CapturedCompactionState {
    receiver: OwnerGraphId,
    positions: CapturedCompactionPositions,
    routing: (f64, f64, f64),
}
impl CapturedCompactionState {
    pub(super) fn capture(
        graph: &mut ArenaGraph,
        receiver: OwnerGraphId,
        context: &dyn limits::Context,
    ) -> Result<Self, limits::Error> {
        let routing = graph.owner_graph_routing_costs(receiver).map_err(invalid)?;
        let roots = graph.owner_graph_member_header(receiver).map_err(invalid)?;
        let positions = CapturedCompactionPositions::capture(graph, roots, context)?;
        Ok(Self {
            receiver,
            positions,
            routing,
        })
    }
    pub(super) fn restore(&self, graph: &mut ArenaGraph) {
        self.positions.restore(graph);
        graph
            .restore_owner_graph_routing_costs(self.receiver, self.routing)
            .expect("captured routing receiver");
    }
}

pub(super) struct CompactionCheckpoint<'a> {
    graph: &'a mut ArenaGraph,
    state: CapturedCompactionState,
    complete: bool,
}
impl<'a> CompactionCheckpoint<'a> {
    pub(super) fn capture(
        graph: &'a mut ArenaGraph,
        receiver: OwnerGraphId,
        context: &dyn limits::Context,
    ) -> Result<Self, limits::Error> {
        let state = CapturedCompactionState::capture(graph, receiver, context)?;
        Ok(Self {
            graph,
            state,
            complete: false,
        })
    }
    pub(super) fn graph(&mut self) -> &mut ArenaGraph {
        self.graph
    }
    pub(super) fn commit(&mut self) {
        self.complete = true;
    }
}
impl Drop for CompactionCheckpoint<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.state.restore(self.graph);
        }
    }
}
