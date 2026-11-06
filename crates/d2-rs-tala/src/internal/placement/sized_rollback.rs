// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! The dense workspace is an adapter for captured canonical geometry. Save its
//! corresponding fields without charging a second source snapshot traversal.
use super::super::limits::OptimizationError;
use super::super::owner_score_session::{OwnerNodeGeometry, OwnerOptimizerMutationSnapshot};
use super::super::shared_topology::OwnerNodePointSnapshot;
use super::super::{ArenaGraph, NodeId, OwnerScoreSession};
use std::collections::BTreeSet;

struct LocalGeometry {
    node: NodeId,
    geometry: OwnerNodeGeometry,
    point: Option<OwnerNodePointSnapshot>,
}
enum ExternalLocation {
    Container(usize),
    ContainerChild(i64, usize),
    AggregateChild(i64, usize),
}
pub(super) struct WorkspaceSnapshot {
    nodes: Vec<LocalGeometry>,
    external: Vec<(ExternalLocation, OwnerNodeGeometry)>,
    costs: super::super::PlacementCostSnapshot,
}
impl WorkspaceSnapshot {
    pub(super) fn capture(
        workspace: &ArenaGraph,
        session: &OwnerScoreSession<'_>,
        captured: &OwnerOptimizerMutationSnapshot,
    ) -> Result<Self, OptimizationError> {
        let selected = captured.captured_nodes().collect::<BTreeSet<_>>();
        let mut nodes = Vec::new();
        for (index, canonical) in session.current_mapping().nodes().iter().enumerate() {
            if !selected.contains(canonical) {
                continue;
            }
            let node = NodeId(index as u32);
            let live = workspace.nodes.get(index).ok_or_else(|| {
                OptimizationError::Invalid(format!("missing workspace node {node:?}"))
            })?;
            let point = match workspace.owner_node_point(node) {
                Ok(point) => Some(point.snapshot()),
                Err(super::super::OwnerTopologyError::UnboundNodePoint(_)) => None,
                Err(error) => return Err(OptimizationError::Invalid(error.to_string())),
            };
            let mut geometry = OwnerNodeGeometry::from_node(live);
            geometry.position = workspace.position(node);
            nodes.push(LocalGeometry {
                node,
                geometry,
                point,
            });
        }
        let mut external = Vec::new();
        for (index, node) in workspace.transaction_external_containers.iter().enumerate() {
            if node
                .owner_geometry_source
                .is_some_and(|id| selected.contains(&id))
            {
                external.push((
                    ExternalLocation::Container(index),
                    OwnerNodeGeometry::from_node(node),
                ));
            }
        }
        for (&key, children) in &workspace.transaction_external_container_children {
            for (index, node) in children.iter().enumerate() {
                if node
                    .owner_geometry_source
                    .is_some_and(|id| selected.contains(&id))
                {
                    external.push((
                        ExternalLocation::ContainerChild(key, index),
                        OwnerNodeGeometry::from_node(node),
                    ));
                }
            }
        }
        for (&key, children) in &workspace.transaction_external_aggregate_children {
            for (index, node) in children.iter().enumerate() {
                if node
                    .owner_geometry_source
                    .is_some_and(|id| selected.contains(&id))
                {
                    external.push((
                        ExternalLocation::AggregateChild(key, index),
                        OwnerNodeGeometry::from_node(node),
                    ));
                }
            }
        }
        Ok(Self {
            nodes,
            external,
            costs: workspace.cost_cache.snapshot_placement(),
        })
    }
    pub(super) fn restore(&self, workspace: &mut ArenaGraph) {
        for saved in &self.nodes {
            if let Some(point) = &saved.point {
                workspace
                    .restore_owner_node_point(saved.node, point)
                    .expect("captured workspace point binding");
            } else {
                workspace.nodes[saved.node.0 as usize].position = saved.geometry.position;
            }
            workspace.nodes[saved.node.0 as usize].rect = saved.geometry.rect;
        }
        for (location, geometry) in &self.external {
            let node = match *location {
                ExternalLocation::Container(index) => {
                    &mut workspace.transaction_external_containers[index]
                }
                ExternalLocation::ContainerChild(key, index) => &mut workspace
                    .transaction_external_container_children
                    .get_mut(&key)
                    .expect("captured container projection")[index],
                ExternalLocation::AggregateChild(key, index) => &mut workspace
                    .transaction_external_aggregate_children
                    .get_mut(&key)
                    .expect("captured aggregate projection")[index],
            };
            node.position = geometry.position;
            node.rect = geometry.rect;
        }
        self.costs.restore();
    }
}
