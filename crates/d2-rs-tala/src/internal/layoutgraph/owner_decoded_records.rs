// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Included in shared_topology: the decoder publishes actual trusted headers,
// not numerical copies reconstructed later from carrier/member equality.
impl ArenaGraph {
    pub(super) fn publish_decoded_owner_records(&mut self, input: &Graph) {
        let nodes = self
            .nodes
            .iter()
            .map(|node| (node.tala_id, node.input_id))
            .collect::<BTreeMap<_, _>>();
        for (index, cluster) in input.serialized_clusters.iter().enumerate() {
            let group = self.publish_decoded_group(
                OwnerGroupKind::Cluster,
                TopologyVessel::Cluster(index),
                cluster.vessel_tala_id,
                cluster.container_tala_id,
                &cluster.member_tala_ids,
                &cluster.edge_abductions,
                &nodes,
                input,
            );
            let edge_abductions = self.owner_cluster_policy(group).unwrap().edge_abductions;
            self.set_owner_cluster_policy(
                group,
                super::owner_groups::OwnerClusterPolicy {
                    arrangement: cluster.arrangement.clone(),
                    desired_arrangement: cluster.desired_arrangement.clone(),
                    padding: cluster.padding,
                    fixed_size: cluster.fixed_size,
                    edge_abductions,
                },
            )
            .expect("decoded cluster policy");
        }
        for (index, sequence) in input.serialized_sequences.iter().enumerate() {
            self.publish_decoded_group(
                OwnerGroupKind::Sequence,
                TopologyVessel::Sequence(index),
                sequence.vessel_tala_id,
                sequence.container_tala_id,
                &sequence.member_tala_ids,
                &sequence.edge_abductions,
                &nodes,
                input,
            );
        }

        let roots_map = SharedOwnerTreeRootsMap::allocated();
        let node_to_tree = SharedOwnerTreeMap::allocated();
        // BuildNodeToTreeForDeserialization explicitly sorts sentinel IDs,
        // then applies a breadth-first walk separately for each root.
        for (&sentinel, roots) in &input.serialized_trees {
            let mut header = OwnerTreeSlice::nil();
            for root in roots {
                let tree = self.publish_decoded_tree(root, None, &nodes, input);
                header = header.append_tree(tree);
                let mut queue = vec![tree];
                let mut index = 0;
                while index < queue.len() {
                    let fields = self.owner_tree(queue[index]).unwrap().fields();
                    node_to_tree
                        .insert(fields.node, Some(queue[index]))
                        .unwrap();
                    queue.extend(
                        (0..fields.children.len())
                            .map(|i| fields.children.get(i).unwrap().unwrap()),
                    );
                    index += 1;
                }
            }
            if !header.is_empty() {
                roots_map
                    .insert((sentinel != 0).then(|| nodes[&sentinel]), header)
                    .unwrap();
            }
        }
        let view = self.shared_topology.views[0].as_mut().unwrap();
        view.tree_roots = roots_map;
        view.node_to_tree = node_to_tree;

        for decoded in &input.serialized_hierarchies {
            let hierarchy = self.new_owner_hierarchy();
            let levels = self.owner_hierarchy_levels(&hierarchy).unwrap();
            for (&id, &level) in &decoded.levels {
                let node = nodes[&id];
                self.write_owner_hierarchy_level(&levels, Some(node), level)
                    .unwrap();
                self.set_owner_node_hierarchy(node, Some(hierarchy.clone()))
                    .unwrap();
            }
        }
    }

    fn publish_decoded_group(
        &mut self,
        kind: OwnerGroupKind,
        reference: TopologyVessel,
        vessel: i64,
        container: i64,
        members: &[i64],
        abductions: &[d2_rs_core::SerializedAggregateEdgeAbduction],
        nodes: &BTreeMap<i64, NodeId>,
        input: &Graph,
    ) -> OwnerGroupId {
        let group = self.new_owner_group(kind);
        let members = members.iter().map(|id| nodes[id]).collect::<Vec<_>>();
        // codec.go:1179/1252 use growslice (confirmed by pinned assembly),
        // unlike the buffered append used for decoded abduction lists.
        let mut slice = SharedNodeSlice::allocated_empty();
        for &member in &members {
            slice = slice.append_node(member).unwrap();
        }
        let header = OwnerMemberHeader {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            slice,
        };
        self.set_owner_group_fields(
            group,
            OwnerGroupFieldView {
                vessel: Some(nodes[&vessel]),
                nodes: header,
                graph: Some(OwnerGraphId(0)),
                container: (container != 0).then(|| nodes[&container]),
            },
        )
        .unwrap();
        for member in members {
            self.set_owner_node_group(member, kind, Some(group))
                .unwrap();
        }
        let resolve = |id| (id != 0).then(|| nodes[&id]);
        let records = abductions
            .iter()
            .map(|record| OwnerEdgeAbduction {
                source_edge: Some(input.tala_decoded_edge_records[&record.edge_id]),
                originally_from: resolve(record.originally_from_node),
                originally_to: resolve(record.originally_to_node),
                current_from: resolve(record.current_from_node),
                current_to: resolve(record.current_to_node),
            })
            .collect();
        let records = self.register_owner_edge_abductions(records, true).unwrap();
        let header = OwnerAbductionHeader {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            slice: SharedAbductionSlice::from_go127_buffered_appends(&records.records).unwrap(),
        };
        match kind {
            OwnerGroupKind::Cluster => self
                .set_owner_cluster_policy(
                    group,
                    super::owner_groups::OwnerClusterPolicy {
                        edge_abductions: Some(header),
                        ..Default::default()
                    },
                )
                .unwrap(),
            OwnerGroupKind::Sequence => self
                .set_owner_sequence_abductions(group, Some(header))
                .unwrap(),
        }
        self.bind_owner_group_reference(reference, group).unwrap();
        let map = self.owner_graph_group_map(OwnerGraphId(0), kind).unwrap();
        self.write_owner_group_map_entry(&map, Some(nodes[&vessel]), Some(group))
            .unwrap();
        group
    }

    fn publish_decoded_tree(
        &mut self,
        decoded: &d2_rs_core::SerializedTreeState,
        parent: Option<OwnerTreeId>,
        nodes: &BTreeMap<i64, NodeId>,
        input: &Graph,
    ) -> OwnerTreeId {
        let tree = self
            .new_owner_tree(Some(nodes[&decoded.node_tala_id]))
            .unwrap();
        let mut children = OwnerTreeSlice::nil();
        for child in &decoded.children {
            let child = self.publish_decoded_tree(child, Some(tree), nodes, input);
            children = children.append_tree(child);
        }
        let edge = input.tala_decoded_edge_records[&decoded
            .sentinel_edge_serialized_id
            .expect("decoded tree retains serialized edge identity")];
        self.set_owner_tree_fields(
            tree,
            super::owner_tree_map::OwnerTreeFields {
                node: Some(nodes[&decoded.node_tala_id]),
                parent,
                children,
                sentinel_edge: Some(edge),
                orientation: decoded.orientation,
            },
        )
        .unwrap();
        tree
    }

    /// Numerical discovery indices are reusable; source record allocations
    /// and retained node pointers are not. Clear only the projection bindings.
    pub(super) fn begin_owner_group_discovery(&mut self, kind: OwnerGroupKind) {
        let matches = |reference: &TopologyVessel| {
            matches!(
                (kind, reference),
                (OwnerGroupKind::Cluster, TopologyVessel::Cluster(_))
                    | (OwnerGroupKind::Sequence, TopologyVessel::Sequence(_))
            )
        };
        self.shared_topology
            .group_bindings
            .retain(|reference, _| !matches(reference));
        self.shared_topology
            .vessels
            .retain(|reference, _| !matches(reference));
        let view = self.shared_topology.views[0].as_mut().unwrap();
        match kind {
            OwnerGroupKind::Cluster => {
                if view.clusters.is_nil() {
                    view.clusters.replace(SharedOwnerGroupMap::allocated());
                } else {
                    for (key, _) in view.clusters.entries() {
                        view.clusters.delete(key);
                    }
                }
            }
            OwnerGroupKind::Sequence => view.sequences.replace(SharedOwnerGroupMap::allocated()),
        }
    }

    /// isValidRememberedSequence and its pointer-conditional cleanup. Return
    /// validity only for actual bound records; unmaterialized numerical groups
    /// retain their separate, still-unqualified discovery adapter.
    pub(super) fn owner_remembered_sequence_validity(&mut self) -> BTreeMap<usize, bool> {
        let mut groups = self.shared_topology.views[0]
            .as_ref()
            .unwrap()
            .sequences
            .entries();
        groups.sort_by_key(|(node, _)| node.map_or(0, |node| self.nodes[node.0 as usize].tala_id));
        let active = self
            .owner_graph_member_header(OwnerGraphId(0))
            .unwrap()
            .iter_slots()
            .flatten()
            .collect::<BTreeSet<_>>();
        let mut result = BTreeMap::new();
        for (vessel, group) in groups {
            let Some(group) = group else {
                continue;
            };
            let fields = self.owner_group_fields(group).unwrap();
            let valid = (|| {
                let Some(vessel) = vessel else {
                    return false;
                };
                if self.topology_node_owner(vessel).is_some()
                    || fields.vessel != Some(vessel)
                    || fields.graph != Some(OwnerGraphId(0))
                    || fields.nodes.len() < 2
                {
                    return false;
                }
                if fields.container.is_some_and(|node| !active.contains(&node)) {
                    return false;
                }
                let Some(children) = self.containers.get_header(&fields.container) else {
                    return false;
                };
                let mut positions = BTreeMap::new();
                let mut duplicate_children = BTreeSet::new();
                for (index, child) in children.iter_slots().enumerate() {
                    if let std::collections::btree_map::Entry::Vacant(e) = positions.entry(child) {
                        e.insert(index);
                    } else {
                        duplicate_children.insert(child);
                    }
                }
                let mut seen = BTreeSet::new();
                let mut previous = None;
                for slot in fields.nodes.iter_slots() {
                    let Some(node) = slot else {
                        return false;
                    };
                    let record = &self.nodes[node.0 as usize];
                    if !seen.insert(node)
                        || !active.contains(&node)
                        || self.topology_node_owner(node) != Some(OwnerGraphId(0))
                        || record.shape != ShapeKind::Step
                        || record.fixed_top_left.is_some()
                        || self
                            .owner_node_group_id(node, OwnerGroupKind::Sequence)
                            .unwrap()
                            != Some(group)
                        || record.container != fields.container
                        || duplicate_children.contains(&Some(node))
                    {
                        return false;
                    }
                    let Some(&position) = positions.get(&Some(node)) else {
                        return false;
                    };
                    if previous.is_some_and(|previous| position != previous + 1) {
                        return false;
                    }
                    previous = Some(position);
                }
                true
            })();
            for (&reference, &bound) in self.shared_topology.group_bindings.iter() {
                if bound == group
                    && let TopologyVessel::Sequence(index) = reference
                {
                    result.insert(index, valid);
                }
            }
            if !valid {
                for node in fields.nodes.iter_slots().flatten() {
                    if self
                        .owner_node_group_id(node, OwnerGroupKind::Sequence)
                        .unwrap()
                        == Some(group)
                    {
                        self.set_owner_node_group(node, OwnerGroupKind::Sequence, None)
                            .unwrap();
                    }
                }
            }
        }
        result
    }
}

impl ArenaGraph {
    /// AddSequences reserves existing graph/group IDs before replacing the
    /// sequence map. hasNodeID also searches tree sentinels and every tree node.
    /// This set remains occupied while those records are removed/rebuilt.
    pub(super) fn sequence_occupied_node_ids(&self) -> BTreeSet<i64> {
        let view = self.shared_topology.views[0].as_ref().unwrap();
        let mut nodes = view.nodes.iter_slots().flatten().collect::<BTreeSet<_>>();
        for map in [&view.clusters, &view.sequences] {
            for (vessel, group) in map.entries() {
                nodes.extend(vessel);
                if let Some(group) = group {
                    nodes.extend(self.owner_group_fields(group).unwrap().nodes.iter_slots().flatten());
                }
            }
        }
        let mut pending = Vec::new();
        for (sentinel, roots) in view.tree_roots.entries() {
            nodes.extend(sentinel);
            pending.extend((0..roots.len()).filter_map(|i| roots.get(i).flatten()));
        }
        let mut seen = BTreeSet::new();
        while let Some(tree) = pending.pop() {
            if !seen.insert(tree) { continue; }
            let fields = self.owner_tree(tree).unwrap().fields();
            nodes.extend(fields.node);
            pending.extend((0..fields.children.len()).filter_map(|i| fields.children.get(i).flatten()));
        }
        let mut ids = nodes.into_iter().map(|n| self.nodes[n.0 as usize].tala_id).collect::<BTreeSet<_>>();
        // Ordinary stages still have numerical groups before canonical vessel
        // publication. Their complete key/member inventory is the same occupied
        // set; do not reserve unrelated retained arena records by accident.
        for (index, group) in self.clusters.iter().enumerate() {
            if self.owner_group_binding(TopologyVessel::Cluster(index)).is_none() {
                ids.insert(group.vessel_tala_id);
                ids.extend(group.members.iter().map(|n|self.nodes[n.0 as usize].tala_id));
            }
        }
        for (index, group) in self.sequences.iter().enumerate() {
            if self.owner_group_binding(TopologyVessel::Sequence(index)).is_none() {
                ids.insert(group.vessel_tala_id);
                ids.extend(group.members.iter().map(|n|self.nodes[n.0 as usize].tala_id));
            }
        }
        ids
    }
}
