// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::*;
use serde_json::{Value, json};

impl ArenaGraph {
    pub(in crate::internal) fn observe_decoded_records(&self, name: &str, input: &Graph) -> Value {
        let node_id = |node: Option<NodeId>| node.map(|node| self.nodes[node.0 as usize].tala_id);
        let edge_wire = input
            .tala_decoded_edge_records
            .iter()
            .map(|(&wire, &edge)| (edge, wire))
            .collect::<BTreeMap<_, _>>();
        let edge = |edge: Option<EdgeId>| {
            edge.map(|edge| json!({"wire":edge_wire[&edge],"id":self.edge_records[edge.0 as usize].tala_id}))
        };
        let node_header = |header: &OwnerMemberHeader| {
            json!({"nil":header.is_nil(),"len":header.len(),"cap":header.capacity(),
            "slots":(0..header.capacity()).map(|i|node_id(header.read_capacity_slot(i).unwrap())).collect::<Vec<_>>()})
        };
        let abductions = |header: &OwnerAbductionHeader| {
            json!({"nil":header.is_nil(),"len":header.len(),"cap":header.capacity(),
            "slots":(0..header.capacity()).map(|i|header.read_capacity_slot(i).unwrap().map(|id| {
                let record=self.owner_edge_abduction(id).unwrap(); json!({"edge":edge(record.source_edge),
                    "original_from":node_id(record.originally_from),"original_to":node_id(record.originally_to),
                    "current_from":node_id(record.current_from),"current_to":node_id(record.current_to)})
            })).collect::<Vec<_>>()})
        };
        let view = self.shared_topology.views[0].as_ref().unwrap();
        let mut groups = Vec::new();
        for (kind, map) in [
            (OwnerGroupKind::Cluster, &view.clusters),
            (OwnerGroupKind::Sequence, &view.sequences),
        ] {
            let mut entries = map.entries();
            entries.sort_by_key(|(node, _)| node_id(*node).unwrap_or(0));
            for (key, group) in entries {
                let group = group.unwrap();
                let fields = self.owner_group_fields(group).unwrap();
                let (header, policy) = match kind {
                    OwnerGroupKind::Cluster => {
                        let policy = self.owner_cluster_policy(group).unwrap();
                        (
                            policy.edge_abductions.unwrap(),
                            json!({"arrangement":policy.arrangement,"desired":policy.desired_arrangement,"fixed":policy.fixed_size,"padding":format!("{:016x}",policy.padding.to_bits())}),
                        )
                    }
                    OwnerGroupKind::Sequence => (
                        self.owner_sequence_abductions(group).unwrap().unwrap(),
                        Value::Null,
                    ),
                };
                groups.push(json!({"kind":if kind==OwnerGroupKind::Cluster{"cluster"}else{"sequence"},"key":node_id(key),
                    "vessel":node_id(fields.vessel),"nodes":node_header(&fields.nodes),"owner":fields.graph==Some(OwnerGraphId(0)),
                    "container":node_id(fields.container),"abductions":abductions(&header),"policy":policy}));
            }
        }
        let mut names = BTreeMap::new();
        let mut order = Vec::new();
        fn register(
            owner: &ArenaGraph,
            tree: OwnerTreeId,
            name: String,
            names: &mut BTreeMap<OwnerTreeId, String>,
            order: &mut Vec<OwnerTreeId>,
        ) {
            names.insert(tree, name.clone());
            order.push(tree);
            let fields = owner.owner_tree(tree).unwrap().fields();
            for i in 0..fields.children.len() {
                register(
                    owner,
                    fields.children.get(i).unwrap().unwrap(),
                    format!("{name}/{i}"),
                    names,
                    order,
                );
            }
        }
        let mut root_entries = view.tree_roots.entries();
        root_entries.sort_by_key(|(node, _)| node_id(*node).unwrap_or(0));
        for (key, roots) in &root_entries {
            for i in 0..roots.len() {
                register(
                    self,
                    roots.get(i).unwrap().unwrap(),
                    format!("{}/{i}", node_id(*key).unwrap_or(0)),
                    &mut names,
                    &mut order,
                );
            }
        }
        let tree_name = |tree: Option<OwnerTreeId>| tree.map(|tree| names[&tree].clone());
        let tree_header = |header: &OwnerTreeSlice| {
            json!({"nil":header.is_nil(),"len":header.len(),"cap":header.capacity(),
            "slots":(0..header.capacity()).map(|i|tree_name(header.read_capacity_slot(i).unwrap())).collect::<Vec<_>>()})
        };
        let roots = root_entries
            .iter()
            .map(|(node, header)| json!({"key":node_id(*node),"roots":tree_header(header)}))
            .collect::<Vec<_>>();
        let trees=order.iter().map(|tree|{let fields=self.owner_tree(*tree).unwrap().fields();json!({"name":names[tree],"node":node_id(fields.node),
            "parent":tree_name(fields.parent),"children":tree_header(&fields.children),"edge":edge(fields.sentinel_edge),"orientation":fields.orientation})}).collect::<Vec<_>>();
        let mut node_tree_entries = Vec::new();
        view.node_to_tree
            .validate_entries::<std::convert::Infallible>(|node, tree| {
                node_tree_entries.push((node, tree));
                Ok(())
            })
            .unwrap();
        node_tree_entries.sort_by_key(|(node, _)| node_id(*node).unwrap_or(0));
        let node_to_tree = node_tree_entries
            .into_iter()
            .map(|(node, tree)| json!({"node":node_id(node),"tree":tree_name(tree)}))
            .collect::<Vec<_>>();
        let mut hierarchy_names = BTreeMap::new();
        let mut hierarchies = Vec::new();
        let mut nodes = Vec::new();
        let mut node_order = self
            .nodes
            .iter()
            .map(|node| node.input_id)
            .collect::<Vec<_>>();
        node_order.sort_by_key(|node| node_id(Some(*node)));
        for node in node_order {
            let hierarchy=self.owner_node_hierarchy(node).unwrap().map(|h|{
                let next=hierarchy_names.len();let index=*hierarchy_names.entry(h.allocation_key()).or_insert(next);
                if index==next {let fields=self.owner_hierarchy_fields(&h).unwrap();let mut levels=fields.levels.values.as_ref()
                    .map(|values|values.read().unwrap().iter().map(|(&node,&level)|(node_id(node),level)).collect::<Vec<_>>()).unwrap_or_default();
                    levels.sort_by_key(|(node,_)|node.unwrap_or(0));
                    hierarchies.push(json!({"index":index,"level_count":fields.level_count,"levels":levels,"levels_nil":fields.levels.is_nil()}));}
                index
            });
            let group = |kind| {
                self.owner_node_group_id(node, kind)
                    .unwrap()
                    .and_then(|group| self.owner_group_fields(group).unwrap().vessel)
            };
            nodes.push(json!({"id":node_id(Some(node)),"cluster":node_id(group(OwnerGroupKind::Cluster)),"sequence":node_id(group(OwnerGroupKind::Sequence)),"hierarchy":hierarchy}));
        }
        json!({"name":name,"graph_nodes":node_header(&self.owner_graph_member_header(OwnerGraphId(0)).unwrap()),"groups":groups,"clusters_nil":view.clusters.is_nil(),"sequences_nil":view.sequences.is_nil(),
            "trees_nil":view.tree_roots.is_nil(),"node_to_tree_nil":view.node_to_tree.is_nil(),"roots":roots,"trees":trees,"node_to_tree":node_to_tree,"nodes":nodes,"hierarchies":hierarchies})
    }
}
