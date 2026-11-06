// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Included in shared_topology. Caller-owned []*EdgeAbduction headers retain
// their nullable backing independently of the legacy dense record adapter.
#[derive(Clone, Debug)]
pub(super) struct OwnerAbductionHeader {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    slice: SharedAbductionSlice,
}
impl OwnerAbductionHeader {
    pub(super) fn is_nil(&self) -> bool {
        self.slice.is_nil()
    }
    pub(super) fn len(&self) -> usize {
        self.slice.len()
    }
    pub(super) fn capacity(&self) -> usize {
        self.slice.capacity()
    }
    pub(super) fn get(&self, index: usize) -> Option<Option<OwnerAbductionId>> {
        self.slice.get(index)
    }
    pub(super) fn read_capacity_slot(&self, index: usize) -> Option<Option<OwnerAbductionId>> {
        self.slice.read_capacity_slot(index)
    }
    pub(super) fn iter_slots(&self) -> impl Iterator<Item = Option<OwnerAbductionId>> + use<> {
        self.slice.iter_slots()
    }
    pub(super) fn borrowed_slots(&self) -> impl Iterator<Item = Option<OwnerAbductionId>> + '_ {
        self.slice.borrowed_slots()
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
}
impl ArenaGraph {
    pub(super) fn validate_owner_abduction_header(
        &self,
        header: &OwnerAbductionHeader,
    ) -> Result<(), OwnerTopologyError> {
        if Some(header.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(header.arena),
            });
        }
        if !MemberScopeId::same_scope(&header.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignAbductionScope);
        }
        // Do not scan future slots here: the source tests each range local
        // after its own Step, and cancellation has priority over nil/record errors.
        Ok(())
    }
    pub(super) fn allocate_owner_abduction_header(
        &mut self,
        slots: &[Option<OwnerAbductionId>],
        capacity: usize,
        present: bool,
    ) -> Result<OwnerAbductionHeader, OwnerTopologyError> {
        self.initialize_shared_topology();
        for id in slots.iter().flatten() {
            self.owner_edge_abduction(*id)
                .ok_or(OwnerTopologyError::UnknownRecord(*id))?;
        }
        let slice = if present {
            let slice = SharedAbductionSlice::with_len_cap(slots.len(), capacity)
                .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
            for (index, &slot) in slots.iter().enumerate() {
                slice
                    .write_capacity_slot(index, slot)
                    .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
            }
            slice
        } else {
            if !slots.is_empty() || capacity != 0 {
                return Err(OwnerTopologyError::ContainerSlice(
                    "nil abduction bounds".into(),
                ));
            }
            SharedAbductionSlice::nil()
        };
        Ok(OwnerAbductionHeader {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            slice,
        })
    }
    pub(super) fn write_owner_abduction_capacity_slot(
        &mut self,
        header: &OwnerAbductionHeader,
        index: usize,
        value: Option<OwnerAbductionId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_abduction_header(header)?;
        if let Some(id) = value {
            self.owner_edge_abduction(id)
                .ok_or(OwnerTopologyError::UnknownRecord(id))?;
        }
        header
            .slice
            .write_capacity_slot(index, value)
            .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))
    }
    pub(super) fn set_owner_edge_abduction(
        &mut self,
        id: OwnerAbductionId,
        record: OwnerEdgeAbduction,
    ) -> Result<(), OwnerTopologyError> {
        self.owner_edge_abduction(id)
            .ok_or(OwnerTopologyError::UnknownRecord(id))?;
        if let Some(edge) = record.source_edge { self.require_topology_edge(edge)?; }
        for node in [
            record.originally_from,
            record.originally_to,
            record.current_from,
            record.current_to,
        ]
        .into_iter()
        .flatten()
        {
            self.require_topology_node(node)?;
        }
        self.shared_topology.records[id.0] = Some(record);
        Ok(())
    }
}
