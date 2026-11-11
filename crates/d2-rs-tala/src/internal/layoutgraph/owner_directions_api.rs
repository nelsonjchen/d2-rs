// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// graph.go86–108/1074–1081: independent NewGraph maps and CopyEntitiesFrom aliases.
type DirectionEntries = BTreeMap<Option<NodeId>, Orientation>;
type DirectionBacking = Arc<crate::internal::go_cell::GoCell<DirectionEntries>>;
#[derive(Clone, Debug)]
pub(super) struct OwnerGraphDirectionHeader {
    arena: OwnerArenaId,
    scope: MemberScopeId,
    entries: Option<DirectionBacking>,
}
impl OwnerGraphDirectionHeader {
    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        match (&self.entries, &other.entries) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
    fn get(&self, key: Option<NodeId>) -> Option<Orientation> {
        self.entries.as_ref().and_then(|entries| {
            entries
                .read()
                .expect("direction map poisoned")
                .get(&key)
                .copied()
        })
    }
    fn set(&self, key: Option<NodeId>, value: Option<Orientation>) {
        match (&self.entries, value) {
            (None, None) => {}
            (None, Some(_)) => panic!("assignment to entry in nil map"),
            (Some(entries), value) => {
                let mut entries = entries.write().expect("direction map poisoned");
                if let Some(value) = value {
                    entries.insert(key, value);
                } else {
                    entries.remove(&key);
                }
            }
        }
    }
    fn fork(&self, maps: &mut BTreeMap<usize, DirectionBacking>) -> Self {
        let entries = self.entries.as_ref().map(|entries| {
            maps.entry(Arc::as_ptr(entries) as usize)
                .or_insert_with(|| {
                    Arc::new(crate::internal::go_cell::GoCell::new(
                        entries.read().expect("direction map poisoned").clone(),
                    ))
                })
                .clone()
        });
        Self {
            arena: self.arena,
            scope: self.scope,
            entries,
        }
    }
}
impl ArenaGraph {
    pub(super) fn new_owner_direction_header(
        &mut self,
        allocated: bool,
    ) -> OwnerGraphDirectionHeader {
        self.initialize_shared_topology();
        OwnerGraphDirectionHeader {
            arena: self.topology_arena_id().unwrap(),
            scope: self.shared_topology.member_scope,
            entries: allocated.then(|| Arc::new(crate::internal::go_cell::GoCell::new(BTreeMap::new()))),
        }
    }
    pub(super) fn owner_graph_direction_header(
        &self,
        view: OwnerGraphId,
    ) -> Result<Option<OwnerGraphDirectionHeader>, OwnerTopologyError> {
        Ok(self
            .owner_graph_view(view)
            .ok_or(OwnerTopologyError::UnknownGraph(view))?
            .directions
            .clone())
    }
    pub(super) fn set_owner_graph_direction_header(
        &mut self,
        view: OwnerGraphId,
        header: OwnerGraphDirectionHeader,
    ) -> Result<(), OwnerTopologyError> {
        if Some(header.arena) != self.topology_arena_id() {
            return Err(OwnerTopologyError::ForeignArena {
                expected: self.topology_arena_id(),
                actual: Some(header.arena),
            });
        }
        if !MemberScopeId::same_scope(&header.scope, &self.shared_topology.member_scope) {
            return Err(OwnerTopologyError::ForeignMemberScope);
        }
        self.shared_topology
            .views
            .get_mut(view.0)
            .and_then(Option::as_mut)
            .ok_or(OwnerTopologyError::UnknownGraph(view))?
            .directions = Some(header);
        Ok(())
    }
    pub(super) fn copy_owner_graph_directions_from(
        &mut self,
        destination: OwnerGraphId,
        source: OwnerGraphId,
    ) -> Result<(), OwnerTopologyError> {
        let header = self
            .owner_graph_view(source)
            .ok_or(OwnerTopologyError::UnknownGraph(source))?
            .directions
            .clone();
        self.shared_topology
            .views
            .get_mut(destination.0)
            .and_then(Option::as_mut)
            .ok_or(OwnerTopologyError::UnknownGraph(destination))?
            .directions = header;
        Ok(())
    }
}

#[cfg(test)]
mod direction_header_tests {
    use super::*;
    #[test]
    fn prepared_component_inherits_actual_independent_direction_header() {
        let mut input = Graph::default();
        for id in ["a", "b"] {
            input.add_node(crate::Node::new(
                id,
                Size {
                    width: 1.,
                    height: 1.,
                },
            ));
        }
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        let header = owner.new_owner_direction_header(true);
        owner
            .set_owner_graph_direction_header(OwnerGraphId(0), header.clone())
            .unwrap();
        owner
            .set_owner_graph_direction_entry(OwnerGraphId(0), None, Some(Orientation::Right))
            .unwrap();
        let mapping = super::super::owner_score_session::CanonicalScoreMapping::new(
            owner.topology_arena_id().unwrap(),
            vec![NodeId(0), NodeId(1)],
            vec![],
        );
        let mut session = super::super::owner_score_session::OwnerScoreSession::new(
            &mut owner,
            OwnerGraphId(0),
            mapping,
            None,
        )
        .unwrap();
        let prepared = session
            .prepare_component(&[NodeId(0), NodeId(1)], &[], &[NodeId(0), NodeId(1)])
            .unwrap();
        let child = session
            .owner()
            .owner_graph_direction_header(prepared.view())
            .unwrap()
            .unwrap();
        assert!(header.same_allocation(&child));
        session
            .owner_mut()
            .set_owner_graph_direction_entry(prepared.view(), None, Some(Orientation::BottomLeft))
            .unwrap();
        assert_eq!(
            session
                .owner()
                .owner_graph_direction_entry(OwnerGraphId(0), None)
                .unwrap(),
            Some(Orientation::BottomLeft)
        );
    }
}
