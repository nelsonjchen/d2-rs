// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Included in shared_topology: handles carry the actual owner namespace.
#[derive(Debug)]
pub(super) struct OwnerHerdMap {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    map: super::node_near_map::NodeNearMap,
}
impl Clone for OwnerHerdMap {
    fn clone(&self) -> Self {
        Self {
            arena: self.arena,
            scope: self.scope,
            map: self.map.retained(),
        }
    }
}
impl OwnerHerdMap {
    pub(super) fn is_nil(&self) -> bool {
        self.map.is_nil()
    }
    pub(super) fn len(&self) -> usize {
        self.map.len()
    }
    pub(super) fn keys(&self) -> Vec<Option<NodeId>> {
        self.map.raw_keys()
    }
    pub(super) fn same_map_allocation(&self, other: &Self) -> bool {
        self.map.same_allocation(&other.map)
    }
    fn detached(&self, scope: &MemberScopeId) -> Self {
        Self {
            arena: self.arena,
            scope: *scope,
            map: self.map.clone(),
        }
    }
}

/// A Go struct-value copy retains its maps' actual headers. Deep map copying
/// belongs to HerdAssignment.Copy, not optimizer rollback.
#[derive(Clone, Debug)]
pub(super) struct OwnerHerdFields {
    pub(super) orientation: i64,
    pub(super) value: f64,
    pub(super) same_side: OwnerHerdMap,
    pub(super) opposite_side: OwnerHerdMap,
}
#[derive(Clone, Debug)]
pub(super) struct OwnerHerd {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    fields: Arc<crate::internal::go_cell::GoCell<OwnerHerdFields>>,
}
impl OwnerHerd {
    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.fields, &other.fields)
    }
    pub(super) fn allocation_key(&self) -> usize {
        Arc::as_ptr(&self.fields) as usize
    }
    fn detached(&self, scope: &MemberScopeId) -> Self {
        let value = self.fields.read().expect("owner herd read");
        Self {
            arena: self.arena,
            scope: *scope,
            fields: Arc::new(crate::internal::go_cell::GoCell::new(OwnerHerdFields {
                orientation: value.orientation,
                value: value.value,
                same_side: value.same_side.detached(scope),
                opposite_side: value.opposite_side.detached(scope),
            })),
        }
    }
}
impl ArenaGraph {
    pub(super) fn has_herd_assignment(&self, node: NodeId) -> bool {
        self.shared_topology.node_herds.get(&node)
            .map_or_else(|| self.nodes[node.0 as usize].herd_assignment.is_some(), Option::is_some)
    }
    /// Numeric placement projections read values from the actual record. Pair
    /// keys here are canonical owner indices (nil=-1), never TALA entity IDs.
    /// This value copy does not replace the owner's record or map headers.
    pub(super) fn herd_assignment_snapshot(&self, node: NodeId) -> Option<super::HerdAssignment> {
        if let Some(binding) = self.shared_topology.node_herds.get(&node) {
            let herd = binding.as_ref()?;
            let fields = herd.fields.read().expect("owner herd read");
            let orientation = match fields.orientation {
                0 => super::Orientation::TopLeft,
                1 => super::Orientation::TopRight,
                2 => super::Orientation::BottomLeft,
                3 => super::Orientation::BottomRight,
                4 => super::Orientation::Top,
                5 => super::Orientation::Right,
                6 => super::Orientation::Bottom,
                7 => super::Orientation::Left,
                8 => super::Orientation::None,
                raw => panic!("numeric herd projection cannot represent raw source orientation {raw}"),
            };
            let keys = |map: &OwnerHerdMap| map.keys().into_iter()
                .map(|key| key.map_or(-1, |node| i64::from(node.0))).collect();
            return Some(super::HerdAssignment {
                orientation, value: fields.value,
                same_side_paired: keys(&fields.same_side),
                opposite_side_paired: keys(&fields.opposite_side),
            });
        }
        self.nodes[node.0 as usize].herd_assignment.clone()
    }

    pub(super) fn herd_orientation_raw(orientation: super::Orientation) -> i64 {
        match orientation {
            super::Orientation::TopLeft => 0, super::Orientation::TopRight => 1,
            super::Orientation::BottomLeft => 2, super::Orientation::BottomRight => 3,
            super::Orientation::Top => 4, super::Orientation::Right => 5,
            super::Orientation::Bottom => 6, super::Orientation::Left => 7,
            super::Orientation::None => 8,
        }
    }

    pub(super) fn create_node_herd(&mut self, node: NodeId, orientation: super::Orientation) {
        let herd = self.new_owner_herd();
        let mut fields = self.owner_herd_fields(&herd).expect("new herd fields");
        fields.orientation = Self::herd_orientation_raw(orientation);
        self.set_owner_herd_fields(&herd, fields).expect("new herd orientation");
        self.set_owner_node_herd(node, Some(herd)).expect("canonical herd node");
    }

    pub(super) fn record_node_herd_pair(&mut self, node: NodeId, uncle: NodeId, opposite: bool) {
        if let Some(Some(herd)) = self.shared_topology.node_herds.get(&node).cloned() {
            self.pair_owner_herd(Some(&herd), Some(uncle), opposite).expect("canonical herd pair");
        } else if let Some(assignment) = self.nodes[node.0 as usize].herd_assignment.as_mut() {
            // Historical numerical fixtures and unbound aggregate scaffolding
            // retain values only; this branch does not construct an owner map.
            let pairs = if opposite { &mut assignment.opposite_side_paired } else { &mut assignment.same_side_paired };
            pairs.insert(i64::from(uncle.0));
        }
    }

    pub(super) fn copy_node_herd(&mut self, source: NodeId, destination: NodeId) {
        if let Some(Some(herd)) = self.shared_topology.node_herds.get(&source).cloned() {
            let copy = self.copy_owner_herd(Some(&herd)).expect("actual herd copy");
            self.set_owner_node_herd(destination, Some(copy)).expect("canonical copied herd node");
        } else {
            let value = self.herd_assignment_snapshot(source);
            self.publish_herd_projection(destination, value);
        }
    }

    /// Sized placement changes scalar fields; its value copy cannot reconstruct
    /// the actual pairing-map allocations or replace an existing record.
    pub(super) fn publish_herd_projection(&mut self, node: NodeId, value: Option<super::HerdAssignment>) {
        if let Some(Some(herd)) = self.shared_topology.node_herds.get(&node) {
            if let Some(value) = value {
                let mut fields = herd.fields.write().expect("owner herd write");
                fields.orientation = Self::herd_orientation_raw(value.orientation);
                fields.value = value.value;
                self.nodes[node.0 as usize].herd_assignment = None;
                return;
            }
            self.set_owner_node_herd(node, None).expect("actual herd pointer clear");
        } else {
            self.nodes[node.0 as usize].herd_assignment = value;
        }
    }

    pub(super) fn allocate_owner_herd_map(&mut self, present: bool) -> OwnerHerdMap {
        self.initialize_shared_topology();
        OwnerHerdMap {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            map: if present {
                super::node_near_map::NodeNearMap::allocated()
            } else {
                super::node_near_map::NodeNearMap::nil()
            },
        }
    }
    pub(super) fn new_owner_herd(&mut self) -> OwnerHerd {
        let same_side = self.allocate_owner_herd_map(true);
        let opposite_side = self.allocate_owner_herd_map(true);
        OwnerHerd {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            fields: Arc::new(crate::internal::go_cell::GoCell::new(OwnerHerdFields {
                orientation: 0,
                value: 0.0,
                same_side,
                opposite_side,
            })),
        }
    }
    fn validate_owner_herd(&self, herd: &OwnerHerd) -> Result<(), OwnerTopologyError> {
        if Some(herd.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(herd.arena),
            });
        }
        if !MemberScopeId::same_scope(&herd.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignHerdScope);
        }
        Ok(())
    }
    fn validate_owner_herd_map(&self, map: &OwnerHerdMap) -> Result<(), OwnerTopologyError> {
        if Some(map.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(map.arena),
            });
        }
        if !MemberScopeId::same_scope(&map.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignHerdScope);
        }
        for node in map.keys().into_iter().flatten() {
            self.require_topology_node(node)?;
        }
        Ok(())
    }
    /// Scalar scoring view; no herd/map handles leave this read segment.
    pub(super) fn owner_node_herd_geometry(
        &self,
        node: NodeId,
    ) -> Result<Option<(i64, f64)>, OwnerTopologyError> {
        self.require_topology_node(node)?;
        if let Some(herd) = self.shared_topology.node_herds.get(&node) {
            return herd.as_ref().map(|herd| {
                self.validate_owner_herd(herd)?;
                let fields = herd.fields.read().expect("owner herd read");
                Ok((fields.orientation, fields.value))
            }).transpose();
        }
        if self.nodes[node.0 as usize].herd_assignment.is_some() {
            return Err(OwnerTopologyError::UnboundHerdAssignment(node));
        }
        Ok(None)
    }
    pub(super) fn owner_herd_fields(
        &self,
        herd: &OwnerHerd,
    ) -> Result<OwnerHerdFields, OwnerTopologyError> {
        self.validate_owner_herd(herd)?;
        Ok(herd.fields.read().expect("owner herd read").clone())
    }
    pub(super) fn set_owner_herd_fields(
        &mut self,
        herd: &OwnerHerd,
        fields: OwnerHerdFields,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_herd(herd)?;
        self.validate_owner_herd_map(&fields.same_side)?;
        self.validate_owner_herd_map(&fields.opposite_side)?;
        *herd.fields.write().expect("owner herd write") = fields;
        Ok(())
    }
    pub(super) fn owner_node_herd(
        &self,
        node: NodeId,
    ) -> Result<Option<OwnerHerd>, OwnerTopologyError> {
        self.require_topology_node(node)?;
        if let Some(herd) = self.shared_topology.node_herds.get(&node) {
            return Ok(herd.clone());
        }
        if self.nodes[node.0 as usize].herd_assignment.is_some() {
            return Err(OwnerTopologyError::UnboundHerdAssignment(node));
        }
        Ok(None)
    }
    pub(super) fn set_owner_node_herd(
        &mut self,
        node: NodeId,
        herd: Option<OwnerHerd>,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        self.require_topology_node(node)?;
        if let Some(herd) = &herd {
            self.validate_owner_herd(herd)?;
        }
        self.nodes[node.0 as usize].herd_assignment = None;
        self.shared_topology.node_herds.insert(node, herd);
        Ok(())
    }
    /// herding.go34–41 clones each map separately, even when the two original
    /// fields alias. Source nil maps stay nil and the record is distinct.
    pub(super) fn copy_owner_herd(
        &mut self,
        herd: Option<&OwnerHerd>,
    ) -> Result<OwnerHerd, OwnerTopologyError> {
        let herd = herd.expect("runtime error: invalid memory address or nil pointer dereference");
        self.validate_owner_herd(herd)?;
        Ok(herd.detached(&self.shared_topology.member_scope))
    }
    /// structure_api.go68–73: assign into the current field without allocating
    /// a nil map. Map locks and record locks never cross a context callback.
    pub(super) fn pair_owner_herd(
        &mut self,
        herd: Option<&OwnerHerd>,
        node: Option<NodeId>,
        opposite: bool,
    ) -> Result<(), OwnerTopologyError> {
        let herd = herd.expect("runtime error: invalid memory address or nil pointer dereference");
        self.validate_owner_herd(herd)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        let fields = self.owner_herd_fields(herd)?;
        let map = if opposite {
            fields.opposite_side
        } else {
            fields.same_side
        };
        map.map
            .insert(node)
            .unwrap_or_else(|_| panic!("assignment to entry in nil map"));
        Ok(())
    }
    pub(super) fn delete_owner_herd_pair(
        &mut self,
        map: &OwnerHerdMap,
        node: Option<NodeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_herd_map(map)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        map.map.delete(node);
        Ok(())
    }
}
