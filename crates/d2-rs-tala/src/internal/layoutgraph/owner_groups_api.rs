// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Included in shared_topology so trusted member headers keep private storage.
#[derive(Clone, Debug)]
pub(super) struct OwnerGroupMap {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    kind: OwnerGroupKind,
    map: SharedOwnerGroupMap,
}
impl OwnerGroupMap {
    pub(super) fn is_nil(&self) -> bool {
        self.map.is_nil()
    }
    pub(super) fn len(&self) -> usize {
        self.map.len()
    }
    pub(super) fn same_map_allocation(&self, other: &Self) -> bool {
        self.kind == other.kind && self.map.same_allocation(&other.map)
    }
}

#[derive(Clone, Debug)]
pub(super) struct OwnerGroupMapField {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    kind: OwnerGroupKind,
    field: SharedOwnerGroupMapField,
}
impl OwnerGroupMapField {
    pub(super) fn header(&self) -> OwnerGroupMap {
        OwnerGroupMap { arena:self.arena, scope:self.scope, kind:self.kind, map:self.field.header() }
    }
    pub(super) fn replace(&self, map: OwnerGroupMap) -> Result<(), OwnerTopologyError> {
        if self.arena != map.arena {
            return Err(OwnerTopologyError::ForeignArena { expected:Some(self.arena), actual:Some(map.arena) });
        }
        if !MemberScopeId::same_scope(&self.scope, &map.scope) { return Err(OwnerTopologyError::ForeignGroupScope); }
        if self.kind != map.kind { return Err(OwnerTopologyError::GroupKindMismatch); }
        self.field.replace(map.map);
        Ok(())
    }
    pub(super) fn same_field(&self, other: &Self) -> bool { self.kind == other.kind && self.field.same_field(&other.field) }
}

#[derive(Clone, Debug)]
pub(super) struct OwnerGroupFieldView {
    pub(super) vessel: Option<NodeId>,
    pub(super) nodes: OwnerMemberHeader,
    pub(super) graph: Option<OwnerGraphId>,
    pub(super) container: Option<NodeId>,
}

impl ArenaGraph {
    pub(super) fn retain_owner_graph_group_map_field(&self, receiver: OwnerGraphId, kind: OwnerGroupKind) -> Result<OwnerGroupMapField, OwnerTopologyError> {
        let view = self.owner_graph_view(receiver).ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        Ok(OwnerGroupMapField { arena:self.topology_arena_id().unwrap(), scope:self.shared_topology.member_scope, kind,
            field:match kind { OwnerGroupKind::Cluster=>view.clusters.clone(), OwnerGroupKind::Sequence=>view.sequences.clone() } })
    }
    pub(super) fn set_owner_node_group(
        &mut self,
        node: NodeId,
        kind: OwnerGroupKind,
        group: Option<OwnerGroupId>,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        self.require_topology_node(node)?;
        if let Some(group) = group {
            self.require_owner_group(group)?;
            if group.kind() != kind {
                return Err(OwnerTopologyError::GroupKindMismatch);
            }
        }
        self.shared_topology.node_groups.insert((node, kind), group);
        Ok(())
    }
    /// Outer None means no group pointer; Some(None) means an actual group
    /// with a nil Vessel. Legacy indices are consulted only without a raw field.
    pub(super) fn owner_node_group_vessel(
        &self,
        node: NodeId,
        kind: OwnerGroupKind,
    ) -> Result<Option<Option<NodeId>>, OwnerTopologyError> {
        let live = self
            .topology_node(node)
            .ok_or(OwnerTopologyError::UnknownNode(node))?;
        if let Some(group) = self.shared_topology.node_groups.get(&(node, kind)) {
            return group
                .map(|group| {
                    self.require_owner_group(group)?;
                    Ok(self.owner_group(group).expect("validated group").vessel())
                })
                .transpose();
        }
        let reference = match kind {
            OwnerGroupKind::Cluster => live.cluster.map(TopologyVessel::Cluster),
            OwnerGroupKind::Sequence => live.sequence.map(TopologyVessel::Sequence),
        };
        reference
            .map(|reference| {
                self.topology_vessel_reference(reference)
                    .ok_or(OwnerTopologyError::UnboundVessel(reference))
            })
            .transpose()
    }
    pub(super) fn set_owner_cluster_vessel_flag(
        &mut self,
        node: NodeId,
        value: bool,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        self.require_topology_node(node)?;
        if value {
            self.shared_topology.cluster_vessel_flags.insert(node);
        } else {
            self.shared_topology.cluster_vessel_flags.remove(&node);
        }
        Ok(())
    }
    pub(super) fn owner_is_cluster_vessel(&self, node: NodeId) -> bool {
        self.shared_topology.cluster_vessel_flags.contains(&node)
    }
    pub(super) fn new_owner_group(&mut self, kind: OwnerGroupKind) -> OwnerGroupId {
        self.initialize_shared_topology();
        let id = OwnerGroupId::new(
            self.topology_arena_id().expect("initialized owner"),
            self.shared_topology.groups.len(),
            kind,
        );
        self.shared_topology
            .groups
            .push((id, Arc::new(OwnerGroupRecord::default())));
        id
    }
    /// Retain the actual allocation independently of graph maps and borrows.
    /// Polling callbacks may update its live fields without replacing the owner.
    pub(super) fn retain_owner_group(
        &self, id: OwnerGroupId,
    ) -> Result<Arc<OwnerGroupRecord>, OwnerTopologyError> {
        self.require_owner_group(id)?;
        Ok(Arc::clone(&self.shared_topology.groups[id.index()].1))
    }
    pub(super) fn owner_group(&self, id: OwnerGroupId) -> Option<&OwnerGroupRecord> {
        (Some(id.arena()) == self.topology_arena_id())
            .then(|| self.shared_topology.groups.get(id.index()))
            .flatten()
            .filter(|(actual, _)| *actual == id)
            .map(|(_, record)| record.as_ref())
    }
    fn require_owner_group(&self, id: OwnerGroupId) -> Result<(), OwnerTopologyError> {
        if Some(id.arena()) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(id.arena()),
            });
        }
        self.owner_group(id)
            .ok_or(OwnerTopologyError::UnknownGroup(id))?;
        Ok(())
    }
    pub(super) fn owner_group_fields(
        &self,
        id: OwnerGroupId,
    ) -> Result<OwnerGroupFieldView, OwnerTopologyError> {
        self.require_owner_group(id)?;
        let fields = self.owner_group(id).expect("validated group").fields();
        Ok(OwnerGroupFieldView {
            vessel: fields.vessel,
            graph: fields.graph,
            container: fields.container,
            nodes: OwnerMemberHeader {
                arena: self.topology_arena_id().unwrap(),
                scope: self.shared_topology.member_scope,
                slice: fields.nodes,
            },
        })
    }
    pub(super) fn set_owner_group_fields(
        &mut self,
        id: OwnerGroupId,
        fields: OwnerGroupFieldView,
    ) -> Result<(), OwnerTopologyError> {
        self.require_owner_group(id)?;
        self.validate_owner_member_header(&fields.nodes)?;
        for node in [fields.vessel, fields.container].into_iter().flatten() {
            self.require_topology_node(node)?;
        }
        for index in 0..fields.nodes.capacity() {
            if let Some(Some(node)) = fields.nodes.read_capacity_slot(index) {
                self.require_topology_node(node)?;
            }
        }
        if let Some(graph) = fields.graph {
            self.owner_graph_view(graph)
                .ok_or(OwnerTopologyError::UnknownGraph(graph))?;
        }
        self.owner_group(id).unwrap().set_fields(OwnerGroupFields {
            vessel: fields.vessel,
            nodes: fields.nodes.slice,
            graph: fields.graph,
            container: fields.container,
        });
        Ok(())
    }
    /// grouping/clusters.go:108–122: AddNewNodeToContainer appends the real
    /// vessel, assigns each member's raw Cluster pointer, then replaces the
    /// shared parent child header by an independently grown filtered slice.
    /// This publishes the container-map part of AddCluster; graph membership
    /// and raw member Container removal retain their separate lifetimes.
    pub(super) fn publish_owner_cluster_children(
        &mut self,
        group: OwnerGroupId,
    ) -> Result<(), OwnerTopologyError> {
        self.require_owner_group(group)?;
        if group.kind() != OwnerGroupKind::Cluster {
            return Err(OwnerTopologyError::GroupKindMismatch);
        }
        let fields = self.owner_group_fields(group)?;
        let vessel = fields
            .vessel
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let members = (0..fields.nodes.len())
            .map(|index| {
                fields
                    .nodes
                    .get(index)
                    .expect("logical cluster member")
                    .expect("runtime error: invalid memory address or nil pointer dereference")
            })
            .collect::<Vec<_>>();
        self.append_owner_container_node(fields.container, vessel)?;
        for member in members {
            self.set_owner_node_group(member, OwnerGroupKind::Cluster, Some(group))?;
        }
        let children = self
            .containers
            .get_header(&fields.container)
            .expect("AddNewNodeToContainer published its header");
        let mut updated = SharedNodeSlice::nil();
        for child in children.iter_slots() {
            let child =
                child.expect("runtime error: invalid memory address or nil pointer dereference");
            let actual_group = self
                .shared_topology
                .node_groups
                .get(&(child, OwnerGroupKind::Cluster))
                .copied()
                .unwrap_or_else(|| {
                    self.nodes[child.0 as usize].cluster.and_then(|index| {
                        self.shared_topology
                            .group_bindings
                            .get(&TopologyVessel::Cluster(index))
                            .copied()
                    })
                });
            if actual_group != Some(group) {
                updated = updated
                    .append_node(child)
                    .expect("Source cluster filtered append");
            }
        }
        self.containers.replace_header(fields.container, updated);
        Ok(())
    }

    pub(super) fn bind_owner_group_reference(
        &mut self,
        reference: TopologyVessel,
        id: OwnerGroupId,
    ) -> Result<(), OwnerTopologyError> {
        self.require_owner_group(id)?;
        let (kind, valid) = match reference {
            TopologyVessel::Cluster(index) => {
                (OwnerGroupKind::Cluster, index < self.clusters.len())
            }
            TopologyVessel::Sequence(index) => {
                (OwnerGroupKind::Sequence, index < self.sequences.len())
            }
        };
        if !valid {
            return Err(OwnerTopologyError::InvalidVessel(reference));
        }
        if kind != id.kind() {
            return Err(OwnerTopologyError::GroupKindMismatch);
        }
        if self
            .shared_topology
            .group_bindings
            .get(&reference)
            .is_some_and(|&bound| bound != id)
        {
            return Err(OwnerTopologyError::GroupBindingConflict(reference));
        }
        if self
            .shared_topology
            .vessels
            .get(&reference)
            .is_some_and(|&vessel| vessel != self.owner_group(id).unwrap().vessel())
        {
            return Err(OwnerTopologyError::GroupBindingConflict(reference));
        }
        self.shared_topology.vessels.remove(&reference);
        self.shared_topology.group_bindings.insert(reference, id);
        Ok(())
    }
    pub(super) fn owner_graph_group_map(
        &self,
        receiver: OwnerGraphId,
        kind: OwnerGroupKind,
    ) -> Result<OwnerGroupMap, OwnerTopologyError> {
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        Ok(OwnerGroupMap {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            kind,
            map: match kind {
                OwnerGroupKind::Cluster => view.clusters.header(),
                OwnerGroupKind::Sequence => view.sequences.header(),
            },
        })
    }
    pub(super) fn allocate_owner_group_map(
        &mut self,
        kind: OwnerGroupKind,
        present: bool,
    ) -> OwnerGroupMap {
        self.initialize_shared_topology();
        OwnerGroupMap {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            kind,
            map: if present {
                SharedOwnerGroupMap::allocated()
            } else {
                SharedOwnerGroupMap::nil()
            },
        }
    }
    fn validate_owner_group_map(&self, map: &OwnerGroupMap) -> Result<(), OwnerTopologyError> {
        if Some(map.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(map.arena),
            });
        }
        if !MemberScopeId::same_scope(&map.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignGroupScope);
        }
        Ok(())
    }
    pub(super) fn owner_group_map_entry(
        &self,
        map: &OwnerGroupMap,
        node: Option<NodeId>,
    ) -> Result<Option<Option<OwnerGroupId>>, OwnerTopologyError> {
        self.validate_owner_group_map(map)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        let entry = map.map.get(node);
        if let Some(Some(group)) = entry {
            self.require_owner_group(group)?;
            if group.kind() != map.kind {
                return Err(OwnerTopologyError::GroupKindMismatch);
            }
        }
        Ok(entry)
    }
    /// Inventory is used only where source explicitly collects and sorts keys.
    pub(super) fn owner_group_map_keys(
        &self,
        map: &OwnerGroupMap,
    ) -> Result<Vec<Option<NodeId>>, OwnerTopologyError> {
        self.validate_owner_group_map(map)?;
        Ok(map.map.entries().into_iter().map(|(key, _)| key).collect())
    }
    pub(super) fn owner_group_map_values(
        &self,
        map: &OwnerGroupMap,
    ) -> Result<super::owner_groups::OwnerGroupMapCursor, OwnerTopologyError> {
        self.validate_owner_group_map(map)?;
        Ok(map.map.live_values())
    }
    /// Read the actual raw group pointer; an explicit nil overrides legacy metadata.
    pub(super) fn owner_node_group_id(
        &self,
        node: NodeId,
        kind: OwnerGroupKind,
    ) -> Result<Option<OwnerGroupId>, OwnerTopologyError> {
        self.require_topology_node(node)?;
        if let Some(group) = self.shared_topology.node_groups.get(&(node, kind)) {
            return Ok(*group);
        }
        let live = self.topology_node(node).expect("validated node");
        let reference = match kind {
            OwnerGroupKind::Cluster => live.cluster.map(TopologyVessel::Cluster),
            OwnerGroupKind::Sequence => live.sequence.map(TopologyVessel::Sequence),
        };
        reference
            .map(|reference| {
                self.shared_topology
                    .group_bindings
                    .get(&reference)
                    .copied()
                    .ok_or(OwnerTopologyError::UnboundVessel(reference))
            })
            .transpose()
    }

    /// Canonical group bound to an algorithm carrier, independently of which
    /// graph map currently retains the group pointer.
    pub(super) fn owner_group_binding(&self, reference: TopologyVessel) -> Option<OwnerGroupId> {
        self.shared_topology.group_bindings.get(&reference).copied()
    }

    pub(super) fn read_owner_group_fields(
        &self, id: OwnerGroupId,
    ) -> Result<crate::internal::go_cell::GoCellRef<'_, super::owner_groups::OwnerGroupFields>, OwnerTopologyError> {
        self.require_owner_group(id)?;
        Ok(self.owner_group(id).expect("validated group").read_fields())
    }
    pub(super) fn read_owner_cluster_policy(
        &self, id: OwnerGroupId,
    ) -> Result<crate::internal::go_cell::GoCellRef<'_, super::owner_groups::OwnerClusterPolicy>, OwnerTopologyError> {
        self.require_owner_group(id)?;
        if id.kind() != OwnerGroupKind::Cluster {
            return Err(OwnerTopologyError::GroupKindMismatch);
        }
        Ok(self.owner_group(id).unwrap().read_policy())
    }
    pub(super) fn owner_cluster_policy(
        &self,
        id: OwnerGroupId,
    ) -> Result<super::owner_groups::OwnerClusterPolicy, OwnerTopologyError> {
        self.require_owner_group(id)?;
        if id.kind() != OwnerGroupKind::Cluster {
            return Err(OwnerTopologyError::GroupKindMismatch);
        }
        Ok(self.owner_group(id).unwrap().policy())
    }
    pub(super) fn owner_sequence_abductions(
        &self,
        id: OwnerGroupId,
    ) -> Result<Option<OwnerAbductionHeader>, OwnerTopologyError> {
        self.require_owner_group(id)?;
        if id.kind() != OwnerGroupKind::Sequence {
            return Err(OwnerTopologyError::GroupKindMismatch);
        }
        Ok(self.owner_group(id).unwrap().sequence_abductions())
    }
    pub(super) fn set_owner_sequence_abductions(
        &mut self,
        id: OwnerGroupId,
        header: Option<OwnerAbductionHeader>,
    ) -> Result<(), OwnerTopologyError> {
        self.require_owner_group(id)?;
        if id.kind() != OwnerGroupKind::Sequence {
            return Err(OwnerTopologyError::GroupKindMismatch);
        }
        if let Some(header) = &header {
            self.validate_owner_abduction_header(header)?;
        }
        self.owner_group(id)
            .unwrap()
            .set_sequence_abductions(header);
        Ok(())
    }
    pub(super) fn set_owner_cluster_policy(
        &mut self,
        id: OwnerGroupId,
        policy: super::owner_groups::OwnerClusterPolicy,
    ) -> Result<(), OwnerTopologyError> {
        self.require_owner_group(id)?;
        if id.kind() != OwnerGroupKind::Cluster {
            return Err(OwnerTopologyError::GroupKindMismatch);
        }
        if let Some(header) = &policy.edge_abductions {
            self.validate_owner_abduction_header(header)?;
        }
        self.owner_group(id).unwrap().set_policy(policy);
        Ok(())
    }

    pub(super) fn set_owner_cluster_desired_arrangement(
        &mut self, id: OwnerGroupId, desired: String,
    ) -> Result<(), OwnerTopologyError> {
        self.require_owner_group(id)?;
        if id.kind() != OwnerGroupKind::Cluster {
            return Err(OwnerTopologyError::GroupKindMismatch);
        }
        self.owner_group(id).unwrap().set_desired_arrangement(desired);
        Ok(())
    }

    /// container_orientation.go changes only these three scalar fields.
    /// FixedSize and the retained EdgeAbductions header are not reassigned.
    pub(super) fn set_owner_cluster_orientation_policy(
        &mut self,
        id: OwnerGroupId,
        arrangement: String,
        desired: String,
        padding: f64,
    ) -> Result<(), OwnerTopologyError> {
        self.require_owner_group(id)?;
        if id.kind() != OwnerGroupKind::Cluster { return Err(OwnerTopologyError::GroupKindMismatch); }
        self.owner_group(id).unwrap().set_orientation_policy(arrangement, desired, padding);
        Ok(())
    }
    pub(super) fn owner_graph_group_entry(
        &self,
        receiver: OwnerGraphId,
        kind: OwnerGroupKind,
        node: Option<NodeId>,
    ) -> Result<Option<Option<OwnerGroupId>>, OwnerTopologyError> {
        // This callback-free lookup returns a copied identity. Retaining a
        // map/scope header is only necessary when the caller keeps that handle
        // across field replacement, as owner_graph_group_map permits.
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        let entry = match kind {
            OwnerGroupKind::Cluster => view.clusters.get(node),
            OwnerGroupKind::Sequence => view.sequences.get(node),
        };
        if let Some(Some(group)) = entry {
            self.require_owner_group(group)?;
            if group.kind() != kind {
                return Err(OwnerTopologyError::GroupKindMismatch);
            }
        }
        Ok(entry)
    }
    pub(super) fn write_owner_group_map_entry(
        &mut self,
        map: &OwnerGroupMap,
        node: Option<NodeId>,
        group: Option<OwnerGroupId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_group_map(map)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        if let Some(group) = group {
            self.require_owner_group(group)?;
            if group.kind() != map.kind {
                return Err(OwnerTopologyError::GroupKindMismatch);
            }
        }
        map.map
            .insert(node, group)
            .map_err(|()| OwnerTopologyError::NilGroupMapAssignment)
    }
    pub(super) fn delete_owner_group_map_entry(
        &mut self,
        map: &OwnerGroupMap,
        node: Option<NodeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_group_map(map)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        map.map.delete(node);
        Ok(())
    }
    pub(super) fn set_owner_graph_group_map(
        &mut self,
        receiver: OwnerGraphId,
        map: OwnerGroupMap,
    ) -> Result<(), OwnerTopologyError> {
        self.owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        self.validate_owner_group_map(&map)?;
        for (node, _) in map.map.entries() {
            self.owner_group_map_entry(&map, node)?;
        }
        let view = self.shared_topology.views[receiver.0].as_mut().unwrap();
        match map.kind {
            OwnerGroupKind::Cluster => view.clusters.replace(map.map),
            OwnerGroupKind::Sequence => view.sequences.replace(map.map),
        }
        Ok(())
    }
    pub(super) fn copy_owner_graph_group_map_from(
        &mut self,
        receiver: OwnerGraphId,
        source: OwnerGraphId,
        kind: OwnerGroupKind,
    ) -> Result<(), OwnerTopologyError> {
        let map = self.owner_graph_group_map(source, kind)?;
        self.set_owner_graph_group_map(receiver, map)
    }
}

#[cfg(test)]
mod cluster_child_publication_tests {
    use super::*;

    #[test]
    fn source_cluster_append_then_filter_replaces_shared_header_by_group_pointer() {
        let mut input = Graph::default();
        let [parent, a, b, peer, vessel] = ["parent", "a", "b", "peer", "vessel"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 20.0,
                    height: 30.0,
                },
            ))
        });
        for member in [a, b, peer] {
            input.node_mut(member).unwrap().parent = Some(parent);
        }
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        let view = owner.create_owner_graph_view(vec![parent], vec![]).unwrap();
        let header = owner
            .allocate_owner_container_header(&[a, b, peer], 4)
            .unwrap();
        owner
            .set_owner_container_header(Some(parent), header.clone())
            .unwrap();
        let shared_map = owner.owner_container_map(view).unwrap();
        let group = owner.new_owner_group(OwnerGroupKind::Cluster);
        let members = owner
            .allocate_owner_member_header(&[Some(a), Some(b)], 2)
            .unwrap();
        owner
            .set_owner_group_fields(
                group,
                OwnerGroupFieldView {
                    vessel: Some(vessel),
                    nodes: members,
                    graph: None,
                    container: Some(parent),
                },
            )
            .unwrap();
        // Two different Cluster pointers may share a Vessel. Source compares
        // child.Cluster with the actual cluster pointer, not vessel equality.
        let other = owner.new_owner_group(OwnerGroupKind::Cluster);
        let empty = owner.allocate_owner_member_header(&[], 0).unwrap();
        owner
            .set_owner_group_fields(
                other,
                OwnerGroupFieldView {
                    vessel: Some(vessel),
                    nodes: empty,
                    graph: None,
                    container: Some(parent),
                },
            )
            .unwrap();
        owner
            .set_owner_node_group(peer, OwnerGroupKind::Cluster, Some(other))
            .unwrap();
        owner.publish_owner_cluster_children(group).unwrap();
        let published = shared_map.get(Some(parent)).unwrap();
        assert_eq!((published.len(), published.capacity()), (2, 2));
        assert_eq!(
            [published.get(0), published.get(1)],
            [Some(Some(peer)), Some(Some(vessel))]
        );
        assert_eq!(header.len(), 3, "previous header length is retained");
        assert_eq!(
            header.read_capacity_slot(3),
            Some(Some(vessel)),
            "Source append writes shared spare capacity before header replacement"
        );
        assert_eq!(
            [header.get(0), header.get(1), header.get(2)],
            [Some(Some(a)), Some(Some(b)), Some(Some(peer))]
        );
        assert_eq!(
            owner
                .shared_topology
                .node_groups
                .get(&(a, OwnerGroupKind::Cluster)),
            Some(&Some(group))
        );
        assert_eq!(
            owner
                .shared_topology
                .node_groups
                .get(&(b, OwnerGroupKind::Cluster)),
            Some(&Some(group))
        );
        assert!(
            shared_map.same_map_allocation(&owner.owner_container_map(OwnerGraphId(0)).unwrap())
        );
    }
}
