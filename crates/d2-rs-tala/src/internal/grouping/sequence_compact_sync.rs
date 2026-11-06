// SPDX-License-Identifier: MPL-2.0
//! Source rdfsWalk order for the numerical arena's compact aggregate carriers.
use super::*;

#[derive(Clone, Copy)]
enum CompactSyncRef {
    Node(NodeId),
    Sequence(usize),
    Cluster(usize),
}

impl ArenaGraph {
    /// A compact carrier is meaningful only at an active-root/container/cluster
    /// boundary. Inside a sequence, its first step remains the original member.
    /// Never choose a physical vessel by entity ID: a bound arena handle and a
    /// virtual carrier are distinct representations.
    fn compact_sequence_sync_reference(
        &self,
        node: NodeId,
        within_cluster: Option<usize>,
    ) -> CompactSyncRef {
        if let Some(index) = self.clusters.iter().enumerate().find_map(|(i, g)| {
            (Some(i) != within_cluster
                && g.members.len() > 1
                && g.members
                    .iter()
                    .skip(1)
                    .any(|&m| !self.node_order_contains(m))
                && g.members.first() == Some(&node)
                && self
                    .topology_vessel_node(TopologyVessel::Cluster(i))
                    .is_none_or(|vessel| !self.node_order_contains(vessel)))
            .then_some(i)
        }) {
            return CompactSyncRef::Cluster(index);
        }
        if let Some(index) = self.sequences.iter().enumerate().find_map(|(i, g)| {
            (self.sequence_is_active(g)
                && g.members.first() == Some(&node)
                && self
                    .topology_vessel_node(TopologyVessel::Sequence(i))
                    .is_none_or(|vessel| !self.node_order_contains(vessel)))
            .then_some(i)
        }) {
            return CompactSyncRef::Sequence(index);
        }
        CompactSyncRef::Node(node)
    }

    pub(super) fn sync_compact_sequences(&mut self) {
        if self.sequences.is_empty() {
            return;
        }
        for node in self.node_order.clone() {
            self.compact_sequence_sync_postorder(self.compact_sequence_sync_reference(node, None));
        }
    }

    fn compact_sequence_sync_postorder(&mut self, node: CompactSyncRef) {
        let sequence = match node {
            CompactSyncRef::Node(node) => {
                if self.nodes[node.0 as usize].is_container {
                    let children = self.containers.get(&Some(node)).unwrap_or_default();
                    for child in children {
                        self.compact_sequence_sync_postorder(
                            self.compact_sequence_sync_reference(child, None),
                        );
                    }
                }
                if let Some(index) = self.clusters.iter().enumerate().find_map(|(i, _)| {
                    (self.topology_vessel_node(TopologyVessel::Cluster(i)) == Some(node))
                        .then_some(i)
                }) {
                    for child in self.clusters[index].members.clone() {
                        self.compact_sequence_sync_postorder(
                            self.compact_sequence_sync_reference(child, Some(index)),
                        );
                    }
                    // Source cluster descent takes precedence. Its callback may
                    // still synchronize a sequence keyed by this same vessel.
                    self.sequences.iter().enumerate().find_map(|(i, _)| {
                        (self.topology_vessel_node(TopologyVessel::Sequence(i)) == Some(node))
                            .then_some(i)
                    })
                } else if let Some(index) = self.sequences.iter().enumerate().find_map(|(i, _)| {
                    (self.topology_vessel_node(TopologyVessel::Sequence(i)) == Some(node))
                        .then_some(i)
                }) {
                    for child in self.sequences[index].members.clone() {
                        self.compact_sequence_sync_postorder(CompactSyncRef::Node(child));
                    }
                    Some(index)
                } else {
                    None
                }
            }
            CompactSyncRef::Sequence(index) => {
                for child in self.sequences[index].members.clone() {
                    self.compact_sequence_sync_postorder(CompactSyncRef::Node(child));
                }
                Some(index)
            }
            CompactSyncRef::Cluster(index) => {
                for child in self.clusters[index].members.clone() {
                    self.compact_sequence_sync_postorder(
                        self.compact_sequence_sync_reference(child, Some(index)),
                    );
                }
                None
            }
        };
        if let Some(index) = sequence {
            self.sync_compact_sequence_geometry(index);
        }
    }
}
