// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Diagnostic observation of actual retained records before/after AddSequences.
#[cfg(feature = "diagnostic-traces")]
impl ArenaGraph {
    pub(super) fn sequence_lifecycle_observer(&self) -> impl FnOnce(&Self) -> serde_json::Value + use<> {
        let initial_nodes = self.nodes.len();
        let initial_groups = self.shared_topology.groups.len();
        let retained_members = self.owner_graph_member_header(OwnerGraphId(0)).unwrap();
        let retained_children = self.containers.keys().map(|key| (key, self.containers.get_header(&key).unwrap())).collect::<Vec<_>>();
        move |graph| {
            use serde_json::{Value, json};
            let nr = |node: Option<NodeId>| node.map(|node| {
                if (node.0 as usize) < initial_nodes { format!("old/{}", graph.nodes[node.0 as usize].tala_id) }
                else { format!("new/{}", node.0 as usize - initial_nodes) }
            });
            let er = |edge: Option<EdgeId>| edge.map(|edge| format!("edge/{}", edge.0));
            let gr = |group: Option<OwnerGroupId>| group.map(|id| {
                if id.index() < initial_groups {
                    let vessel = graph.owner_group_fields(id).unwrap().vessel.unwrap();
                    format!("old-{}/{}", if id.kind() == OwnerGroupKind::Sequence { "sequence" } else { "cluster" }, graph.nodes[vessel.0 as usize].tala_id)
                } else { format!("new-sequence/{}", id.index() - initial_groups) }
            });
            let owner = |value: Option<OwnerGraphId>| match value { None => "nil", Some(OwnerGraphId(0)) => "root", _ => "other" };
            let nh = |header: &SharedNodeSlice| json!({"nil":header.is_nil(), "len":header.len(), "cap":header.capacity(),
                "slots":(0..header.capacity()).map(|i|nr(header.read_capacity_slot(i).unwrap())).collect::<Vec<_>>()});
            let eh = |header: OwnerIncidentHeader| json!({"nil":header.is_nil(), "len":header.len(), "cap":header.capacity(),
                "slots":(0..header.capacity()).map(|i|er(header.read_capacity_slot(i).unwrap())).collect::<Vec<_>>()});
            let bits = |value: f64| format!("{:016x}", value.to_bits());
            let mut nodes = graph.nodes.iter().enumerate().map(|(index, node)| {
                let id = NodeId(index as u32);
                json!({"ref":nr(Some(id)), "id":node.tala_id, "owner":owner(graph.topology_node_owner(id)),
                    "container":nr(node.container), "point":graph.position(id).map(|p|[bits(p.x),bits(p.y)]),
                    "size":[bits(node.rect.size.width),bits(node.rect.size.height)],
                    "sequence":gr(graph.owner_node_group_id(id,OwnerGroupKind::Sequence).unwrap()),
                    "cluster":gr(graph.owner_node_group_id(id,OwnerGroupKind::Cluster).unwrap()),
                    "edges":eh(graph.owner_node_incident_header(id).unwrap())})
            }).collect::<Vec<_>>();
            nodes.sort_by(|a,b|a["ref"].as_str().cmp(&b["ref"].as_str()));
            let edges = graph.edge_records.iter().enumerate().map(|(index, edge)| {
                let id = EdgeId(index as u32);
                json!({"ref":er(Some(id)),"id":edge.tala_id,"from":nr(graph.owner_edge_endpoint(id,false).unwrap()),"to":nr(graph.owner_edge_endpoint(id,true).unwrap())})
            }).collect::<Vec<_>>();
            let mut sequences = graph.shared_topology.groups.iter().filter(|(id,_)|id.kind()==OwnerGroupKind::Sequence).map(|(id,_)| {
                let fields = graph.owner_group_fields(*id).unwrap();
                let abductions = graph.owner_sequence_abductions(*id).unwrap();
                let abductions = abductions.map_or_else(||json!({"nil":true,"len":0,"cap":0,"slots":[]}), |header| {
                    let slots = (0..header.capacity()).map(|i|header.read_capacity_slot(i).unwrap().map_or(Value::Null, |id| {
                        let record = graph.owner_edge_abduction(id).unwrap();
                        json!({"edge":er(record.source_edge),"original_from":nr(record.originally_from),"original_to":nr(record.originally_to),
                            "current_from":nr(record.current_from),"current_to":nr(record.current_to)})
                    })).collect::<Vec<_>>();
                    json!({"nil":header.is_nil(),"len":header.len(),"cap":header.capacity(),"slots":slots})
                });
                json!({"ref":gr(Some(*id)),"vessel":nr(fields.vessel),"owner":owner(fields.graph),"container":nr(fields.container),"nodes":nh(&fields.nodes.slice),"abductions":abductions})
            }).collect::<Vec<_>>();
            sequences.sort_by(|a,b|a["ref"].as_str().cmp(&b["ref"].as_str()));
            let mut member_aliases = Vec::new();
            for (left, _) in graph.shared_topology.groups.iter().filter(|(id,_)|id.kind()==OwnerGroupKind::Sequence) {
                for (right, _) in graph.shared_topology.groups.iter().filter(|(id,_)|id.kind()==OwnerGroupKind::Sequence) {
                    let a = gr(Some(*left)).unwrap();
                    let b = gr(Some(*right)).unwrap();
                    let lh = graph.owner_group_fields(*left).unwrap().nodes;
                    let rh = graph.owner_group_fields(*right).unwrap().nodes;
                    if a < b && lh.len() > 0 && rh.len() > 0 && lh.slot_address(0)==rh.slot_address(0) {
                        member_aliases.push([a,b]);
                    }
                }
            }
            member_aliases.sort();
            let children = |headers: Vec<(Option<NodeId>, SharedNodeSlice)>| {
                let mut rows = headers.into_iter().map(|(key,header)|(nr(key),nh(&header))).collect::<Vec<_>>();
                rows.sort_by(|a,b|a.0.cmp(&b.0));
                rows.into_iter().map(|(key,nodes)|json!({"key":key,"nodes":nodes})).collect::<Vec<_>>()
            };
            json!({"nodes":nodes,"edges":edges,"sequences":sequences,"member_aliases":member_aliases,
                "members":nh(&graph.owner_graph_member_header(OwnerGraphId(0)).unwrap().slice),
                "retained_members":nh(&retained_members.slice),"graph_edges":eh(graph.owner_graph_edge_header(OwnerGraphId(0)).unwrap()),
                "containers":children(graph.containers.keys().map(|key|(key,graph.containers.get_header(&key).unwrap())).collect()),
                "retained_containers":children(retained_children)})
        }
    }
}
