// SPDX-License-Identifier: MPL-2.0
//! sequence.go::Graph.SyncSequences over actual owner records and graph maps.
use super::super::owner_groups::{OwnerGroupId, OwnerGroupKind};
use super::super::shared_topology::{OwnerGraphId, OwnerGroupMap};
use super::*;

#[cfg(test)]
thread_local! {
    pub(super) static VISITS: RefCell<Option<Vec<(NodeId, Option<OwnerGroupId>)>>> = const { RefCell::new(None) };
}
const NIL_POINTER: &str = "runtime error: invalid memory address or nil pointer dereference";

impl ArenaGraph {
    /// Only dispatch the numerical adapter when its current roots and every
    /// local sequence have actual corresponding owner records. Compact-only
    /// vessels still use their existing numerical/projection implementation.
    pub(super) fn owner_sequence_sync_ready(&self) -> bool {
        if !self.shared_topology_initialized() {
            return false;
        }
        let root = OwnerGraphId(0);
        let Ok(map) = self.owner_graph_group_map(root, OwnerGroupKind::Sequence) else {
            return false;
        };
        if self.sequences.is_empty() && map.len() == 0 {
            return false;
        }
        if !self.sequences.iter().enumerate().all(|(i, _)| {
            self.owner_group_binding(TopologyVessel::Sequence(i))
                .is_some()
        }) {
            return false;
        }
        let Ok(header) = self.owner_graph_member_header(root) else {
            return false;
        };
        if header.len() != self.node_order.len()
            || self
                .node_order
                .iter()
                .enumerate()
                .any(|(i, &node)| header.get(i) != Some(Some(node)))
        {
            return false;
        }
        true
    }

    pub(super) fn sync_owner_graph_sequences(&mut self, receiver: OwnerGraphId) {
        let sequences = self
            .owner_graph_group_map(receiver, OwnerGroupKind::Sequence)
            .expect("known sequence receiver");
        // Source returns before touching nil roots or their Graph fields.
        if sequences.len() == 0 {
            return;
        }
        let roots = self
            .owner_graph_member_header(receiver)
            .expect("known graph roots");
        for i in 0..roots.len() {
            self.owner_sequence_postorder(roots.get(i).expect("logical root"), &sequences);
        }
    }

    fn owner_sequence_postorder(&mut self, node: Option<NodeId>, sequences: &OwnerGroupMap) {
        let node = node.expect(NIL_POINTER);
        if self.nodes[node.0 as usize].is_container {
            let owner = self.topology_node_owner(node).expect(NIL_POINTER);
            let containers = self.owner_container_map(owner).expect("known node Graph");
            if let Some(children) = containers.get(Some(node)) {
                for i in 0..children.len() {
                    self.owner_sequence_postorder(
                        children.get(i).expect("logical child"),
                        sequences,
                    );
                }
            }
        }
        let owner = self.topology_node_owner(node).expect(NIL_POINTER);
        if self.owner_is_cluster_vessel(node) {
            let group = self
                .owner_graph_group_entry(owner, OwnerGroupKind::Cluster, Some(node))
                .expect("known node Graph")
                .flatten()
                .expect(NIL_POINTER);
            let fields = self.owner_group_fields(group).expect("known cluster");
            for i in 0..fields.nodes.len() {
                self.owner_sequence_postorder(
                    fields.nodes.get(i).expect("logical cluster member"),
                    sequences,
                );
            }
        } else if let Some(group) = self
            .owner_graph_group_entry(owner, OwnerGroupKind::Sequence, Some(node))
            .expect("known node Graph")
        {
            let fields = self
                .owner_group_fields(group.expect(NIL_POINTER))
                .expect("known sequence");
            for i in 0..fields.nodes.len() {
                self.owner_sequence_postorder(
                    fields.nodes.get(i).expect("logical sequence member"),
                    sequences,
                );
            }
        }
        // No global visited set: each root/path can reach and synchronize the
        // same object again, after earlier synchronization changed geometry.
        if let Some(group) = self
            .owner_group_map_entry(sequences, Some(node))
            .expect("known receiver map")
        {
            #[cfg(test)]
            VISITS.with(|events| {
                if let Some(events) = events.borrow_mut().as_mut() {
                    events.push((node, group));
                }
            });
            self.sync_owner_sequence_geometry(group);
        }
    }

    fn sync_owner_sequence_geometry(&mut self, group: Option<OwnerGroupId>) {
        let group = group
            .unwrap_or_else(|| panic!("layout invariant violated: sequence is missing its vessel"));
        let fields = self.owner_group_fields(group).expect("known sequence");
        let vessel = fields
            .vessel
            .unwrap_or_else(|| panic!("layout invariant violated: sequence is missing its vessel"));
        let mut members = Vec::with_capacity(fields.nodes.len());
        for i in 0..fields.nodes.len() {
            members.push(
                fields
                    .nodes
                    .get(i)
                    .expect("logical sequence member")
                    .unwrap_or_else(|| {
                        panic!("layout invariant violated: sequence contains a nil step")
                    }),
            );
        }
        let size = sequence_geometry(members.iter().map(|&n| self.nodes[n.0 as usize].rect.size));
        self.nodes[vessel.0 as usize].rect.size = size;
        // Maintain only the legacy view actually bound to this group record.
        for i in 0..self.sequences.len() {
            if self.owner_group_binding(TopologyVessel::Sequence(i)) == Some(group) {
                self.serialized_vessel_sizes_by_tala
                    .insert(self.sequences[i].vessel_tala_id, size);
            }
        }
        if let Some(mut point) = self.position(vessel) {
            for node in members {
                self.set_position(node, point);
                point.x += sequence_advance(self.nodes[node.0 as usize].rect.size.width);
            }
        }
    }
}
