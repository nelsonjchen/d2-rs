// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

impl ArenaGraph {
    /// grouping/clusters.go CreateVessel, AddCluster and abductClusterEdges.
    /// Publish real owner entities at discovery, before PreprocessHubs and
    /// recursive placement can observe their pointers and container headers.
    pub(super) fn publish_cluster_creation(
        &mut self,
        index: usize,
    ) -> Result<(), OwnerTopologyError> {
        let cluster = self.clusters[index].clone();
        let members = &cluster.members;
        let container = self.nodes[members[0].0 as usize].container;
        let mut fresh = Graph::default();
        let local = fresh.add_node(crate::Node::new(
            String::new(),
            self.cluster_vessel_size(index),
        ));
        fresh.tala_id_overrides[local.0 as usize] = Some(cluster.vessel_tala_id);
        let mut entity = ArenaGraph::from_input(&fresh).nodes.remove(0);
        entity.container = container;
        entity.position = self.pending_cluster_vessel_positions.get(&index).copied();
        if let Some(point) = entity.position {
            entity.rect.origin = point;
        }
        let vessel = self.retain_topology_node(entity);
        let group = self.new_owner_group(OwnerGroupKind::Cluster);
        let nodes = self.allocate_owner_member_header(
            &members.iter().copied().map(Some).collect::<Vec<_>>(),
            members.len(),
        )?;
        self.set_owner_group_fields(
            group,
            OwnerGroupFieldView {
                vessel: Some(vessel),
                nodes,
                graph: Some(OwnerGraphId(0)),
                container,
            },
        )?;
        self.bind_owner_group_reference(TopologyVessel::Cluster(index), group)?;
        self.bind_topology_vessel(TopologyVessel::Cluster(index), vessel)?;
        self.set_owner_cluster_vessel_flag(vessel, true)?;
        self.add_owner_graph_node(
            OwnerGraphId(0),
            vessel,
            &mut OwnerGraphOwnershipJournal::default(),
        )?;
        self.publish_owner_cluster_children(group)?;

        let mut records = Vec::new();
        for edge in self.edge_order.read() {
            let mut from = self.owner_edge_endpoint(edge, false)?.unwrap();
            let to = self.owner_edge_endpoint(edge, true)?.unwrap();
            if members.contains(&from) {
                records.push(OwnerEdgeAbduction {
                    source_edge: Some(edge),
                    originally_from: Some(from),
                    originally_to: None,
                    current_from: Some(vessel),
                    current_to: Some(to),
                });
                self.reconnect_owner_edge_endpoint(edge, vessel, false);
                from = vessel;
            }
            if members.contains(&to) {
                records.push(OwnerEdgeAbduction {
                    source_edge: Some(edge),
                    originally_from: None,
                    originally_to: Some(to),
                    current_from: Some(from),
                    current_to: Some(vessel),
                });
                self.reconnect_owner_edge_endpoint(edge, vessel, true);
            }
        }
        let records = self.register_owner_edge_abductions(records, true)?;
        let header = self.allocate_owner_abduction_header(
            &records
                .records
                .iter()
                .copied()
                .map(Some)
                .collect::<Vec<_>>(),
            records.records.len(),
            true,
        )?;
        self.set_owner_cluster_policy(
            group,
            super::owner_groups::OwnerClusterPolicy {
                arrangement: format!("{:?}", cluster.arrangement),
                desired_arrangement: format!("{:?}", cluster.desired_arrangement),
                padding: cluster.padding,
                fixed_size: cluster.fixed_size,
                edge_abductions: Some(header),
            },
        )?;
        let map = self.owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)?;
        self.write_owner_group_map_entry(&map, Some(vessel), Some(group))?;
        Ok(())
    }
}
