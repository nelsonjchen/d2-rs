// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

impl ArenaGraph {
    /// grouping/sequences.go buildSequence/addSequence. The numerical group
    /// has already resized steps and disconnected defining edges; publish the
    /// actual allocation and all remaining owner mutations at this boundary.
    pub(super) fn publish_sequence_creation(&mut self, index: usize, remembered: Option<OwnerMemberHeader>) -> Result<(), OwnerTopologyError> {
        let sequence = self.sequences[index].clone();
        let members = &sequence.members;
        let mut fresh = Graph::default();
        let local = fresh.add_node(crate::Node::new(String::new(), Size::default()));
        fresh.tala_id_overrides[local.0 as usize] = Some(sequence.vessel_tala_id);
        let entity = ArenaGraph::from_input(&fresh).nodes.remove(0);
        let vessel = self.retain_topology_node(entity);
        let group = self.new_owner_group(OwnerGroupKind::Sequence);
        let header = if let Some(header) = remembered { header } else {
            let mut header = self.allocate_owner_member_header(&[Some(members[0])], 1)?;
            for &member in &members[1..] {
                header = self.append_owner_member_header(&header, &[Some(member)])?;
            }
            header
        };
        self.set_owner_group_fields(group, OwnerGroupFieldView {
            vessel: Some(vessel), nodes: header, graph: Some(OwnerGraphId(0)), container: sequence.container,
        })?;
        self.bind_owner_group_reference(TopologyVessel::Sequence(index), group)?;
        self.bind_topology_vessel(TopologyVessel::Sequence(index), vessel)?;

        // NewNode.TopLeft is nil, so SyncGeometry only resizes its box.
        let mut size = Size::default();
        let mut offset = 0.0_f64;
        for &member in members {
            let child = self.nodes[member.0 as usize].rect.size;
            size.width = size.width.max(offset + child.width.max(0.0));
            offset += if child.width <= 0.0 { 0.0 } else if child.width <= 35.0 { child.width - child.width / 2.0 } else { child.width - 35.0 };
            size.height = size.height.max(child.height.max(0.0));
        }
        self.nodes[vessel.0 as usize].rect.size = size;
        let member_set = members.iter().copied().collect::<BTreeSet<_>>();
        let mut records = Vec::new();
        for edge in self.edge_order.read() {
            let mut from = self.owner_edge_endpoint(edge, false)?.unwrap();
            let to = self.owner_edge_endpoint(edge, true)?.unwrap();
            let from_member = member_set.contains(&from);
            let to_member = member_set.contains(&to);
            if from_member && to_member { continue; }
            if from_member {
                records.push(OwnerEdgeAbduction { source_edge: Some(edge), originally_from: Some(from), originally_to: None,
                    current_from: Some(vessel), current_to: Some(to) });
                self.reconnect_owner_edge_endpoint(edge, vessel, false);
                from = vessel;
            }
            if to_member {
                records.push(OwnerEdgeAbduction { source_edge: Some(edge), originally_from: None, originally_to: Some(to),
                    current_from: Some(from), current_to: Some(vessel) });
                self.reconnect_owner_edge_endpoint(edge, vessel, true);
            }
        }
        let records = self.register_owner_edge_abductions(records, true)?;
        let mut abductions = SharedAbductionSlice::nil();
        for id in records.records {
            abductions = abductions.append(&[Some(id)]).expect("sequence abduction append");
        }
        self.set_owner_sequence_abductions(group, Some(OwnerAbductionHeader {
            arena: self.topology_arena_id().unwrap(), scope: self.shared_topology.member_scope, slice: abductions,
        }))?;

        let mut point = Some(Point { x: f64::INFINITY, y: f64::INFINITY });
        for &member in members {
            let Some(child) = self.position(member) else { point = None; break; };
            let value = point.as_mut().unwrap();
            value.x = value.x.min(child.x);
            value.y = value.y.min(child.y);
        }
        let point_header = self.allocate_owner_node_point(point);
        self.bind_owner_node_point(vessel, point_header)?;
        if let Some(point) = point { self.nodes[vessel.0 as usize].rect.origin = point; }
        self.add_owner_graph_node(OwnerGraphId(0), vessel, &mut OwnerGraphOwnershipJournal::default())?;
        self.append_owner_container_node(sequence.container, vessel)?;
        for &member in members {
            self.set_owner_node_group(member, OwnerGroupKind::Sequence, Some(group))?;
        }
        let children = self.containers.get_header(&sequence.container).unwrap();
        let mut filtered = SharedNodeSlice::nil();
        for child in children.iter_slots() {
            let node = child.unwrap();
            if self.owner_node_group_id(node, OwnerGroupKind::Sequence)? != Some(group) {
                filtered = filtered.append_node(node).expect("sequence child append");
            }
        }
        // The pinned Go 1.27 compiler keeps this unaliased local in a
        // four-pointer stack buffer and publishes via moveSliceNoCap.
        // Above four entries it has already escaped through growslice.
        if !filtered.is_empty() && filtered.len() <= 4 {
            filtered = SharedNodeSlice::from_nodes(&filtered.iter_slots().map(Option::unwrap).collect::<Vec<_>>()).expect("published sequence children");
        }
        self.containers.replace_header(sequence.container, filtered);
        for &member in members {
            let old = self.shared_topology.views[0].as_ref().unwrap().nodes.header();
            let capacity = old.len().checked_sub(1).expect("RemoveNode from nonempty graph");
            let mut updated = SharedNodeSlice::with_len_cap(0, capacity).expect("RemoveNode capacity");
            for node in old.iter_slots() {
                if node != Some(member) { updated = updated.append(&[node]).expect("RemoveNode append"); }
            }
            self.shared_topology.views[0].as_mut().unwrap().nodes.replace(updated);
            self.nodes[member.0 as usize].container = None;
        }
        let map = self.owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Sequence)?;
        self.write_owner_group_map_entry(&map, Some(vessel), Some(group))?;
        self.descendant_cache.clear();
        Ok(())
    }
}
