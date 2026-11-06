// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Included in shared_topology; the field/header scope cannot be forged.
#[derive(Clone, Debug)]
pub(super) struct OwnerNodePoint {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    point: Option<SharedOwnerPoint>,
}
impl OwnerNodePoint {
    /// The same geo.Point allocation used by route endpoints and node origins.
    pub(super) fn shared_point(&self) -> Option<SharedOwnerPoint> {
        self.point.clone()
    }
    pub(super) fn get(&self) -> Option<Point> {
        self.point.as_ref().map(SharedOwnerPoint::get)
    }
    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        match (&self.point, &other.point) {
            (Some(a), Some(b)) => a.same_allocation(b),
            (None, None) => true,
            _ => false,
        }
    }
    pub(super) fn snapshot(&self) -> OwnerNodePointSnapshot {
        OwnerNodePointSnapshot {
            header: self.clone(),
            value: self.get(),
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct OwnerNodePointSnapshot {
    header: OwnerNodePoint,
    value: Option<Point>,
}
impl OwnerNodePointSnapshot {
    pub(super) fn captured_value(&self) -> Option<Point> { self.value }
}
impl ArenaGraph {
    /// Materialize an inline production TopLeft before entering the pointer
    /// kernel. Existing explicit bindings retain their allocation unchanged.
    pub(super) fn materialize_owner_node_point(&mut self, node: NodeId) -> Result<bool, OwnerTopologyError> {
        self.require_topology_node(node)?;
        if self.shared_topology.points.contains_key(&node) { return Ok(false); }
        self.shared_topology.points.insert(node, self.nodes[node.0 as usize].position.map(SharedOwnerPoint::new));
        Ok(true)
    }
    pub(super) fn release_materialized_owner_node_point(&mut self, node: NodeId) {
        let value = self.position(node);
        self.shared_topology.points.remove(&node);
        self.nodes[node.0 as usize].position = value;
        if let Some(value) = value { self.nodes[node.0 as usize].rect.origin = value; }
    }
    fn validate_owner_point(&self, point: &OwnerNodePoint) -> Result<(), OwnerTopologyError> {
        if Some(point.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(point.arena),
            });
        }
        if !MemberScopeId::same_scope(&point.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignPointScope);
        }
        Ok(())
    }
    fn wrap_owner_point(&self, point: Option<SharedOwnerPoint>) -> OwnerNodePoint {
        OwnerNodePoint {
            arena: self.topology_arena_id().expect("initialized owner"),
            scope: self.shared_topology.member_scope,
            point,
        }
    }
    pub(super) fn allocate_owner_node_point(&mut self, value: Option<Point>) -> OwnerNodePoint {
        self.initialize_shared_topology();
        self.wrap_owner_point(value.map(SharedOwnerPoint::new))
    }
    pub(super) fn owner_node_point(
        &self,
        node: NodeId,
    ) -> Result<OwnerNodePoint, OwnerTopologyError> {
        self.require_topology_node(node)?;
        Ok(self.wrap_owner_point(
            self.shared_topology
                .points
                .get(&node)
                .ok_or(OwnerTopologyError::UnboundNodePoint(node))?
                .clone(),
        ))
    }
    pub(super) fn bind_owner_node_point(
        &mut self,
        node: NodeId,
        point: OwnerNodePoint,
    ) -> Result<(), OwnerTopologyError> {
        self.require_topology_node(node)?;
        self.validate_owner_point(&point)?;
        self.shared_topology.points.insert(node, point.point);
        self.nodes[node.0 as usize].position = None;
        Ok(())
    }
    #[track_caller]
    pub(super) fn write_owner_point(
        &mut self,
        point: &OwnerNodePoint,
        value: Point,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_point(point)?;
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_value("D2RS_TRACE_OWNER_POINT_MUTATION").is_some()
            && let Some(backing) = point.point.as_ref() {
            for (node, bound) in self.shared_topology.points.iter().collect::<std::collections::BTreeMap<_, _>>() {
                if bound.as_ref().is_some_and(|bound| bound.same_allocation(backing)) {
                    self.trace_owner_point_write(node, "write", point.get(), Some(value));
                }
            }
        }
        point
            .point
            .as_ref()
            .expect("runtime error: invalid memory address or nil pointer dereference")
            .set(value);
        Ok(())
    }
    pub(super) fn restore_owner_node_point(
        &mut self,
        node: NodeId,
        snapshot: &OwnerNodePointSnapshot,
    ) -> Result<(), OwnerTopologyError> {
        self.validate_owner_point(&snapshot.header)?;
        if let Some(value) = snapshot.value {
            snapshot.header.point.as_ref().unwrap().set(value);
        }
        self.bind_owner_node_point(node, snapshot.header.clone())
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn owner_bound_point_address(&self, node: NodeId) -> Option<Option<usize>> {
        self.shared_topology
            .points
            .get(&node)
            .map(|p| p.as_ref().map(SharedOwnerPoint::point_address))
    }
    pub(super) fn owner_bound_point_value(&self, node: NodeId) -> Option<Option<Point>> {
        self.shared_topology
            .points
            .get(&node)
            .map(|p| p.as_ref().map(SharedOwnerPoint::get))
    }
    pub(super) fn replace_bound_point_value(&mut self, node: NodeId, value: Option<Point>) -> bool {
        if let Some(point) = self.shared_topology.points.get_mut(&node) {
            *point = value.map(SharedOwnerPoint::new);
            true
        } else {
            false
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    #[track_caller]
    fn trace_owner_point_write(&self, node: NodeId, operation: &str, previous: Option<Point>, next: Option<Point>) {
        if crate::internal::trace_env_value("D2RS_TRACE_OWNER_POINT_MUTATION")
            .is_some_and(|ids| ids.split(',').any(|id| id.parse::<i64>().ok() == Some(self.nodes[node.0 as usize].tala_id)))
        {
            eprintln!("OWNER_POINT_MUTATION operation={} caller={} canonical={} tala={} previous={:?} next={:?}", operation, std::panic::Location::caller(), node.0, self.nodes[node.0 as usize].tala_id, previous, next);
        }
    }
    #[track_caller]
    pub(super) fn translate_owner_point(&mut self, node: NodeId, dx: f64, dy: f64) {
        #[cfg(feature = "diagnostic-traces")]
        self.trace_owner_point_write(node, "translate", self.position(node), self.position(node).map(|point| Point { x: point.x + dx, y: point.y + dy }));
        if let Some(point) = self.shared_topology.points.get(&node) {
            point
                .as_ref()
                .expect("runtime error: invalid memory address or nil pointer dereference")
                .translate(dx, dy);
        } else {
            let point = self.nodes[node.0 as usize]
                .position
                .as_mut()
                .expect("runtime error: invalid memory address or nil pointer dereference");
            point.x += dx;
            point.y += dy;
        }
        self.nodes[node.0 as usize].rect.origin = self.position(node).unwrap();
    }
}
