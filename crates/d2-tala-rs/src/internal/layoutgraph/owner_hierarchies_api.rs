// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Included in shared_topology. Source: layoutgraph/hierarchy.go7–40.

/// A hierarchy record and its level map have independent identities. Replacing
/// the map does not change a Node.Hierarchy pointer or a retained map header.
#[derive(Clone, Debug)]
pub(super) struct OwnerHierarchyMap {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    values: Option<Arc<crate::internal::go_cell::GoCell<BTreeMap<Option<NodeId>, i64>>>>,
}
impl OwnerHierarchyMap {
    pub(super) fn is_nil(&self) -> bool {
        self.values.is_none()
    }
    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        match (&self.values, &other.values) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
    pub(super) fn entries(&self) -> Vec<(Option<NodeId>, i64)> {
        self.values.as_ref().map_or_else(Vec::new, |values| {
            values
                .read()
                .expect("hierarchy map read")
                .iter()
                .map(|(&key, &value)| (key, value))
                .collect()
        })
    }
    fn detached(&self, scope: &MemberScopeId) -> Self {
        Self {
            arena: self.arena,
            scope: *scope,
            values: self.values.as_ref().map(|values| {
                Arc::new(crate::internal::go_cell::GoCell::new(
                    values.read().expect("hierarchy map read").clone(),
                ))
            }),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct OwnerHierarchyFields {
    pub(super) levels: OwnerHierarchyMap,
    pub(super) level_count: i64,
}
#[derive(Clone, Debug)]
pub(super) struct OwnerHierarchy {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    fields: Arc<crate::internal::go_cell::GoCell<OwnerHierarchyFields>>,
}
impl OwnerHierarchy {
    pub(super) fn level(&self, node: NodeId) -> i64 {
        let fields = self.fields.read().expect("hierarchy read");
        fields.levels.values.as_ref().and_then(|map| {
            map.read().expect("hierarchy map read").get(&Some(node)).copied()
        }).unwrap_or(0)
    }
    pub(super) fn level_count(&self) -> i64 {
        self.fields.read().expect("hierarchy read").level_count
    }
    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.fields, &other.fields)
    }
    pub(super) fn allocation_key(&self) -> usize {
        Arc::as_ptr(&self.fields) as usize
    }
    fn snapshot(&self) -> OwnerHierarchySnapshot {
        let fields = self.fields.read().expect("hierarchy read").clone();
        let entries = fields.levels.entries();
        OwnerHierarchySnapshot {
            hierarchy: self.clone(),
            fields,
            entries,
        }
    }
}
#[derive(Debug)]
struct OwnerHierarchySnapshot {
    hierarchy: OwnerHierarchy,
    fields: OwnerHierarchyFields,
    entries: Vec<(Option<NodeId>, i64)>,
}
impl OwnerHierarchySnapshot {
    fn restore(self) {
        if let Some(values) = &self.fields.levels.values {
            let mut values = values.write().expect("hierarchy map restore");
            values.clear();
            values.extend(self.entries);
        }
        *self.hierarchy.fields.write().expect("hierarchy restore") = self.fields;
    }
}

impl ArenaGraph {
    /// Numerical candidates detach each actual record and each map allocation
    /// once, including aliases retained by projected external node records.
    pub(super) fn detach_hierarchy_storage(&mut self) {
        let mut records = BTreeMap::<usize, OwnerHierarchy>::new();
        let mut maps = Vec::<(OwnerHierarchyMap, OwnerHierarchyMap)>::new();
        let scope = self.shared_topology.member_scope;
        let mut detach = |node: &mut ArenaNode| {
            let Some(membership) = &mut node.hierarchy else { return; };
            let hierarchy = &membership.record;
            let detached = records.entry(hierarchy.allocation_key()).or_insert_with(|| {
                let mut fields = hierarchy.fields.read().expect("hierarchy read").clone();
                let levels = if let Some((_, levels)) = maps.iter()
                    .find(|(old, _)| old.same_allocation(&fields.levels)) {
                    levels.clone()
                } else {
                    let levels = fields.levels.detached(&scope);
                    maps.push((fields.levels.clone(), levels.clone()));
                    levels
                };
                fields.levels = levels;
                OwnerHierarchy { arena: hierarchy.arena, scope: scope,
                    fields: Arc::new(crate::internal::go_cell::GoCell::new(fields)) }
            });
            membership.record = detached.clone();
        };
        for node in &mut self.nodes { detach(node); }
        for node in &mut self.transaction_external_containers { detach(node); }
        for node in &mut self.transaction_validation_external_containers { detach(node); }
        for projected in &mut self.projected_transaction_nodes { detach(&mut projected.node); }
        for nodes in self.transaction_external_container_children.values_mut()
            .chain(self.transaction_external_aggregate_children.values_mut())
            .chain(self.trace_container_children.values_mut()) {
            for node in nodes { detach(node); }
        }
    }
}

impl ArenaGraph {
    pub(super) fn assign_hierarchy_member(
        &mut self, node: NodeId, id: usize, scope: Option<NodeId>, level: usize, level_count: usize,
    ) {
        let record = self.nodes.iter().filter_map(|node| node.hierarchy.as_ref())
            .find(|h| h.id == id && h.scope == scope).map(|h| h.record.clone())
            .unwrap_or_else(|| self.new_owner_hierarchy());
        let mut fields = self.owner_hierarchy_fields(&record).expect("live hierarchy record");
        fields.level_count = level_count as i64;
        self.set_owner_hierarchy_fields(&record, fields).expect("hierarchy count");
        let levels = self.owner_hierarchy_levels(&record).expect("hierarchy levels");
        self.write_owner_hierarchy_level(&levels, Some(node), level as i64).expect("hierarchy member");
        self.nodes[node.0 as usize].hierarchy = Some(HierarchyMembership { id, scope, record, node });
    }

    /// Clear the current arena's assignments in two passes, as Assign does:
    /// a record can retain a key whose node now points to another hierarchy.
    /// The full guarded OwnedNodes discovery remains a separate stage contract.
    pub(super) fn clear_hierarchy_memberships(&mut self) {
        let owned = self.nodes.iter().map(|node| node.hierarchy.as_ref()
            .map_or(node.input_id, |h| h.node)).collect::<BTreeSet<_>>();
        let records = self.nodes.iter().filter_map(|node| node.hierarchy.as_ref())
            .map(|h| (h.record.allocation_key(), h.record.clone())).collect::<BTreeMap<_, _>>();
        for record in records.values() {
            let mut fields = record.fields.write().expect("hierarchy write");
            if fields.levels.is_nil() { fields.levels = self.allocate_owner_hierarchy_map(true); }
            fields.levels.values.as_ref().unwrap().write().expect("hierarchy map write")
                .retain(|node, _| !node.is_some_and(|node| owned.contains(&node)));
        }
        for node in &mut self.nodes { node.hierarchy = None; }
    }

    /// A member leaving the record must also disappear from its retained map.
    pub(super) fn remove_hierarchy_member(&mut self, node: NodeId) {
        if let Some(h) = self.nodes[node.0 as usize].hierarchy.take() {
            let mut fields = h.record.fields.write().expect("hierarchy write");
            if fields.levels.is_nil() {
                fields.levels = self.allocate_owner_hierarchy_map(true);
            }
            fields.levels.values.as_ref().unwrap().write().expect("hierarchy map write")
                .remove(&Some(h.node));
        }
    }
    pub(super) fn allocate_owner_hierarchy_map(&mut self, present: bool) -> OwnerHierarchyMap {
        self.initialize_shared_topology();
        OwnerHierarchyMap {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            values: present.then(|| Arc::new(crate::internal::go_cell::GoCell::new(BTreeMap::new()))),
        }
    }
    pub(super) fn new_owner_hierarchy(&mut self) -> OwnerHierarchy {
        let levels = self.allocate_owner_hierarchy_map(true);
        OwnerHierarchy {
            arena: levels.arena,
            scope: levels.scope,
            fields: Arc::new(crate::internal::go_cell::GoCell::new(OwnerHierarchyFields {
                levels,
                level_count: 0,
            })),
        }
    }
    fn validate_owner_hierarchy(
        &self,
        hierarchy: &OwnerHierarchy,
    ) -> Result<(), OwnerTopologyError> {
        if Some(hierarchy.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(hierarchy.arena),
            });
        }
        if !MemberScopeId::same_scope(&hierarchy.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignHierarchyScope);
        }
        Ok(())
    }
    fn validate_owner_hierarchy_map(
        &self,
        map: &OwnerHierarchyMap,
    ) -> Result<(), OwnerTopologyError> {
        if Some(map.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(map.arena),
            });
        }
        if !MemberScopeId::same_scope(&map.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignHierarchyScope);
        }
        Ok(())
    }
    pub(super) fn owner_hierarchy_fields(
        &self,
        hierarchy: &OwnerHierarchy,
    ) -> Result<OwnerHierarchyFields, OwnerTopologyError> {
        self.validate_owner_hierarchy(hierarchy)?;
        Ok(hierarchy.fields.read().expect("hierarchy read").clone())
    }
    pub(super) fn set_owner_hierarchy_fields(
        &mut self,
        hierarchy: &OwnerHierarchy,
        fields: OwnerHierarchyFields,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_hierarchy(hierarchy)?;
        self.validate_owner_hierarchy_map(&fields.levels)?;
        *hierarchy.fields.write().expect("hierarchy write") = fields;
        Ok(())
    }
    /// Source Levels lazily allocates a nil map and returns that same header.
    pub(super) fn owner_hierarchy_levels(
        &mut self,
        hierarchy: &OwnerHierarchy,
    ) -> Result<OwnerHierarchyMap, OwnerTopologyError> {
        self.validate_owner_hierarchy(hierarchy)?;
        let mut fields = self.owner_hierarchy_fields(hierarchy)?;
        if fields.levels.is_nil() {
            fields.levels = self.allocate_owner_hierarchy_map(true);
            self.set_owner_hierarchy_fields(hierarchy, fields.clone())?;
        }
        Ok(fields.levels)
    }
    pub(super) fn write_owner_hierarchy_level(
        &self,
        map: &OwnerHierarchyMap,
        node: Option<NodeId>,
        level: i64,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_hierarchy_map(map)?;
        if let Some(node) = node {
            self.require_topology_node(node)?;
        }
        map.values
            .as_ref()
            .expect("assignment to entry in nil map")
            .write()
            .expect("hierarchy map write")
            .insert(node, level);
        Ok(())
    }
    pub(super) fn delete_owner_hierarchy_level(
        &self,
        map: &OwnerHierarchyMap,
        node: Option<NodeId>,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_hierarchy_map(map)?;
        if let Some(values) = &map.values {
            values.write().expect("hierarchy map write").remove(&node);
        }
        Ok(())
    }
    pub(super) fn owner_node_hierarchy(
        &self,
        node: NodeId,
    ) -> Result<Option<OwnerHierarchy>, OwnerTopologyError> {
        self.require_topology_node(node)?;
        Ok(self.nodes[node.0 as usize].hierarchy.as_ref().map(|h| h.record.clone()))
    }
    pub(super) fn set_owner_node_hierarchy(
        &mut self,
        node: NodeId,
        hierarchy: Option<OwnerHierarchy>,
    ) -> Result<(), OwnerTopologyError> {
        self.initialize_shared_topology();
        self.require_topology_node(node)?;
        if let Some(hierarchy) = &hierarchy {
            self.validate_owner_hierarchy(hierarchy)?;
        }
        self.nodes[node.0 as usize].hierarchy = hierarchy.map(|record| {
            let existing = self.nodes.iter().filter_map(|node| node.hierarchy.as_ref())
                .find(|membership| membership.record.same_allocation(&record));
            let (id, scope) = existing.map(|h| (h.id, h.scope)).unwrap_or_else(|| {
                (self.nodes.iter().filter_map(|n| n.hierarchy.as_ref().map(|h| h.id + 1))
                    .max().unwrap_or(0), None)
            });
            HierarchyMembership { id, scope, record, node }
        });
        Ok(())
    }
    pub(super) fn owner_hierarchy_level(&self, node: NodeId) -> Result<i64, OwnerTopologyError> {
        let hierarchy = self
            .owner_node_hierarchy(node)?
            .expect("runtime error: invalid memory address or nil pointer dereference");
        let fields = self.owner_hierarchy_fields(&hierarchy)?;
        Ok(fields
            .levels
            .values
            .as_ref()
            .and_then(|map| {
                map.read()
                    .expect("hierarchy map read")
                    .get(&Some(node))
                    .copied()
            })
            .unwrap_or(0))
    }
}
