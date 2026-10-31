// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Actual nullable Node.Nears map storage (node.go610,715–728,759–771).
//!
//! A raw range retains its original map and loads a key before its caller's
//! work/poll. Future deletions are skipped; insertion may be observed and a
//! key is never returned twice. These are the pinned Go1.27 maps/map.go132–179
//! contracts. This slot cursor does not reproduce Go's pointer hash, table
//! growth, or randomized directory/slot offsets, so multi-key range order and
//! cancellation prefixes remain unqualified. Sorted source helpers must sort
//! a materialized key list at their own boundary, not use this cursor order.

use super::{ArenaGraph, NodeId, OwnerTopologyError};
use std::collections::BTreeSet;
use crate::internal::go_cell::{GoCell};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
struct NearSlots {
    // Outer None is a deleted slot; Some(None) is the genuine nil map key.
    slots: Vec<Option<Option<NodeId>>>,
    len: usize,
}

#[derive(Debug, Default)]
pub(super) struct NodeNearMap {
    storage: Option<Arc<GoCell<NearSlots>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NilNearMapAssignment;

// ArenaNode/ArenaGraph Clone is a numerical copy, rather than a source map
// header assignment. Existing numerical candidates must own separate maps.
// Actual source assignments and ranges use retained(), which shares storage.
impl Clone for NodeNearMap {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.as_ref().map(|storage| {
                Arc::new(GoCell::new(storage.read().expect("near map read").clone()))
            }),
        }
    }

    // Uniquely owned storage is refilled in place; no retained() alias can
    // tell that from a fresh numerical copy.
    fn clone_from(&mut self, source: &Self) {
        if let (Some(target), Some(from)) = (self.storage.as_mut(), source.storage.as_ref())
            && let Some(target) = Arc::get_mut(target)
        {
            let NearSlots { slots, len } = target.get_mut().expect("near map write");
            let from = from.read().expect("near map read");
            slots.clone_from(&from.slots);
            *len = from.len;
            return;
        }
        *self = source.clone();
    }
}

impl NodeNearMap {
    pub(super) fn nil() -> Self {
        Self::default()
    }

    pub(super) fn allocated() -> Self {
        Self {
            storage: Some(Arc::new(GoCell::new(NearSlots::default()))),
        }
    }

    pub(super) fn from_raw_keys(keys: impl IntoIterator<Item = Option<NodeId>>) -> Self {
        let result = Self::allocated();
        for key in keys {
            result.insert(key).expect("allocated near map");
        }
        result
    }

    /// Copy the source map header, keeping the actual allocation alive.
    pub(super) fn retained(&self) -> Self {
        Self {
            storage: self.storage.as_ref().map(Arc::clone),
        }
    }

    pub(super) fn is_nil(&self) -> bool {
        self.storage.is_none()
    }

    pub(super) fn len(&self) -> usize {
        self.storage
            .as_ref()
            .map_or(0, |storage| storage.read().expect("near map read").len)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(super) fn contains_raw(&self, key: Option<NodeId>) -> bool {
        self.storage.as_ref().is_some_and(|storage| {
            storage
                .read()
                .expect("near map read")
                .slots
                .contains(&Some(key))
        })
    }

    pub(super) fn contains(&self, node: &NodeId) -> bool {
        self.contains_raw(Some(*node))
    }

    /// Raw map assignment permits nil and self keys. Node.AddNear performs
    /// its separate nil/self precheck before allocating either node's map.
    pub(super) fn insert(&self, key: Option<NodeId>) -> Result<bool, NilNearMapAssignment> {
        let storage = self.storage.as_ref().ok_or(NilNearMapAssignment)?;
        let mut slots = storage.write().expect("near map write");
        if slots.slots.contains(&Some(key)) {
            return Ok(false);
        }
        if let Some(empty) = slots.slots.iter_mut().find(|slot| slot.is_none()) {
            *empty = Some(key);
        } else {
            slots.slots.push(Some(key));
        }
        slots.len += 1;
        Ok(true)
    }

    pub(super) fn delete(&self, key: Option<NodeId>) -> bool {
        let Some(storage) = &self.storage else {
            return false;
        };
        let mut slots = storage.write().expect("near map write");
        let Some(slot) = slots.slots.iter_mut().find(|slot| **slot == Some(key)) else {
            return false;
        };
        *slot = None;
        slots.len -= 1;
        true
    }

    pub(super) fn clear(&self) {
        if let Some(storage) = &self.storage {
            let mut slots = storage.write().expect("near map write");
            slots.slots.fill(None);
            slots.len = 0;
        }
    }

    /// Ordinary AddNear producers allocate a nil map before insertion.
    /// This helper alone does not implement AddNear's reciprocal/self policy.
    pub(super) fn push(&mut self, node: NodeId) {
        if self.is_nil() {
            *self = Self::allocated();
        }
        self.insert(Some(node)).expect("allocated near map");
    }

    /// Materialization for existing ordinary/sorted consumers. A nil key is
    /// an unsupported operand in those paths, never a silently dropped key.
    pub(super) fn keys(&self) -> Vec<NodeId> {
        self.raw_keys()
            .into_iter()
            .map(|key| key.expect("ordinary Near traversal requires a nonnil node key"))
            .collect()
    }

    pub(super) fn raw_keys(&self) -> Vec<Option<NodeId>> {
        self.storage.as_ref().map_or_else(Vec::new, |storage| {
            storage
                .read()
                .expect("near map read")
                .slots
                .iter()
                .filter_map(|slot| *slot)
                .collect()
        })
    }

    fn cursor_end(&self) -> usize {
        self.storage.as_ref().map_or(0, |storage| {
            let slots = storage.read().expect("near map read");
            if slots.len == 0 { 0 } else { slots.slots.len() }
        })
    }

    pub(super) fn live_cursor(&self) -> NearCursor {
        NearCursor { original: self.retained(), next_slot: 0, end: self.cursor_end(), seen: BTreeSet::new() }
    }

    /// Iterate a separately retained map without retaining its allocation twice.
    /// Each next() drops its read guard before the consumer's owner callback.
    pub(super) fn borrowed_cursor(&self) -> impl Iterator<Item = Option<NodeId>> + '_ {
        let end = self.cursor_end();
        let mut next_slot = 0;
        let mut seen = BTreeSet::new();
        std::iter::from_fn(move || next_key(self, &mut next_slot, end, &mut seen))
    }

    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        match (&self.storage, &other.storage) {
            (None, None) => true,
            (Some(first), Some(second)) => Arc::ptr_eq(first, second),
            _ => false,
        }
    }

    pub(super) fn snapshot(&self) -> NodeNearMapSnapshot {
        NodeNearMapSnapshot {
            original: self.retained(),
            keys: self.raw_keys(),
        }
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn reference_address(&self) -> Option<usize> {
        self.storage
            .as_ref()
            .map(|storage| Arc::as_ptr(storage) as usize)
    }
}

impl FromIterator<NodeId> for NodeNearMap {
    fn from_iter<T: IntoIterator<Item = NodeId>>(iter: T) -> Self {
        Self::from_raw_keys(iter.into_iter().map(Some))
    }
}

pub(super) struct NearCursor {
    original: NodeNearMap,
    next_slot: usize,
    end: usize,
    // A membership set only; traversal never follows BTree key order.
    seen: BTreeSet<Option<NodeId>>,
}

impl Iterator for NearCursor {
    type Item = Option<NodeId>;

    fn next(&mut self) -> Option<Self::Item> {
        next_key(&self.original, &mut self.next_slot, self.end, &mut self.seen)
    }
}

fn next_key(map: &NodeNearMap, next_slot: &mut usize, end: usize, seen: &mut BTreeSet<Option<NodeId>>) -> Option<Option<NodeId>> {
    let storage = map.storage.as_ref()?;
    while *next_slot < end {
        let index = *next_slot;
        *next_slot += 1;
        let key = storage.read().expect("near map read").slots.get(index).copied().flatten();
        if let Some(key) = key {
            if seen.insert(key) { return Some(key); }
        }
    }
    None
}

#[derive(Debug)]
pub(super) struct NodeNearMapSnapshot {
    original: NodeNearMap,
    keys: Vec<Option<NodeId>>,
}

impl NodeNearMapSnapshot {
    /// Repair retained aliases in the original allocation, then return that
    /// same header. A replacement map remains independent and unchanged.
    pub(super) fn restore(self) -> NodeNearMap {
        self.original.clear();
        for key in self.keys {
            self.original
                .insert(key)
                .expect("captured allocated near map");
        }
        self.original
    }
}

impl ArenaGraph {
    /// Actual Node.AddNear: nil/self operands return before allocation, then
    /// both genuine node maps are allocated and receive reciprocal entries.
    pub(super) fn add_owner_near(
        &mut self,
        first: Option<NodeId>,
        second: Option<NodeId>,
    ) -> Result<(), OwnerTopologyError> {
        let (Some(first), Some(second)) = (first, second) else {
            return Ok(());
        };
        if first == second {
            return Ok(());
        }
        for node in [first, second] {
            self.topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?;
        }
        self.nodes[first.0 as usize].nears.push(second);
        self.nodes[second.0 as usize].nears.push(first);
        Ok(())
    }

    pub(super) fn owner_node_near_map(
        &self,
        node: NodeId,
    ) -> Result<NodeNearMap, OwnerTopologyError> {
        Ok(self
            .topology_node(node)
            .ok_or(OwnerTopologyError::UnknownNode(node))?
            .nears
            .retained())
    }

    /// Raw field assignment permits nil/self keys without AddNear filtering.
    /// Validate actual node keys before writing this one node's map header.
    pub(super) fn set_owner_node_near_map(
        &mut self,
        node: NodeId,
        map: NodeNearMap,
    ) -> Result<(), OwnerTopologyError> {
        self.topology_node(node)
            .ok_or(OwnerTopologyError::UnknownNode(node))?;
        for key in map.raw_keys().into_iter().flatten() {
            self.topology_node(key)
                .ok_or(OwnerTopologyError::UnknownNode(key))?;
        }
        self.nodes[node.0 as usize].nears = map;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_add_near_filters_nil_self_allocates_both_maps_and_raw_field_keeps_nil() {
        use crate::{Graph, Node, Size};
        let mut input = Graph::default();
        let first = input.add_node(Node::new(
            "C",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let second = input.add_node(Node::new(
            "D",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.node_mut(first).unwrap().near = Some(first);
        let mut owner = ArenaGraph::from_input(&input);
        assert!(owner.nodes[first.0 as usize].nears.is_empty());
        owner
            .set_owner_node_near_map(first, NodeNearMap::nil())
            .unwrap();
        owner
            .set_owner_node_near_map(second, NodeNearMap::nil())
            .unwrap();
        owner.add_owner_near(None, Some(first)).unwrap();
        owner.add_owner_near(Some(first), Some(first)).unwrap();
        assert!(owner.nodes[first.0 as usize].nears.is_nil());
        assert!(owner.nodes[second.0 as usize].nears.is_nil());
        owner.add_owner_near(Some(first), Some(second)).unwrap();
        owner.add_owner_near(Some(second), Some(first)).unwrap();
        assert_eq!(owner.nodes[first.0 as usize].nears.keys(), [second]);
        assert_eq!(owner.nodes[second.0 as usize].nears.keys(), [first]);
        let held = owner.owner_node_near_map(first).unwrap();
        owner
            .set_owner_node_near_map(first, NodeNearMap::from_raw_keys([None, Some(first)]))
            .unwrap();
        assert_eq!(
            owner.nodes[first.0 as usize].nears.raw_keys(),
            [None, Some(first)]
        );
        assert_eq!(held.keys(), [second]);
        assert_eq!(
            owner.set_owner_node_near_map(first, NodeNearMap::from_raw_keys([Some(NodeId(99))])),
            Err(OwnerTopologyError::UnknownNode(NodeId(99)))
        );
        assert_eq!(
            owner.nodes[first.0 as usize].nears.raw_keys(),
            [None, Some(first)]
        );
    }

    #[test]
    fn near_nil_empty_raw_nil_self_and_set_assignment_are_distinct() {
        let nil = NodeNearMap::nil();
        let empty = NodeNearMap::allocated();
        assert!(nil.is_nil());
        assert!(!empty.is_nil());
        assert!(nil.is_empty() && empty.is_empty());
        assert_eq!(nil.insert(None), Err(NilNearMapAssignment));
        assert!(!nil.delete(None));
        nil.clear();
        assert!(nil.is_nil());
        assert_eq!(empty.insert(None), Ok(true));
        assert_eq!(empty.insert(Some(NodeId(0))), Ok(true));
        assert_eq!(empty.insert(None), Ok(false));
        assert_eq!(empty.len(), 2);
        assert_eq!(
            empty.live_cursor().collect::<Vec<_>>(),
            [None, Some(NodeId(0))]
        );
        assert!(empty.delete(None));
        assert_eq!(empty.keys(), [NodeId(0)]);
    }

    #[test]
    fn near_cursor_keeps_loaded_key_original_header_and_live_future_deletions() {
        let mut current =
            NodeNearMap::from_raw_keys([Some(NodeId(0)), Some(NodeId(1)), Some(NodeId(2)), None]);
        let original = current.retained();
        let mut cursor = current.live_cursor();
        let loaded = cursor.next().unwrap();
        assert_eq!(loaded, Some(NodeId(0)));
        original.delete(loaded);
        original.delete(Some(NodeId(1)));
        current = NodeNearMap::from_raw_keys([Some(NodeId(9))]);
        assert_eq!(loaded, Some(NodeId(0)));
        assert_eq!(cursor.collect::<Vec<_>>(), [Some(NodeId(2)), None]);
        assert_eq!(current.keys(), [NodeId(9)]);
        assert_eq!(original.raw_keys(), [Some(NodeId(2)), None]);
        assert!(!current.same_allocation(&original));
    }

    #[test]
    fn near_cursor_can_observe_future_slot_insertion_without_repeating_a_key() {
        let map = NodeNearMap::from_raw_keys([Some(NodeId(0)), Some(NodeId(1)), Some(NodeId(2))]);
        let mut cursor = map.live_cursor();
        assert_eq!(cursor.next(), Some(Some(NodeId(0))));
        map.delete(Some(NodeId(1)));
        map.insert(Some(NodeId(3))).unwrap();
        map.delete(Some(NodeId(0)));
        map.insert(Some(NodeId(0))).unwrap();
        assert_eq!(
            cursor.collect::<Vec<_>>(),
            [Some(NodeId(3)), Some(NodeId(2))]
        );
        let empty = NodeNearMap::allocated();
        let mut empty_range = empty.live_cursor();
        empty.insert(None).unwrap();
        assert_eq!(empty_range.next(), None);
    }

    #[test]
    fn near_numerical_clone_detaches_while_source_header_retains_actual_alias() {
        let map = NodeNearMap::from_raw_keys([None, Some(NodeId(0))]);
        let retained = map.retained();
        let copied = map.clone();
        retained.delete(None);
        retained.insert(Some(NodeId(1))).unwrap();
        assert!(retained.same_allocation(&map));
        assert!(!copied.same_allocation(&map));
        assert_eq!(map.keys(), [NodeId(1), NodeId(0)]);
        assert_eq!(copied.raw_keys(), [None, Some(NodeId(0))]);
        assert!(NodeNearMap::nil().clone().is_nil());
    }

    #[test]
    fn near_snapshot_restores_original_alias_and_nil_without_changing_replacement() {
        let mut field = NodeNearMap::from_raw_keys([None, Some(NodeId(0))]);
        let retained = field.retained();
        let snapshot = field.snapshot();
        field.delete(None);
        field.insert(Some(NodeId(1))).unwrap();
        let replacement = NodeNearMap::from_raw_keys([Some(NodeId(9))]);
        field = replacement.retained();
        assert!(field.same_allocation(&replacement));
        field = snapshot.restore();
        assert!(field.same_allocation(&retained));
        assert_eq!(retained.raw_keys(), [None, Some(NodeId(0))]);
        assert_eq!(replacement.keys(), [NodeId(9)]);
        assert!(NodeNearMap::nil().snapshot().restore().is_nil());
    }
}
