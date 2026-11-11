// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Actual Node.LongDistanceNeighborRequirements map headers and integer values.
use super::model::LongDistanceNeighborRequirements;
use super::node_near_map::{NearCursor, NilNearMapAssignment, NodeNearMap, NodeNearMapSnapshot};
use super::{ArenaGraph, NodeId, OwnerTopologyError};
use crate::internal::go_cell::GoCell;
use std::collections::BTreeMap;
use std::sync::Arc;
#[derive(Debug, Default)]
pub(super) struct NodeRequirementsMap {
    keys: NodeNearMap,
    values: Option<Arc<GoCell<BTreeMap<Option<NodeId>, LongDistanceNeighborRequirements>>>>,
}
impl Clone for NodeRequirementsMap {
    fn clone(&self) -> Self {
        Self {
            keys: self.keys.clone(),
            values: self
                .values
                .as_ref()
                .map(|v| Arc::new(GoCell::new(v.read().expect("requirement map read").clone()))),
        }
    }
}
impl NodeRequirementsMap {
    pub(super) fn nil() -> Self {
        Self::default()
    }
    pub(super) fn allocated() -> Self {
        Self {
            keys: NodeNearMap::allocated(),
            values: Some(Arc::new(GoCell::new(BTreeMap::new()))),
        }
    }
    pub(super) fn from_raw_keys(keys: impl IntoIterator<Item = Option<NodeId>>) -> Self {
        let map = Self::allocated();
        for key in keys {
            map.set(key, LongDistanceNeighborRequirements::default())
                .unwrap();
        }
        map
    }
    pub(super) fn from_entries(
        entries: impl IntoIterator<Item = (Option<NodeId>, LongDistanceNeighborRequirements)>,
    ) -> Self {
        let map = Self::allocated();
        for (key, value) in entries {
            map.set(key, value).unwrap();
        }
        map
    }
    pub(super) fn retained(&self) -> Self {
        Self {
            keys: self.keys.retained(),
            values: self.values.as_ref().map(Arc::clone),
        }
    }
    pub(super) fn is_nil(&self) -> bool {
        self.values.is_none()
    }
    pub(super) fn len(&self) -> usize {
        self.keys.len()
    }
    pub(super) fn raw_keys(&self) -> Vec<Option<NodeId>> {
        self.keys.raw_keys()
    }
    pub(super) fn live_cursor(&self) -> NearCursor {
        self.keys.live_cursor()
    }
    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        match (&self.values, &other.values) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub(super) fn get(&self, key: Option<NodeId>) -> LongDistanceNeighborRequirements {
        self.values
            .as_ref()
            .and_then(|v| v.read().expect("requirement map read").get(&key).copied())
            .unwrap_or_default()
    }
    pub(super) fn set(
        &self,
        key: Option<NodeId>,
        value: LongDistanceNeighborRequirements,
    ) -> Result<(), NilNearMapAssignment> {
        let values = self.values.as_ref().ok_or(NilNearMapAssignment)?;
        values
            .write()
            .expect("requirement map write")
            .insert(key, value);
        self.keys.insert(key)?;
        Ok(())
    }
    pub(super) fn snapshot(&self) -> NodeRequirementsMapSnapshot {
        NodeRequirementsMapSnapshot {
            original: self.retained(),
            keys: self.keys.snapshot(),
            values: self
                .values
                .as_ref()
                .map(|v| v.read().expect("requirement map read").clone()),
        }
    }
}
#[derive(Debug)]
pub(super) struct NodeRequirementsMapSnapshot {
    original: NodeRequirementsMap,
    keys: NodeNearMapSnapshot,
    values: Option<BTreeMap<Option<NodeId>, LongDistanceNeighborRequirements>>,
}
impl NodeRequirementsMapSnapshot {
    pub(super) fn restore(self) -> NodeRequirementsMap {
        let map = self.original;
        let _ = self.keys.restore();
        if let Some(values) = &map.values {
            *values.write().expect("requirement map restore") =
                self.values.expect("allocated map values");
        }
        map
    }
}
impl ArenaGraph {
    pub(super) fn owner_node_requirement_map_read(
        &self,
        node: NodeId,
    ) -> Result<NodeRequirementsMap, OwnerTopologyError> {
        if self.nodes.get(node.0 as usize).is_none() {
            return Err(OwnerTopologyError::UnknownNode(node));
        }
        if let Some(map) = self.shared_topology.long_distance_requirements.get(&node) {
            return Ok(map.retained());
        }
        Ok(self.long_distance_neighbor_data[node.0 as usize]
            .as_ref()
            .map_or_else(NodeRequirementsMap::nil, |values| {
                NodeRequirementsMap::from_entries(
                    values.iter().map(|(&key, &value)| (Some(key), value)),
                )
            }))
    }
    pub(super) fn owner_node_requirement_map(
        &mut self,
        node: NodeId,
    ) -> Result<NodeRequirementsMap, OwnerTopologyError> {
        let map = self.owner_node_requirement_map_read(node)?;
        if !self
            .shared_topology
            .long_distance_requirements
            .contains_key(&node)
        {
            self.long_distance_neighbor_data[node.0 as usize] = None;
            self.shared_topology
                .long_distance_requirements
                .insert(node, map.retained());
        }
        Ok(map)
    }
    pub(super) fn set_owner_node_requirement_map(
        &mut self,
        node: NodeId,
        map: NodeRequirementsMap,
    ) -> Result<(), OwnerTopologyError> {
        if self.nodes.get(node.0 as usize).is_none() {
            return Err(OwnerTopologyError::UnknownNode(node));
        }
        self.long_distance_neighbor_data[node.0 as usize] = None;
        self.shared_topology
            .long_distance_requirements
            .insert(node, map);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_topology_snapshot_restores_full_values_and_clone_preserves_internal_aliases() {
        let mut input = crate::Graph::default();
        for i in 0..3 {
            input.add_node(crate::Node::new(
                format!("n{i}"),
                crate::Size {
                    width: 10.,
                    height: 8.,
                },
            ));
        }
        let mut owner = ArenaGraph::from_input(&input);
        let map = NodeRequirementsMap::allocated();
        map.set(
            Some(NodeId(2)),
            LongDistanceNeighborRequirements {
                edge_count: 259,
                max_width: 1 << 33,
                max_height: 65536,
            },
        )
        .unwrap();
        owner
            .set_owner_node_requirement_map(NodeId(0), map.retained())
            .unwrap();
        owner
            .set_owner_node_requirement_map(NodeId(1), map.retained())
            .unwrap();
        let mut copied = owner.clone();
        let copy0 = copied.owner_node_requirement_map(NodeId(0)).unwrap();
        let copy1 = copied.owner_node_requirement_map(NodeId(1)).unwrap();
        assert!(copy0.same_allocation(&copy1));
        assert!(!copy0.same_allocation(&map));
        copy0
            .set(
                Some(NodeId(2)),
                LongDistanceNeighborRequirements {
                    edge_count: 777,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(copy1.get(Some(NodeId(2))).edge_count, 777);
        assert_eq!(map.get(Some(NodeId(2))).edge_count, 259);
        let snapshot = owner.owner_topology_snapshot();
        map.set(
            Some(NodeId(2)),
            LongDistanceNeighborRequirements {
                edge_count: 999,
                ..Default::default()
            },
        )
        .unwrap();
        map.set(None, LongDistanceNeighborRequirements::default())
            .unwrap();
        let replacement = NodeRequirementsMap::allocated();
        replacement
            .set(
                None,
                LongDistanceNeighborRequirements {
                    edge_count: 1,
                    ..Default::default()
                },
            )
            .unwrap();
        owner
            .set_owner_node_requirement_map(NodeId(0), replacement.retained())
            .unwrap();
        owner.restore_owner_topology_snapshot(snapshot);
        assert!(
            owner
                .owner_node_requirement_map(NodeId(0))
                .unwrap()
                .same_allocation(&map)
        );
        assert!(
            owner
                .owner_node_requirement_map(NodeId(1))
                .unwrap()
                .same_allocation(&map)
        );
        assert_eq!(map.get(Some(NodeId(2))).max_width, 1 << 33);
        assert_eq!(map.get(Some(NodeId(2))).edge_count, 259);
        assert_eq!(map.raw_keys(), [Some(NodeId(2))]);
        assert_eq!(replacement.get(None).edge_count, 1);
    }
}
