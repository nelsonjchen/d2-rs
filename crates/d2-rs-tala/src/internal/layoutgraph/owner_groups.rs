// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Graph.Clusters/Sequences are shared nullable pointer maps (graph.go47,61,
//! 87–103,1074–1084). Group Nodes, Vessel, Graph and Container are live fields
//! (cluster.go17–29; sequence.go14–20). Cluster scalar policy and nullable
//! EdgeAbductions retain independent actual records alongside membership.

use super::NodeId;
use super::container_slices::SharedNodeSlice;
use super::shared_topology::{OwnerArenaId, OwnerGraphId};
use crate::internal::go_cell::GoCell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Address of a Graph.Clusters/Sequences field, separate from its map value.
/// A retained map header keeps its old allocation after field replacement.
#[derive(Clone, Debug)]
pub(super) struct SharedOwnerGroupMapField(Arc<GoCell<SharedOwnerGroupMap>>);
impl SharedOwnerGroupMapField {
    pub(super) fn new(map: SharedOwnerGroupMap) -> Self {
        Self(Arc::new(GoCell::new(map)))
    }
    pub(super) fn header(&self) -> SharedOwnerGroupMap {
        self.0.read().expect("graph group map field").clone()
    }
    pub(super) fn replace(&self, map: SharedOwnerGroupMap) {
        *self.0.write().expect("graph group map field") = map;
    }
    pub(super) fn same_field(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(super) fn is_nil(&self) -> bool {
        self.0.read().expect("graph group map field").is_nil()
    }
    pub(super) fn len(&self) -> usize {
        self.0.read().expect("graph group map field").len()
    }
    pub(super) fn get(&self, key: Option<NodeId>) -> Option<Option<OwnerGroupId>> {
        self.0.read().expect("graph group map field").get(key)
    }
    pub(super) fn entries(&self) -> Vec<(Option<NodeId>, Option<OwnerGroupId>)> {
        self.0.read().expect("graph group map field").entries()
    }
    pub(super) fn delete(&self, key: Option<NodeId>) {
        self.header().delete(key);
    }
    pub(super) fn insert(
        &self,
        key: Option<NodeId>,
        group: Option<OwnerGroupId>,
    ) -> Result<(), ()> {
        self.header().insert(key, group)
    }
    pub(super) fn snapshot(&self) -> SharedOwnerGroupMapSnapshot {
        self.header().snapshot()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub(super) enum OwnerGroupKind {
    Cluster,
    Sequence,
}

static NEXT_GROUP_ALLOCATION: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub(super) struct OwnerGroupId {
    arena: OwnerArenaId,
    index: usize,
    allocation: u64,
    kind: OwnerGroupKind,
}
impl OwnerGroupId {
    pub(super) fn new(arena: OwnerArenaId, index: usize, kind: OwnerGroupKind) -> Self {
        Self {
            arena,
            index,
            kind,
            allocation: NEXT_GROUP_ALLOCATION
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                    next.checked_add(1)
                })
                .expect("canonical group identity overflow"),
        }
    }
    pub(super) fn arena(self) -> OwnerArenaId {
        self.arena
    }
    pub(super) fn index(self) -> usize {
        self.index
    }
    pub(super) fn kind(self) -> OwnerGroupKind {
        self.kind
    }
}

#[derive(Clone, Debug)]
pub(super) struct OwnerGroupFields {
    pub(super) vessel: Option<NodeId>,
    pub(super) nodes: SharedNodeSlice,
    pub(super) graph: Option<OwnerGraphId>,
    pub(super) container: Option<NodeId>,
}
impl Default for OwnerGroupFields {
    fn default() -> Self {
        Self {
            vessel: None,
            nodes: SharedNodeSlice::nil(),
            graph: None,
            container: None,
        }
    }
}

/// Cluster scalar fields retain the source string values, including an empty
/// or unknown arrangement. Abduction headers retain actual nullable backing.
#[derive(Clone, Debug, Default)]
pub(super) struct OwnerClusterPolicy {
    pub(super) arrangement: String,
    pub(super) desired_arrangement: String,
    pub(super) padding: f64,
    pub(super) fixed_size: bool,
    pub(super) edge_abductions: Option<super::shared_topology::OwnerAbductionHeader>,
}

/// Fixed allocation independently retained by graph maps and node bindings.
#[derive(Debug, Default)]
pub(super) struct OwnerGroupRecord {
    fields: GoCell<OwnerGroupFields>,
    // Mirror of `fields.vessel` for lock-free reads: cluster lookups scan
    // every group's vessel. 0 is nil, otherwise NodeId + 1. Written only
    // together with `fields`, on the layout thread.
    vessel_slot: AtomicU64,
    policy: GoCell<OwnerClusterPolicy>,
    sequence_abductions: GoCell<Option<super::shared_topology::OwnerAbductionHeader>>,
}
impl OwnerGroupRecord {
    /// A callback-free field view. Drop before field replacement or owner hooks.
    pub(super) fn read_fields(&self) -> crate::internal::go_cell::GoCellRef<'_, OwnerGroupFields> {
        self.fields.read().expect("owner group fields")
    }

    pub(super) fn fields(&self) -> OwnerGroupFields {
        self.fields.read().expect("owner group fields").clone()
    }
    pub(super) fn vessel(&self) -> Option<NodeId> {
        match self.vessel_slot.load(Ordering::Relaxed) {
            0 => None,
            slot => Some(NodeId((slot - 1) as u32)),
        }
    }
    pub(super) fn set_fields(&self, fields: OwnerGroupFields) {
        let mut current = self.fields.write().expect("owner group fields");
        self.vessel_slot
            .store(vessel_slot(fields.vessel), Ordering::Relaxed);
        *current = fields;
    }
    /// Borrow policy only until the next callback or field mutation.
    pub(super) fn read_policy(
        &self,
    ) -> crate::internal::go_cell::GoCellRef<'_, OwnerClusterPolicy> {
        self.policy.read().expect("owner cluster policy")
    }
    pub(super) fn policy(&self) -> OwnerClusterPolicy {
        self.policy.read().expect("owner cluster policy").clone()
    }
    pub(super) fn set_policy(&self, policy: OwnerClusterPolicy) {
        *self.policy.write().expect("owner cluster policy") = policy;
    }

    pub(super) fn set_desired_arrangement(&self, desired: String) {
        self.policy
            .write()
            .expect("owner cluster policy")
            .desired_arrangement = desired;
    }

    pub(super) fn set_orientation_policy(
        &self,
        arrangement: String,
        desired: String,
        padding: f64,
    ) {
        let mut policy = self.policy.write().expect("group policy");
        policy.arrangement = arrangement;
        policy.desired_arrangement = desired;
        policy.padding = padding;
    }
    pub(super) fn sequence_abductions(
        &self,
    ) -> Option<super::shared_topology::OwnerAbductionHeader> {
        self.sequence_abductions
            .read()
            .expect("owner sequence abductions")
            .clone()
    }
    pub(super) fn set_sequence_abductions(
        &self,
        header: Option<super::shared_topology::OwnerAbductionHeader>,
    ) {
        *self
            .sequence_abductions
            .write()
            .expect("owner sequence abductions") = header;
    }
    pub(super) fn numerical_copy(&self, nodes: SharedNodeSlice) -> Self {
        let original = self.read_fields();
        let fields = OwnerGroupFields {
            vessel: original.vessel,
            nodes,
            graph: original.graph,
            container: original.container,
        };
        Self {
            vessel_slot: AtomicU64::new(vessel_slot(fields.vessel)),
            fields: GoCell::new(fields),
            policy: GoCell::new(self.policy()),
            sequence_abductions: GoCell::new(self.sequence_abductions()),
        }
    }
}

fn vessel_slot(vessel: Option<NodeId>) -> u64 {
    vessel.map_or(0, |node| u64::from(node.0) + 1)
}

type GroupEntries = BTreeMap<Option<NodeId>, Option<OwnerGroupId>>;
#[derive(Clone, Debug, Default)]
pub(super) struct SharedOwnerGroupMap {
    entries: Option<Arc<GoCell<GroupEntries>>>,
}
impl SharedOwnerGroupMap {
    pub(super) fn nil() -> Self {
        Self::default()
    }
    pub(super) fn allocated() -> Self {
        Self {
            entries: Some(Arc::new(GoCell::new(BTreeMap::new()))),
        }
    }
    pub(super) fn is_nil(&self) -> bool {
        self.entries.is_none()
    }
    pub(super) fn len(&self) -> usize {
        self.entries
            .as_ref()
            .map_or(0, |entries| entries.read().expect("group map").len())
    }
    pub(super) fn get(&self, key: Option<NodeId>) -> Option<Option<OwnerGroupId>> {
        self.entries
            .as_ref()
            .and_then(|entries| entries.read().expect("group map").get(&key).copied())
    }
    pub(super) fn insert(
        &self,
        key: Option<NodeId>,
        group: Option<OwnerGroupId>,
    ) -> Result<(), ()> {
        self.entries
            .as_ref()
            .ok_or(())?
            .write()
            .expect("group map")
            .insert(key, group);
        Ok(())
    }
    pub(super) fn delete(&self, key: Option<NodeId>) {
        if let Some(entries) = &self.entries {
            entries.write().expect("group map").remove(&key);
        }
    }
    pub(super) fn entries(&self) -> Vec<(Option<NodeId>, Option<OwnerGroupId>)> {
        self.entries.as_ref().map_or_else(Vec::new, |entries| {
            entries
                .read()
                .expect("group map")
                .iter()
                .map(|(&node, &group)| (node, group))
                .collect()
        })
    }
    /// Retain the selected map, loading each live value before the caller's
    /// Step. Deleted unvisited entries are skipped. This does not qualify Go's
    /// unspecified multi-key traversal order or insertion visibility.
    pub(super) fn live_values(&self) -> OwnerGroupMapCursor {
        OwnerGroupMapCursor {
            original: self.clone(),
            keys: self
                .entries()
                .into_iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>()
                .into_iter(),
        }
    }
    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        match (&self.entries, &other.entries) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub(super) fn fork_batch(headers: &[Self]) -> Vec<Self> {
        let mut maps = BTreeMap::new();
        headers
            .iter()
            .map(|header| Self {
                entries: header.entries.as_ref().map(|entries| {
                    Arc::clone(
                        maps.entry(Arc::as_ptr(entries) as usize)
                            .or_insert_with(|| {
                                Arc::new(GoCell::new(entries.read().expect("group map").clone()))
                            }),
                    )
                }),
            })
            .collect()
    }
    pub(super) fn snapshot(&self) -> SharedOwnerGroupMapSnapshot {
        SharedOwnerGroupMapSnapshot {
            original: self.clone(),
            entries: self.entries(),
        }
    }
}
pub(super) struct OwnerGroupMapCursor {
    original: SharedOwnerGroupMap,
    keys: std::vec::IntoIter<Option<NodeId>>,
}
impl Iterator for OwnerGroupMapCursor {
    type Item = Option<OwnerGroupId>;
    fn next(&mut self) -> Option<Self::Item> {
        for key in self.keys.by_ref() {
            if let Some(value) = self.original.get(key) {
                return Some(value);
            }
        }
        None
    }
}
#[derive(Debug)]
pub(super) struct SharedOwnerGroupMapSnapshot {
    original: SharedOwnerGroupMap,
    entries: Vec<(Option<NodeId>, Option<OwnerGroupId>)>,
}
impl SharedOwnerGroupMapSnapshot {
    pub(super) fn restore(self) -> SharedOwnerGroupMap {
        if let Some(entries) = &self.original.entries {
            let mut current = entries.write().expect("restore group map");
            current.clear();
            current.extend(self.entries);
        }
        self.original
    }
}

#[cfg(test)]
#[path = "owner_groups_tests.rs"]
mod tests;
