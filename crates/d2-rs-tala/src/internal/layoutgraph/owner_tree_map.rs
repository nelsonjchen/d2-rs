// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Actual Graph.NodeToTree/Graph.Trees headers and live NewTree allocations.
//!
//! D2 v0.9.0 graph.go57,87–103,1074–1083 and tree.go5–20 distinguish
//! a nil map, a present nil value, and a real Tree pointer. The constructor
//! only probes key presence. Tree fields and child headers retain their live
//! allocations; guarded traversal is supplied by placement/owner_median.rs.
//! Graph.Trees follows NewGraph/CopyEntitiesFrom and the exact capacity-window
//! restoration in transaction_snapshot.go16–68,254–268,338.

use super::shared_topology::OwnerArenaId;
use super::{EdgeId, NodeId};
use crate::internal::go_cell::GoCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

static NEXT_TREE_ALLOCATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub(super) struct OwnerTreeId {
    arena: OwnerArenaId,
    index: usize,
    allocation: u64,
}

impl OwnerTreeId {
    pub(super) fn new(arena: OwnerArenaId, index: usize) -> Self {
        Self {
            arena,
            index,
            allocation: NEXT_TREE_ALLOCATION
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                    next.checked_add(1)
                })
                .expect("canonical owner Tree allocation identity overflow"),
        }
    }

    pub(super) fn arena(self) -> OwnerArenaId {
        self.arena
    }

    pub(super) fn index(self) -> usize {
        self.index
    }
}

#[derive(Clone, Debug)]
pub(super) struct OwnerTreeFields {
    pub(super) node: Option<NodeId>,
    pub(super) parent: Option<OwnerTreeId>,
    pub(super) children: OwnerTreeSlice,
    pub(super) sentinel_edge: Option<EdgeId>,
    pub(super) orientation: i64,
}

/// The record's address stays fixed while its live fields change. No lock
/// remains borrowed across a work/context callback.
#[derive(Debug)]
pub(super) struct OwnerTreeRecord {
    fields: GoCell<OwnerTreeFields>,
}

impl OwnerTreeRecord {
    pub(super) fn new(node: Option<NodeId>) -> Self {
        Self {
            fields: GoCell::new(OwnerTreeFields {
                node,
                parent: None,
                // Source make([]*Tree, 0), distinct from a nil Children header.
                children: OwnerTreeSlice::allocated_empty(),
                sentinel_edge: None,
                // NewTree leaves Go's zero Orientation, geo.TopLeft.
                orientation: 0,
            }),
        }
    }

    /// A callback-free field view. Drop before field replacement or owner hooks.
    pub(super) fn read_fields(&self) -> crate::internal::go_cell::GoCellRef<'_, OwnerTreeFields> {
        self.fields.read().expect("owner Tree read")
    }

    pub(super) fn fields(&self) -> OwnerTreeFields {
        self.fields.read().expect("owner Tree read").clone()
    }

    pub(super) fn set_orientation(&self, orientation: i64) {
        // Geometry rollback changes this field alone; Node/Parent/Children stay live.
        self.fields
            .write()
            .expect("owner Tree orientation write")
            .orientation = orientation;
    }

    pub(super) fn set_fields(&self, fields: OwnerTreeFields) {
        *self.fields.write().expect("owner Tree write") = fields;
    }

    pub(super) fn numerical_copy_with_children(&self, children: OwnerTreeSlice) -> Self {
        let original = self.read_fields();
        let fields = OwnerTreeFields {
            node: original.node,
            parent: original.parent,
            children,
            sentinel_edge: original.sentinel_edge,
            orientation: original.orientation,
        };
        Self {
            fields: GoCell::new(fields),
        }
    }
}

type TreeBacking = Arc<GoCell<Box<[Option<OwnerTreeId>]>>>;

/// A nullable pointer slice with explicit length/capacity and shared backing.
/// A retained header reads later slot edits from its original allocation.
#[derive(Clone, Debug, Default)]
pub(super) struct OwnerTreeSlice {
    backing: Option<TreeBacking>,
    offset: usize,
    length: usize,
    capacity: usize,
}

impl OwnerTreeSlice {
    pub(super) fn nil() -> Self {
        Self::default()
    }

    pub(super) fn allocated_empty() -> Self {
        static ZERO: OnceLock<TreeBacking> = OnceLock::new();
        Self {
            backing: Some(Arc::clone(
                ZERO.get_or_init(|| Arc::new(GoCell::new(Box::new([None])))),
            )),
            ..Self::default()
        }
    }

    pub(super) fn from_slots(slots: &[Option<OwnerTreeId>], capacity: usize) -> Option<Self> {
        if slots.len() > capacity {
            return None;
        }
        if capacity == 0 {
            return Some(Self::allocated_empty());
        }
        let mut values = vec![None; capacity].into_boxed_slice();
        values[..slots.len()].copy_from_slice(slots);
        Some(Self {
            backing: Some(Arc::new(GoCell::new(values))),
            offset: 0,
            length: slots.len(),
            capacity,
        })
    }

    pub(super) fn is_nil(&self) -> bool {
        self.backing.is_none()
    }
    pub(super) fn len(&self) -> usize {
        self.length
    }
    pub(super) fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub(super) fn capacity(&self) -> usize {
        self.capacity
    }

    /// append to a heap-owned Tree.Children or Graph.Trees value. These field
    /// appends use growslice, unlike the codec's buffered local node slices.
    pub(super) fn append_tree(&self, tree: OwnerTreeId) -> Self {
        if self.length < self.capacity {
            assert!(self.write_capacity_slot(self.length, Some(tree)));
            return self
                .reslice(0, self.length + 1)
                .expect("tree append header");
        }
        let capacity =
            super::container_slices::go127_pointer_growth_capacity(self.capacity, self.length + 1)
                .expect("tree pointer capacity");
        let mut slots = (0..self.length)
            .map(|i| self.get(i).unwrap())
            .collect::<Vec<_>>();
        slots.push(Some(tree));
        Self::from_slots(&slots, capacity).expect("tree append allocation")
    }

    pub(super) fn get(&self, index: usize) -> Option<Option<OwnerTreeId>> {
        (index < self.length).then(|| self.read_capacity_slot(index).expect("retained Tree slot"))
    }

    pub(super) fn read_capacity_slot(&self, index: usize) -> Option<Option<OwnerTreeId>> {
        if index >= self.capacity {
            return None;
        }
        self.backing
            .as_ref()
            .map(|backing| backing.read().expect("Tree slice read")[self.offset + index])
    }

    pub(super) fn write_capacity_slot(&self, index: usize, tree: Option<OwnerTreeId>) -> bool {
        if index >= self.capacity {
            return false;
        }
        let Some(backing) = &self.backing else {
            return false;
        };
        backing.write().expect("Tree slice write")[self.offset + index] = tree;
        true
    }

    pub(super) fn reslice(&self, low: usize, high: usize) -> Option<Self> {
        if low > high || high > self.capacity {
            return None;
        }
        Some(Self {
            backing: self.backing.clone(),
            offset: self.offset + low,
            length: high - low,
            capacity: self.capacity - low,
        })
    }

    pub(super) fn reslice_with_capacity(
        &self,
        low: usize,
        high: usize,
        max: usize,
    ) -> Option<Self> {
        if low > high || high > max || max > self.capacity {
            return None;
        }
        Some(Self {
            backing: self.backing.clone(),
            offset: self.offset + low,
            length: high - low,
            capacity: max - low,
        })
    }

    pub(super) fn data_address(&self) -> Option<usize> {
        self.backing.as_ref().map(|backing| {
            backing.read().expect("Tree slice address").as_ptr() as usize
                + self.offset * std::mem::size_of::<Option<OwnerTreeId>>()
        })
    }

    pub(super) fn slot_address(&self, index: usize) -> Option<usize> {
        if index >= self.capacity {
            return None;
        }
        self.data_address()
            .map(|data| data + index * std::mem::size_of::<Option<OwnerTreeId>>())
    }

    fn snapshot(&self) -> OwnerTreeSliceSnapshot {
        OwnerTreeSliceSnapshot {
            original: self.clone(),
            slots: (0..self.capacity)
                .map(|index| self.read_capacity_slot(index).expect("Tree capacity slot"))
                .collect(),
        }
    }

    /// Numerical forks detach storage in one batch, retaining alias windows.
    pub(super) fn fork_batch(headers: &[Self]) -> Vec<Self> {
        let mut backings: BTreeMap<usize, TreeBacking> = BTreeMap::new();
        headers
            .iter()
            .map(|header| {
                let backing = header.backing.as_ref().map(|backing| {
                    let id = Arc::as_ptr(backing) as usize;
                    Arc::clone(backings.entry(id).or_insert_with(|| {
                        Arc::new(GoCell::new(
                            backing.read().expect("numerical Tree slice").clone(),
                        ))
                    }))
                });
                Self {
                    backing,
                    offset: header.offset,
                    length: header.length,
                    capacity: header.capacity,
                }
            })
            .collect()
    }
}

#[derive(Debug)]
struct OwnerTreeSliceSnapshot {
    original: OwnerTreeSlice,
    slots: Vec<Option<OwnerTreeId>>,
}

impl OwnerTreeSliceSnapshot {
    fn restore(self) -> OwnerTreeSlice {
        for (index, tree) in self.slots.into_iter().enumerate() {
            assert!(
                self.original.write_capacity_slot(index, tree),
                "captured Tree capacity window"
            );
        }
        self.original
    }
}

type TreeRootEntries = BTreeMap<Option<NodeId>, OwnerTreeSlice>;

/// Source Graph.Trees is a map of slice headers, independent of NodeToTree.
/// A get copies the header; slot edits remain visible through that allocation.
#[derive(Clone, Debug, Default)]
pub(super) struct SharedOwnerTreeRootsMap {
    entries: Option<Arc<GoCell<TreeRootEntries>>>,
}

impl SharedOwnerTreeRootsMap {
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
            .map_or(0, |entries| entries.read().expect("Tree roots map").len())
    }

    pub(super) fn get(&self, node: Option<NodeId>) -> Option<OwnerTreeSlice> {
        self.entries
            .as_ref()
            .and_then(|entries| entries.read().expect("Tree roots map").get(&node).cloned())
    }

    pub(super) fn insert(&self, node: Option<NodeId>, roots: OwnerTreeSlice) -> Result<(), ()> {
        self.entries
            .as_ref()
            .ok_or(())?
            .write()
            .expect("Tree roots map")
            .insert(node, roots);
        Ok(())
    }

    pub(super) fn delete(&self, node: Option<NodeId>) {
        if let Some(entries) = &self.entries {
            entries.write().expect("Tree roots map").remove(&node);
        }
    }

    pub(super) fn entries(&self) -> Vec<(Option<NodeId>, OwnerTreeSlice)> {
        self.entries.as_ref().map_or_else(Vec::new, |entries| {
            entries
                .read()
                .expect("Tree roots map")
                .iter()
                .map(|(&node, roots)| (node, roots.clone()))
                .collect()
        })
    }

    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        match (&self.entries, &other.entries) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Detach all maps once per map allocation, and all slice backings once
    /// per backing allocation. Extra fields (Tree.Children) join this same
    /// batch so a root header overlapping a child header retains its aliases.
    pub(super) fn fork_with_slices(
        headers: &[Self],
        extra: &[OwnerTreeSlice],
    ) -> (Vec<Self>, Vec<OwnerTreeSlice>) {
        let mut unique = BTreeMap::new();
        for header in headers {
            if let Some(entries) = &header.entries {
                unique
                    .entry(Arc::as_ptr(entries) as usize)
                    .or_insert_with(|| header.entries());
            }
        }
        let mut slices: Vec<_> = unique
            .values()
            .flat_map(|entries| entries.iter().map(|(_, roots)| roots.clone()))
            .collect();
        slices.extend_from_slice(extra);
        let mut detached = OwnerTreeSlice::fork_batch(&slices).into_iter();
        let maps: BTreeMap<_, _> = unique
            .into_iter()
            .map(|(id, entries)| {
                let copied: TreeRootEntries = entries
                    .into_iter()
                    .map(|(key, _)| (key, detached.next().expect("detached root header")))
                    .collect();
                (id, Arc::new(GoCell::new(copied)))
            })
            .collect();
        let result = headers
            .iter()
            .map(|header| Self {
                entries: header
                    .entries
                    .as_ref()
                    .map(|entries| Arc::clone(&maps[&(Arc::as_ptr(entries) as usize)])),
            })
            .collect();
        (result, detached.collect())
    }

    pub(super) fn snapshot(&self) -> SharedOwnerTreeRootsMapSnapshot {
        SharedOwnerTreeRootsMapSnapshot {
            original: self.clone(),
            entries: self
                .entries()
                .into_iter()
                .map(|(node, roots)| (node, roots.snapshot()))
                .collect(),
        }
    }
}

#[derive(Debug)]
pub(super) struct SharedOwnerTreeRootsMapSnapshot {
    original: SharedOwnerTreeRootsMap,
    entries: Vec<(Option<NodeId>, OwnerTreeSliceSnapshot)>,
}

impl SharedOwnerTreeRootsMapSnapshot {
    pub(super) fn restore(self) -> SharedOwnerTreeRootsMap {
        if let Some(entries) = &self.original.entries {
            let mut current = entries.write().expect("restore Tree roots map");
            current.clear();
            for (node, roots) in self.entries {
                current.insert(node, roots.restore());
            }
        }
        self.original
    }
}

type TreeEntries = BTreeMap<Option<NodeId>, Option<OwnerTreeId>>;

/// Cloning a header aliases its map. Numerical forks explicitly detach maps
/// in one batch so equivalent headers retain equivalence within the fork.
#[derive(Clone, Debug, Default)]
pub(super) struct SharedOwnerTreeMap {
    entries: Option<Arc<GoCell<TreeEntries>>>,
}

impl SharedOwnerTreeMap {
    pub(super) fn live_keys(&self) -> impl Iterator<Item = Option<NodeId>> + use<> {
        let keys = self.entries.as_ref().map_or_else(Vec::new, |entries| {
            entries
                .read()
                .expect("owner tree map read lock")
                .keys()
                .copied()
                .collect()
        });
        let original = self.clone();
        keys.into_iter()
            .filter(move |key| original.get(*key).is_some())
    }
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
        self.entries.as_ref().map_or(0, |entries| {
            entries.read().expect("owner tree map read lock").len()
        })
    }

    pub(super) fn get(&self, node: Option<NodeId>) -> Option<Option<OwnerTreeId>> {
        self.entries.as_ref().and_then(|entries| {
            entries
                .read()
                .expect("owner tree map read lock")
                .get(&node)
                .copied()
        })
    }

    /// The topology layer validates identities before this actual map write.
    /// A nil map does not silently allocate on assignment.
    pub(super) fn insert(&self, node: Option<NodeId>, tree: Option<OwnerTreeId>) -> Result<(), ()> {
        let Some(entries) = &self.entries else {
            return Err(());
        };
        entries
            .write()
            .expect("owner tree map write lock")
            .insert(node, tree);
        Ok(())
    }

    pub(super) fn delete(&self, node: Option<NodeId>) {
        if let Some(entries) = &self.entries {
            entries
                .write()
                .expect("owner tree map write lock")
                .remove(&node);
        }
    }

    pub(super) fn validate_entries<E>(
        &self,
        mut validate: impl FnMut(Option<NodeId>, Option<OwnerTreeId>) -> Result<(), E>,
    ) -> Result<(), E> {
        if let Some(entries) = &self.entries {
            for (&node, &tree) in entries.read().expect("owner tree map read lock").iter() {
                validate(node, tree)?;
            }
        }
        Ok(())
    }

    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        match (&self.entries, &other.entries) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    pub(super) fn fork_storage_batch(headers: &[Self]) -> Vec<Self> {
        let mut detached = BTreeMap::<usize, Arc<GoCell<TreeEntries>>>::new();
        headers
            .iter()
            .map(|header| Self {
                entries: header.entries.as_ref().map(|entries| {
                    let allocation = Arc::as_ptr(entries) as usize;
                    Arc::clone(detached.entry(allocation).or_insert_with(|| {
                        Arc::new(GoCell::new(
                            entries.read().expect("owner tree map read lock").clone(),
                        ))
                    }))
                }),
            })
            .collect()
    }

    pub(super) fn snapshot(&self) -> SharedOwnerTreeMapSnapshot {
        SharedOwnerTreeMapSnapshot {
            original: self.clone(),
            saved: self.entries.as_ref().and_then(|entries| {
                let current = entries.read().expect("owner tree map read lock");
                // Source cloneMapContents returns nil for an empty map.
                (!current.is_empty()).then(|| current.clone())
            }),
        }
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn map_address(&self) -> Option<usize> {
        self.entries
            .as_ref()
            .map(|entries| Arc::as_ptr(entries) as usize)
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn observed_entries(&self) -> Vec<(Option<NodeId>, Option<OwnerTreeId>)> {
        self.entries.as_ref().map_or_else(Vec::new, |entries| {
            entries
                .read()
                .expect("owner tree map read lock")
                .iter()
                .map(|(&node, &tree)| (node, tree))
                .collect()
        })
    }
}

#[derive(Debug)]
pub(super) struct SharedOwnerTreeMapSnapshot {
    original: SharedOwnerTreeMap,
    saved: Option<TreeEntries>,
}

impl SharedOwnerTreeMapSnapshot {
    /// Source restoreMap clears and copies the original allocation, then
    /// restores that header. Retained replacement allocations are untouched.
    pub(super) fn restore(self) -> SharedOwnerTreeMap {
        if let Some(entries) = &self.original.entries {
            let mut current = entries.write().expect("owner tree map write lock");
            current.clear();
            if let Some(saved) = self.saved {
                current.extend(saved);
            }
        }
        self.original
    }
}

#[cfg(test)]
#[path = "owner_tree_roots_tests.rs"]
mod roots_tests;
