// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Included in shared_topology: trusted raw headers retain their private scope.
#[derive(Clone, Debug)]
pub(super) struct OwnerIncidentHeader {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    slice: SharedEdgeSlice,
}
impl OwnerIncidentHeader {
    pub(super) fn is_nil(&self) -> bool {
        self.slice.is_nil()
    }
    pub(super) fn len(&self) -> usize {
        self.slice.len()
    }
    pub(super) fn capacity(&self) -> usize {
        self.slice.capacity()
    }
    pub(super) fn get(&self, index: usize) -> Option<Option<EdgeId>> {
        self.slice.get(index)
    }
    pub(super) fn read_capacity_slot(&self, index: usize) -> Option<Option<EdgeId>> {
        self.slice.read_capacity_slot(index)
    }
    pub(super) fn iter_slots(&self) -> impl Iterator<Item = Option<EdgeId>> + use<> {
        self.slice.iter_slots()
    }
    pub(super) fn reslice_with_capacity(
        &self,
        low: usize,
        high: usize,
        max: usize,
    ) -> Result<Self, OwnerTopologyError> {
        Ok(Self {
            arena: self.arena,
            scope: self.scope,
            slice: self
                .slice
                .reslice_with_capacity(low, high, max)
                .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?,
        })
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn slot_address(&self, index: usize) -> Option<usize> {
        self.slice.slot_address(index)
    }
}

impl ArenaGraph {
    pub(super) fn materialize_owner_node_incident_header(&mut self, node: NodeId) -> Result<bool, OwnerTopologyError> {
        self.require_topology_node(node)?;
        // Node.Edges is always the real header; there is nothing to infer.
        Ok(false)
    }
    pub(super) fn release_materialized_owner_node_incident_header(&mut self, _node: NodeId) {}
    fn validate_incident_header(
        &self,
        header: &OwnerIncidentHeader,
    ) -> Result<(), OwnerTopologyError> {
        if Some(header.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(header.arena),
            });
        }
        if !MemberScopeId::same_scope(&header.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignEdgeScope);
        }
        for index in 0..header.capacity() {
            if let Some(Some(edge)) = header.read_capacity_slot(index) {
                self.require_topology_edge(edge)?;
            }
        }
        Ok(())
    }
    fn wrap_incident_header(&self, slice: SharedEdgeSlice) -> OwnerIncidentHeader {
        OwnerIncidentHeader {
            arena: self.topology_arena_id().expect("initialized owner"),
            scope: self.shared_topology.member_scope,
            slice,
        }
    }
    pub(super) fn allocate_owner_incident_header(
        &mut self,
        slots: &[Option<EdgeId>],
        capacity: usize,
        present: bool,
    ) -> Result<OwnerIncidentHeader, OwnerTopologyError> {
        self.initialize_shared_topology();
        let slice = if present {
            let slice = SharedEdgeSlice::with_len_cap(slots.len(), capacity)
                .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
            for (index, &edge) in slots.iter().enumerate() {
                slice
                    .write_capacity_slot(index, edge)
                    .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
            }
            slice
        } else {
            if !slots.is_empty() || capacity != 0 {
                return Err(OwnerTopologyError::ContainerSlice(
                    "nil incident bounds".into(),
                ));
            }
            SharedEdgeSlice::nil()
        };
        let header = self.wrap_incident_header(slice);
        self.validate_incident_header(&header)?;
        Ok(header)
    }
    /// Retain the actual Node.Edges header, including nil and every spare slot.
    pub(super) fn owner_node_incident_header(
        &self,
        node: NodeId,
    ) -> Result<OwnerIncidentHeader, OwnerTopologyError> {
        self.require_topology_node(node)?;
        Ok(self.wrap_incident_header(self.nodes[node.0 as usize].edges.header()))
    }
    /// The node's visible incident slots, read in place. Equivalent to
    /// collecting `owner_node_incident_header(node)?.iter_slots()`.
    pub(super) fn owner_node_incident_slots(
        &self,
        node: NodeId,
    ) -> Result<Vec<Option<EdgeId>>, OwnerTopologyError> {
        self.require_topology_node(node)?;
        Ok(self.nodes[node.0 as usize]
            .edges
            .with_header(|slice| slice.visible_slots()))
    }
    /// Visit the node's visible incident slots in place. The visitor must
    /// not write this node's incidence.
    pub(super) fn for_each_owner_node_incident(
        &self,
        node: NodeId,
        visit: impl FnMut(&Option<EdgeId>),
    ) -> Result<(), OwnerTopologyError> {
        self.require_topology_node(node)?;
        self.nodes[node.0 as usize]
            .edges
            .with_header(|slice| slice.for_each_visible(visit));
        Ok(())
    }

    pub(super) fn set_owner_node_incident_header(
        &mut self,
        node: NodeId,
        header: OwnerIncidentHeader,
    ) -> Result<(), OwnerTopologyError> {
        self.require_topology_node(node)?;
        self.validate_incident_header(&header)?;
        self.nodes[node.0 as usize].edges.replace_header(header.slice);
        Ok(())
    }
    pub(super) fn owner_graph_edge_header(
        &self,
        receiver: OwnerGraphId,
    ) -> Result<OwnerIncidentHeader, OwnerTopologyError> {
        let view = self
            .owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        Ok(self.wrap_incident_header(
            view.edges.header(),
        ))
    }
    pub(super) fn set_owner_graph_edge_header(
        &mut self,
        receiver: OwnerGraphId,
        header: OwnerIncidentHeader,
    ) -> Result<(), OwnerTopologyError> {
        self.owner_graph_view(receiver)
            .ok_or(OwnerTopologyError::UnknownGraph(receiver))?;
        self.validate_incident_header(&header)?;
        self.shared_topology.views[receiver.0]
            .as_mut()
            .unwrap()
            .edges.replace_header(header.slice);
        Ok(())
    }
    pub(super) fn write_owner_incident_capacity_slot(
        &mut self,
        header: &OwnerIncidentHeader,
        index: usize,
        edge: Option<EdgeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_incident_header(header)?;
        if let Some(edge) = edge {
            self.require_topology_edge(edge)?;
        }
        header
            .slice
            .write_capacity_slot(index, edge)
            .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))
    }
    pub(super) fn append_owner_incident_slot(
        &mut self,
        node: NodeId,
        edge: Option<EdgeId>,
    ) -> Result<(), OwnerTopologyError> {
        if let Some(edge) = edge {
            self.require_topology_edge(edge)?;
        }
        let header = self.owner_node_incident_header(node)?;
        let slice = header
            .slice
            .append(&[edge])
            .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
        self.nodes[node.0 as usize].edges.replace_header(slice);
        Ok(())
    }
    pub(super) fn remove_owner_incident_slot(
        &mut self,
        node: NodeId,
        edge: Option<EdgeId>,
    ) -> Result<(), OwnerTopologyError> {
        if let Some(edge) = edge {
            self.require_topology_edge(edge)?;
        }
        let header = self.owner_node_incident_header(node)?;
        self.nodes[node.0 as usize].edges.replace_header(header.slice.remove_first_slot(edge));
        Ok(())
    }
    pub(super) fn owner_node_incident_count(&self, node: NodeId) -> usize {
        self.nodes[node.0 as usize].edges.len()
    }

    pub(super) fn owner_edge_endpoint(
        &self,
        edge: EdgeId,
        is_to: bool,
    ) -> Result<Option<NodeId>, OwnerTopologyError> {
        self.require_topology_edge(edge)?;
        if self
            .shared_topology
            .nil_edge_endpoints
            .contains(&(edge, is_to))
        {
            return Ok(None);
        }
        Ok(Some(if is_to {
            self.edge_records[edge.0 as usize].to
        } else {
            self.edge_records[edge.0 as usize].from
        }))
    }
    /// `owner_edge_endpoint(edge, false)` and `(edge, true)` with one bound
    /// check and record read; an empty nil-endpoint set needs no probes.
    pub(super) fn owner_edge_endpoint_pair(
        &self,
        edge: EdgeId,
    ) -> Result<(Option<NodeId>, Option<NodeId>), OwnerTopologyError> {
        self.require_topology_edge(edge)?;
        let record = &self.edge_records[edge.0 as usize];
        let nil = &self.shared_topology.nil_edge_endpoints;
        if nil.is_empty() {
            return Ok((Some(record.from), Some(record.to)));
        }
        Ok((
            (!nil.contains(&(edge, false))).then_some(record.from),
            (!nil.contains(&(edge, true))).then_some(record.to),
        ))
    }
    /// Raw Edge.From/To field assignment changes no incident header.
    pub(super) fn set_owner_edge_endpoint(
        &mut self,
        edge: EdgeId,
        is_to: bool,
        node: Option<NodeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.require_topology_edge(edge)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
            if is_to {
                self.edge_records[edge.0 as usize].to = node
            } else {
                self.edge_records[edge.0 as usize].from = node
            }
            self.shared_topology
                .nil_edge_endpoints
                .remove(&(edge, is_to));
        } else {
            self.shared_topology
                .nil_edge_endpoints
                .insert((edge, is_to));
        }
        Ok(())
    }
}

impl ArenaGraph {
    fn visit_incident_nodes(&self, mut visit: impl FnMut(&ArenaNode)) {
        for node in &self.nodes { visit(node); }
        for node in &self.transaction_external_containers { visit(node); }
        for node in &self.transaction_validation_external_containers { visit(node); }
        for projected in &self.projected_transaction_nodes { visit(&projected.node); }
        for nodes in self.transaction_external_container_children.values()
            .chain(self.transaction_external_aggregate_children.values())
            .chain(self.trace_container_children.values()) {
            for node in nodes { visit(node); }
        }
    }
    fn visit_incident_nodes_mut(&mut self, mut visit: impl FnMut(&mut ArenaNode)) {
        for node in &mut self.nodes { visit(node); }
        for node in &mut self.transaction_external_containers { visit(node); }
        for node in &mut self.transaction_validation_external_containers { visit(node); }
        for projected in &mut self.projected_transaction_nodes { visit(&mut projected.node); }
        for nodes in self.transaction_external_container_children.values_mut()
            .chain(self.transaction_external_aggregate_children.values_mut())
            .chain(self.trace_container_children.values_mut()) {
            for node in nodes { visit(node); }
        }
    }
    /// Fork once per real backing across Node.Edges, retained projections and
    /// Graph.Edges. Per-node numerical clones alone would break internal aliases.
    pub(super) fn detach_incident_storage_from(&mut self, original: &Self) {
        let mut headers = Vec::new();
        original.visit_incident_nodes(|node| headers.push(node.edges.header().clone()));
        headers.extend(original.shared_topology.views.iter().flatten()
            .map(|view| view.edges.header()));
        let mut detached = SharedEdgeSlice::fork_storage_batch(&headers)
            .expect("numerical incidence backing").into_iter();
        self.visit_incident_nodes_mut(|node| {
            node.edges = IncidentEdges::from_header(detached.next().expect("detached node incidence"));
        });
        for view in self.shared_topology.views.iter_mut().flatten() {
            view.edges.replace_header(detached.next().expect("detached graph edges"));
        }
        assert!(detached.next().is_none());
        if let Some(view) = self.shared_topology.views.first().and_then(Option::as_ref) {
            self.edge_order = view.edges.share_graph();
        }
    }
}
