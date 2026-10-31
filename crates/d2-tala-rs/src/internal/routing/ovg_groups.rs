// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Routing reads actual group pointers, including inactive decoded groups.
use super::{ArenaGraph, NodeId};
use crate::internal::owner_groups::{OwnerGroupId, OwnerGroupKind};
use crate::internal::shared_topology::OwnerTopologyError;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Group {
    Owned(OwnerGroupId),
    Legacy(OwnerGroupKind, usize),
}
pub(super) fn group(graph: &ArenaGraph, node: NodeId, kind: OwnerGroupKind) -> Option<Group> {
    match graph.owner_node_group_id(node, kind) {
        Ok(group) => group.map(Group::Owned),
        // Ordinary numerical scopes can have a group before owner binding.
        // An explicit nil raw pointer returned above must not use this fallback.
        Err(OwnerTopologyError::UnboundVessel(_)) => {
            let n = &graph.nodes[node.0 as usize];
            match kind {
                OwnerGroupKind::Cluster => n.cluster,
                OwnerGroupKind::Sequence => n.sequence,
            }
            .map(|i| Group::Legacy(kind, i))
        }
        Err(error) => panic!("routing group pointer: {error:?}"),
    }
}
impl Group {
    pub(super) fn first(self, graph: &ArenaGraph) -> Option<NodeId> {
        match self {
            Self::Owned(id) => graph
                .owner_group_fields(id)
                .expect("routing group fields")
                .nodes
                .get(0)
                .expect("runtime error: index out of range [0] with length 0"),
            Self::Legacy(kind, i) => {
                let members = match kind {
                    OwnerGroupKind::Cluster => &graph.clusters[i].members,
                    OwnerGroupKind::Sequence => &graph.sequences[i].members,
                };
                Some(
                    *members
                        .first()
                        .expect("runtime error: index out of range [0] with length 0"),
                )
            }
        }
    }
    pub(super) fn last(self, graph: &ArenaGraph) -> Option<NodeId> {
        match self {
            Self::Owned(id) => {
                let fields = graph.owner_group_fields(id).expect("routing group fields");
                let index = fields
                    .nodes
                    .len()
                    .checked_sub(1)
                    .expect("runtime error: index out of range [-1]");
                fields.nodes.get(index).expect("routing group last slot")
            }
            Self::Legacy(kind, i) => {
                let members = match kind {
                    OwnerGroupKind::Cluster => &graph.clusters[i].members,
                    OwnerGroupKind::Sequence => &graph.sequences[i].members,
                };
                Some(
                    *members
                        .last()
                        .expect("runtime error: index out of range [-1]"),
                )
            }
        }
    }
    pub(super) fn members(self, graph: &ArenaGraph) -> Vec<NodeId> {
        match self {
            Self::Owned(id) => graph
                .owner_group_fields(id)
                .expect("routing group fields")
                .nodes
                .iter_slots()
                .map(|node| {
                    node.expect("runtime error: invalid memory address or nil pointer dereference")
                })
                .collect(),
            Self::Legacy(OwnerGroupKind::Cluster, i) => graph.clusters[i].members.clone(),
            Self::Legacy(OwnerGroupKind::Sequence, i) => graph.sequences[i].members.clone(),
        }
    }
    pub(super) fn vessel(self, graph: &ArenaGraph) -> Option<NodeId> {
        match self {
            Self::Owned(id) => {
                graph
                    .owner_group_fields(id)
                    .expect("routing group fields")
                    .vessel
            }
            Self::Legacy(kind, i) => {
                let id = match kind {
                    OwnerGroupKind::Cluster => graph.clusters[i].vessel_tala_id,
                    OwnerGroupKind::Sequence => graph.sequences[i].vessel_tala_id,
                };
                graph
                    .nodes
                    .iter()
                    .find(|n| n.tala_id == id)
                    .map(|n| n.input_id)
            }
        }
    }
}
