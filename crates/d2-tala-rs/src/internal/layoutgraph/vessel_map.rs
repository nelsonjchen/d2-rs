// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dense bindings for the numerical cluster/sequence indices. Go follows
//! Node.Cluster/Sequence.Vessel directly (layoutgraph/node.go:2344–2351).
//! These bindings adapt that pointer to a stable Rust arena identity; they
//! are authoritative storage, not a cache of the mutable Vessel field.

use super::TopologyVessel;
use std::fmt;

#[derive(Clone)]
pub(super) struct VesselMap<T> {
    clusters: Vec<Option<(TopologyVessel, T)>>,
    sequences: Vec<Option<(TopologyVessel, T)>>,
}

impl<T> Default for VesselMap<T> {
    fn default() -> Self {
        Self {
            clusters: Vec::new(),
            sequences: Vec::new(),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for VesselMap<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<T> VesselMap<T> {
    #[inline]
    pub(super) fn get(&self, key: &TopologyVessel) -> Option<&T> {
        let (slots, index) = match *key {
            TopologyVessel::Cluster(index) => (&self.clusters, index),
            TopologyVessel::Sequence(index) => (&self.sequences, index),
        };
        slots.get(index)?.as_ref().map(|(_, value)| value)
    }

    pub(super) fn insert(&mut self, key: TopologyVessel, value: T) -> Option<T> {
        let (slots, index) = match key {
            TopologyVessel::Cluster(index) => (&mut self.clusters, index),
            TopologyVessel::Sequence(index) => (&mut self.sequences, index),
        };
        // Callers validate these indices against the actual group slices.
        if slots.len() <= index {
            slots.resize_with(index + 1, || None);
        }
        slots[index].replace((key, value)).map(|(_, value)| value)
    }

    pub(super) fn remove(&mut self, key: &TopologyVessel) -> Option<T> {
        let (slots, index) = match *key {
            TopologyVessel::Cluster(index) => (&mut self.clusters, index),
            TopologyVessel::Sequence(index) => (&mut self.sequences, index),
        };
        slots.get_mut(index)?.take().map(|(_, value)| value)
    }

    /// Preserve the previous BTreeMap order: clusters before sequences, then
    /// increasing index. Reverse vessel lookup must keep its first match.
    pub(super) fn iter(&self) -> impl Iterator<Item = (&TopologyVessel, &T)> {
        self.clusters
            .iter()
            .chain(self.sequences.iter())
            .flatten()
            .map(|(key, value)| (key, value))
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&TopologyVessel, &mut T) -> bool) {
        for slot in self.clusters.iter_mut().chain(self.sequences.iter_mut()) {
            if let Some((key, value)) = slot
                && !keep(key, value)
            {
                *slot = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn vessel_alias_order_nil_and_discovery_reuse_remain_live() {
        let mut input = Graph::default();
        let [member, vessel, replacement] = ["member", "vessel", "replacement"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 20.0,
                    height: 30.0,
                },
            ))
        });
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.clusters = (0..3)
            .map(|index| ClusterState {
                members: vec![member],
                arrangement: ClusterArrangement::Row,
                desired_arrangement: ClusterArrangement::Row,
                padding: 0.0,
                vessel_tala_id: 100 + index,
                fixed_size: false,
            })
            .collect();
        owner.sequences.push(SequenceState {
            members: vec![member],
            vessel_tala_id: 200,
            container: None,
            has_edge_abductions: false,
        });
        let c0 = TopologyVessel::Cluster(0);
        let c1 = TopologyVessel::Cluster(1);
        let c2 = TopologyVessel::Cluster(2);
        let s0 = TopologyVessel::Sequence(0);
        for reference in [s0, c2, c0] {
            owner
                .set_topology_vessel_reference(reference, Some(vessel))
                .unwrap();
        }
        assert_eq!(owner.topology_node_vessel(vessel), Some(c0));
        owner.set_topology_vessel_reference(c0, None).unwrap();
        assert_eq!(owner.topology_vessel_reference(c0), Some(None));
        assert_eq!(owner.topology_vessel_reference(c1), None);
        assert_eq!(owner.topology_node_vessel(vessel), Some(c2));

        let group = owner.new_owner_group(OwnerGroupKind::Cluster);
        let members = owner
            .allocate_owner_member_header(&[Some(member)], 1)
            .unwrap();
        owner
            .set_owner_group_fields(
                group,
                OwnerGroupFieldView {
                    vessel: Some(vessel),
                    nodes: members,
                    graph: None,
                    container: None,
                },
            )
            .unwrap();
        owner.bind_owner_group_reference(c2, group).unwrap();
        let retained = owner.retain_owner_group(group).unwrap();
        let snapshot = owner.owner_topology_snapshot();
        owner
            .set_topology_vessel_reference(c0, Some(replacement))
            .unwrap();
        let mut fields = retained.fields();
        fields.vessel = Some(replacement);
        retained.set_fields(fields);
        assert_eq!(owner.topology_vessel_node(c2), Some(replacement));
        assert_eq!(owner.topology_node_vessel(vessel), Some(s0));
        owner.restore_owner_topology_snapshot(snapshot);
        assert_eq!(owner.topology_vessel_reference(c0), Some(None));
        assert_eq!(owner.topology_vessel_node(c2), Some(replacement));
        owner
            .set_topology_vessel_reference(c2, Some(vessel))
            .unwrap();
        assert_eq!(owner.topology_node_vessel(vessel), Some(c2));

        owner.begin_owner_group_discovery(OwnerGroupKind::Cluster);
        assert_eq!(owner.topology_vessel_reference(c0), None);
        assert_eq!(owner.topology_vessel_reference(c2), None);
        assert_eq!(owner.topology_vessel_node(s0), Some(vessel));
        assert_eq!(retained.vessel(), Some(vessel));
        owner
            .set_topology_vessel_reference(c1, Some(replacement))
            .unwrap();
        let cloned = owner.clone();
        owner.set_topology_vessel_reference(c1, None).unwrap();
        assert_eq!(cloned.topology_vessel_node(c1), Some(replacement));
        assert_eq!(owner.topology_vessel_reference(c1), Some(None));
    }
}
