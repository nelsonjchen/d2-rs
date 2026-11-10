// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// The growth policy and size classes below are derived from Go 1.27.0
// runtime/slice.go, runtime/msize.go and internal/runtime/gc/{malloc,sizeclasses}.go.
// Copyright 2009, 2025 The Go Authors. All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
// * Redistributions of source code must retain the above copyright notice,
//   this list of conditions and the following disclaimer.
// * Redistributions in binary form must reproduce the above copyright notice,
//   this list of conditions and the following disclaimer in the documentation
//   and/or other materials provided with the distribution.
// * Neither the name of Google LLC nor the names of its contributors may be
//   used to endorse or promote products derived from this software without
//   specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.

//! Actual shared container maps and source-shaped slice headers/backings.
//!
//! D2 v0.9.0 graph.go241–259,763–783,1074–1084 shares these allocations;
//! transaction_snapshot.go16–68 restores the original map/header and every
//! slot in each header's capacity window. Node/edge/record entity and geometry
//! sharing are separate contracts, not supplied by these slice primitives.

use super::NodeId;
use crate::internal::go_cell::GoCell;
use d2_rs_core::stable_sort::StableSort;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ContainerSliceError {
    Bounds,
    LengthOverflow,
    Allocation(String),
    NilMapAssignment,
}

impl fmt::Display for ContainerSliceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "shared container slice: {self:?}")
    }
}
impl std::error::Error for ContainerSliceError {}

struct NodeSliceBacking {
    // Every slot is initialized; no read ever enters Vec's uninitialized
    // spare capacity. The fixed Box allocation never grows or moves.
    slots: NodeSlots,
}

/// Go *Node slots. Writers run only on the layout thread and rayon route
/// workers only read, so relaxed loads and stores (plain ldr/str on arm64)
/// replace a lock round trip per slot. A slot holds `NodeId.0`, or
/// `NIL_SLOT` for a nil pointer; `NodeId::ROOT` (u32::MAX) stays distinct.
struct NodeSlots(Box<[AtomicU64]>);
const NIL_SLOT: u64 = u64::MAX;

/// A reference into a captured header's fixed backing, not a captured value.
/// Load only after the caller's item poll so aliases can still change the slot.
pub(super) struct NodeSlotReader<'a>(&'a AtomicU64);

impl NodeSlotReader<'_> {
    pub(super) fn get(&self) -> Option<NodeId> {
        match self.0.load(Relaxed) {
            NIL_SLOT => None,
            id => Some(NodeId(id as u32)),
        }
    }
}

/// The header window is validated once; iterating it borrows slot addresses.
#[derive(Clone, Copy, Default)]
pub(super) struct BorrowedNodeSlots<'a>(&'a [AtomicU64]);

impl<'a> BorrowedNodeSlots<'a> {
    pub(super) fn readers(self) -> impl ExactSizeIterator<Item = NodeSlotReader<'a>> {
        self.0.iter().map(NodeSlotReader)
    }
}

impl NodeSlots {
    fn new(len: usize) -> Self {
        Self((0..len).map(|_| AtomicU64::new(NIL_SLOT)).collect())
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn load(&self, index: usize) -> Option<NodeId> {
        match self.0[index].load(Relaxed) {
            NIL_SLOT => None,
            id => Some(NodeId(id as u32)),
        }
    }
    fn store(&self, index: usize, value: Option<NodeId>) {
        self.0[index].store(value.map_or(NIL_SLOT, |id| u64::from(id.0)), Relaxed);
    }
    fn copy_out(&self, range: std::ops::Range<usize>) -> Vec<Option<NodeId>> {
        range.map(|index| self.load(index)).collect()
    }
    fn copy_in(&self, start: usize, values: &[Option<NodeId>]) {
        for (offset, &value) in values.iter().enumerate() {
            self.store(start + offset, value);
        }
    }
    /// `copy_in(0, &source.copy_out(0..source.len()))` without the
    /// intermediate image: the same slot reads and writes, in order.
    fn copy_from(&self, source: &Self) {
        for index in 0..source.len() {
            self.store(index, source.load(index));
        }
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    fn as_ptr(&self) -> *const AtomicU64 {
        self.0.as_ptr()
    }
}

fn allocated_zero_backing() -> Arc<NodeSliceBacking> {
    // Go's nonnil zero-cap header points at zerobase. Retain an actual,
    // initialized sentinel allocation rather than inventing an address/ID.
    // Its one physical sentinel is outside every zero-cap header window.
    static ZERO: OnceLock<Arc<NodeSliceBacking>> = OnceLock::new();
    Arc::clone(ZERO.get_or_init(|| {
        Arc::new(NodeSliceBacking {
            slots: NodeSlots::new(1),
        })
    }))
}

fn allocate_backing(capacity: usize) -> Result<Arc<NodeSliceBacking>, ContainerSliceError> {
    if capacity == 0 {
        return Ok(allocated_zero_backing());
    }
    check_pointer_capacity(capacity)?;
    let mut slots = Vec::new();
    slots
        .try_reserve_exact(capacity)
        .map_err(|error| ContainerSliceError::Allocation(error.to_string()))?;
    slots.extend((0..capacity).map(|_| AtomicU64::new(NIL_SLOT)));
    Ok(Arc::new(NodeSliceBacking {
        slots: NodeSlots(slots.into_boxed_slice()),
    }))
}

/// Clone copies a header and aliases its actual backing allocation. It never
/// clones the backing values. Default is a genuine nil slice.
#[derive(Clone, Default)]
pub(super) struct SharedNodeSlice {
    backing: Option<Arc<NodeSliceBacking>>,
    start: usize,
    len: usize,
    cap: usize,
}

impl fmt::Debug for SharedNodeSlice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedNodeSlice")
            .field("nil", &self.is_nil())
            .field("start", &self.start)
            .field("len", &self.len)
            .field("cap", &self.cap)
            .finish()
    }
}

impl SharedNodeSlice {
    pub(super) fn nil() -> Self {
        Self::default()
    }

    pub(super) fn allocated_empty() -> Self {
        Self {
            backing: Some(allocated_zero_backing()),
            ..Self::default()
        }
    }

    /// Source make([]*Node, len, cap), whose logical cap is exact even when
    /// the Go allocator reserves a larger physical size class.
    pub(super) fn with_len_cap(len: usize, cap: usize) -> Result<Self, ContainerSliceError> {
        if len > cap {
            return Err(ContainerSliceError::Bounds);
        }
        Ok(Self {
            backing: Some(allocate_backing(cap)?),
            start: 0,
            len,
            cap,
        })
    }

    pub(super) fn from_slots(slots: &[Option<NodeId>]) -> Result<Self, ContainerSliceError> {
        let result = Self::with_len_cap(slots.len(), slots.len())?;
        if !slots.is_empty() {
            result
                .backing
                .as_ref()
                .expect("allocated slice")
                .slots
                .copy_in(0, slots);
        }
        Ok(result)
    }

    pub(super) fn from_nodes(nodes: &[NodeId]) -> Result<Self, ContainerSliceError> {
        let result = Self::with_len_cap(nodes.len(), nodes.len())?;
        if !nodes.is_empty() {
            let slots = &result.backing.as_ref().expect("allocated slice").slots;
            for (index, &node) in nodes.iter().enumerate() {
                slots.store(index, Some(node));
            }
        }
        Ok(result)
    }

    pub(super) fn is_nil(&self) -> bool {
        self.backing.is_none()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(super) fn len(&self) -> usize {
        self.len
    }
    pub(super) fn capacity(&self) -> usize {
        self.cap
    }
    pub(super) fn offset(&self) -> usize {
        self.start
    }

    /// Range over a copied header. Each item is read from the live backing
    /// when next() is called; nil slots are not discarded.
    pub(super) fn iter_slots(&self) -> impl Iterator<Item = Option<NodeId>> + use<> {
        let header = self.clone();
        (0..header.len).map(move |index| header.get(index).expect("captured slice bound"))
    }

    /// Borrow an already-owned header. The borrow prevents replacing or dropping
    /// that header; each slot still reads the live backing when reached. Use
    /// iter_slots instead when the iterator must outlive its header.
    pub(super) fn borrowed_slots(&self) -> impl Iterator<Item = Option<NodeId>> + '_ {
        (0..self.len).map(|index| self.get(index).expect("borrowed slice bound"))
    }

    /// Like a Go range over the header, but defer each value load until after
    /// an item poll. The backing never moves and its slots remain atomic.
    pub(super) fn borrowed_slot_readers(&self) -> BorrowedNodeSlots<'_> {
        BorrowedNodeSlots(self.backing.as_ref().map_or(&[], |backing| {
            &backing.slots.0[self.start..self.start + self.len]
        }))
    }

    pub(super) fn require_nodes(&self) -> impl Iterator<Item = NodeId> + use<> {
        self.iter_slots()
            .map(|slot| slot.expect("nil container child"))
    }

    /// Copy a numerical view without callbacks. Unlike require_nodes(), this
    /// adapter has always returned owned storage, so read the captured window
    /// under one guard. Do not use it for retained live range traversal.
    pub(super) fn copied_nodes(&self) -> Vec<NodeId> {
        let Some(backing) = &self.backing else {
            return Vec::new();
        };
        let nodes = (self.start..self.start + self.len)
            .map(|index| backing.slots.load(index))
            .collect::<Option<Vec<NodeId>>>();
        // A nil pointer panics with no guard held, preserving the backing's
        // usability when an outer source-shaped recovery runs.
        nodes.expect("nil container child")
    }

    /// None is out of bounds; Some(None) is an actual nil pointer slot.
    pub(super) fn get(&self, index: usize) -> Option<Option<NodeId>> {
        (index < self.len)
            .then(|| self.read_capacity_slot(index))
            .flatten()
    }

    pub(super) fn read_capacity_slot(&self, index: usize) -> Option<Option<NodeId>> {
        if index >= self.cap {
            return None;
        }
        Some(
            self.backing
                .as_ref()
                .expect("positive capacity has backing")
                .slots
                .load(self.start + index),
        )
    }

    pub(super) fn write_capacity_slot(
        &self,
        index: usize,
        value: Option<NodeId>,
    ) -> Result<(), ContainerSliceError> {
        if index >= self.cap {
            return Err(ContainerSliceError::Bounds);
        }
        self.backing
            .as_ref()
            .expect("positive capacity has backing")
            .slots
            .store(self.start + index, value);
        Ok(())
    }

    pub(super) fn visible_slots(&self) -> Vec<Option<NodeId>> {
        self.slot_window(self.len)
    }
    pub(super) fn capacity_slots(&self) -> Vec<Option<NodeId>> {
        self.slot_window(self.cap)
    }

    fn slot_window(&self, length: usize) -> Vec<Option<NodeId>> {
        let Some(backing) = &self.backing else {
            return Vec::new();
        };
        backing.slots.copy_out(self.start..self.start + length)
    }

    pub(super) fn reslice(&self, low: usize, high: usize) -> Result<Self, ContainerSliceError> {
        self.reslice_with_capacity(low, high, self.cap)
    }

    pub(super) fn reslice_with_capacity(
        &self,
        low: usize,
        high: usize,
        max: usize,
    ) -> Result<Self, ContainerSliceError> {
        if low > high || high > max || max > self.cap {
            return Err(ContainerSliceError::Bounds);
        }
        Ok(Self {
            backing: self.backing.clone(),
            start: self.start + low,
            len: high - low,
            cap: max - low,
        })
    }

    /// Source append returns a new header. Existing header lengths do not
    /// change; writes within spare capacity affect every backing alias.
    pub(super) fn append(&self, values: &[Option<NodeId>]) -> Result<Self, ContainerSliceError> {
        self.append_with_buffer(values, 0)
    }

    /// Build an unaliased local slice using the pinned compiler's four-pointer
    /// stack buffer, then publish its heap header. codec.go's containerNodes
    /// appends call growsliceBuf(..., 4); publication calls moveSlice, preserving
    /// capacity. No intermediate local header escapes this constructor.
    pub(super) fn from_go127_buffered_appends(
        nodes: &[NodeId],
    ) -> Result<Self, ContainerSliceError> {
        let mut header = Self::allocated_empty();
        for &node in nodes {
            header = header.append_with_buffer(&[Some(node)], 4)?;
        }
        Ok(header)
    }

    fn append_with_buffer(
        &self,
        values: &[Option<NodeId>],
        buffer_capacity: usize,
    ) -> Result<Self, ContainerSliceError> {
        if values.is_empty() {
            return Ok(self.clone());
        }
        let new_len = self
            .len
            .checked_add(values.len())
            .ok_or(ContainerSliceError::LengthOverflow)?;
        if new_len <= self.cap {
            self.backing
                .as_ref()
                .expect("positive capacity has backing")
                .slots
                .copy_in(self.start + self.len, values);
            return Ok(Self {
                len: new_len,
                ..self.clone()
            });
        }
        // runtime.growsliceBuf rounds just the requested size while it fits
        // the buffer; beyond the buffer it delegates to ordinary growslice.
        let growth_base = if new_len <= buffer_capacity {
            0
        } else {
            self.cap
        };
        let new_cap = go127_pointer_growth_capacity(growth_base, new_len)?;
        let result = Self::with_len_cap(new_len, new_cap)?;
        let old_values = self.visible_slots();
        {
            let slots = &result.backing.as_ref().expect("grown allocation").slots;
            slots.copy_in(0, &old_values);
            slots.copy_in(self.len, values);
        }
        Ok(result)
    }

    pub(super) fn append_node(&self, node: NodeId) -> Result<Self, ContainerSliceError> {
        self.append(&[Some(node)])
    }

    /// node.go1072–1080 uses append(prefix, suffix...), not slices.Delete:
    /// remove only the first match, shift in place, retain the stale tail.
    pub(super) fn remove_first_slot(&self, value: Option<NodeId>) -> Self {
        if self.len == 0 {
            return self.clone();
        }
        let slots = &self
            .backing
            .as_ref()
            .expect("positive length has backing")
            .slots;
        let Some(index) =
            (0..self.len).position(|relative| slots.load(self.start + relative) == value)
        else {
            return self.clone();
        };
        for relative in index..self.len - 1 {
            slots.store(self.start + relative, slots.load(self.start + relative + 1));
        }
        Self {
            len: self.len - 1,
            ..self.clone()
        }
    }

    pub(super) fn clear_visible(&self) {
        if let Some(backing) = &self.backing {
            for index in self.start..self.start + self.len {
                backing.slots.store(index, None);
            }
        }
    }

    /// Independent graph cloning uses make(len), including a nonnil empty
    /// result for nil/empty input. It is distinct from header Clone above.
    pub(super) fn clone_allocated(&self) -> Result<Self, ContainerSliceError> {
        Self::from_slots(&self.visible_slots())
    }

    /// Independent numerical candidates retain source header shape and all
    /// stale backing slots. Fork the complete inventory together so overlapping
    /// member headers still alias within the candidate, without sharing writes
    /// with its parent. This is separate from copying a live range header.
    pub(super) fn fork_storage_batch(headers: &[Self]) -> Result<Vec<Self>, ContainerSliceError> {
        Self::fork_borrowed_storage_batch(headers.iter())
    }

    /// Source owners keep every backing alive; duplication does not call owner hooks.
    pub(super) fn fork_borrowed_storage_batch<'a>(
        headers: impl IntoIterator<Item = &'a Self>,
    ) -> Result<Vec<Self>, ContainerSliceError> {
        // Each original Arc remains alive in headers for this entire walk;
        // its actual allocation address is only a local deduplication key.
        let mut backings: BTreeMap<usize, Arc<NodeSliceBacking>> = BTreeMap::new();
        headers
            .into_iter()
            .map(|header| {
                let backing = if let Some(source) = &header.backing {
                    let address = Arc::as_ptr(source) as usize;
                    if let Some(existing) = backings.get(&address) {
                        Some(Arc::clone(existing))
                    } else {
                        let copied = allocate_backing(source.slots.len())?;
                        copied.slots.copy_from(&source.slots);
                        backings.insert(address, Arc::clone(&copied));
                        Some(copied)
                    }
                } else {
                    None
                };
                Ok(Self {
                    backing,
                    start: header.start,
                    len: header.len,
                    cap: header.cap,
                })
            })
            .collect()
    }

    pub(super) fn snapshot(&self) -> SharedNodeSliceSnapshot {
        SharedNodeSliceSnapshot {
            original: self.clone(),
            capacity_values: self.capacity_slots(),
        }
    }

    /// Equal Go slice headers: the same backing allocation, offset, length
    /// and capacity. A copy of either is indistinguishable from the other.
    pub(super) fn same_header(&self, other: &Self) -> bool {
        self.same_backing_allocation(other)
            && self.start == other.start
            && self.len == other.len
            && self.cap == other.cap
    }

    pub(super) fn same_backing_allocation(&self, other: &Self) -> bool {
        match (&self.backing, &other.backing) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false, // nil does not name an allocation.
        }
    }

    /// Actual addresses are observation-only. Zero-cap data equality alone
    /// does not establish that an independently allocated heap slice aliases.
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn data_address(&self) -> Option<usize> {
        let backing = self.backing.as_ref()?;
        Some(backing.slots.as_ptr().wrapping_add(self.start) as usize)
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn slot_address(&self, index: usize) -> Option<usize> {
        (index < self.cap)
            .then(|| {
                self.backing
                    .as_ref()
                    .map(|backing| backing.slots.as_ptr().wrapping_add(self.start + index) as usize)
            })
            .flatten()
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn backing_address(&self) -> Option<usize> {
        self.backing
            .as_ref()
            .map(|backing| Arc::as_ptr(backing) as usize)
    }
}

#[derive(Clone, Debug)]
pub(super) struct SharedNodeSliceSnapshot {
    original: SharedNodeSlice,
    capacity_values: Vec<Option<NodeId>>,
}

impl SharedNodeSliceSnapshot {
    /// Repair exactly [start,start+cap), never the entire Box. Writes through
    /// aliases outside every captured header's window must survive rollback.
    pub(super) fn restore(&self) -> SharedNodeSlice {
        if let Some(backing) = &self.original.backing {
            backing
                .slots
                .copy_in(self.original.start, &self.capacity_values);
        }
        self.original.clone()
    }
}

type SliceEntries<K> = BTreeMap<K, SharedNodeSlice>;

/// Clone aliases the actual map allocation. A key can be absent, or present
/// with a genuine nil header. Default/new is the allocated NewGraph map;
/// use nil() for an actual nil source map.
pub(super) struct SharedSliceMap<K = Option<NodeId>> {
    entries: Option<Arc<GoCell<SliceEntries<K>>>>,
}

impl<K> Clone for SharedSliceMap<K> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
}
impl<K> Default for SharedSliceMap<K> {
    fn default() -> Self {
        Self {
            entries: Some(Arc::new(GoCell::new(BTreeMap::new()))),
        }
    }
}
impl<K> fmt::Debug for SharedSliceMap<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedSliceMap")
            .field("nil", &self.is_nil())
            .finish()
    }
}
impl<K> SharedSliceMap<K> {
    pub(super) fn new() -> Self {
        Self::default()
    }
    pub(super) fn nil() -> Self {
        Self { entries: None }
    }
    pub(super) fn is_nil(&self) -> bool {
        self.entries.is_none()
    }
    /// A map count reads its actual allocation without copying any headers.
    pub(super) fn len(&self) -> usize {
        self.entries.as_ref().map_or(0, |entries| {
            entries.read().expect("container map lock poisoned").len()
        })
    }
    pub(super) fn same_map_allocation(&self, other: &Self) -> bool {
        match (&self.entries, &other.entries) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn map_address(&self) -> Option<usize> {
        self.entries
            .as_ref()
            .map(|entries| Arc::as_ptr(entries) as usize)
    }
}

impl<K: Ord + Clone> SharedSliceMap<K> {
    /// Read the current entry without retaining the map or slice allocation.
    /// The callback must not mutate this map or call external owner callbacks.
    pub(super) fn with_entry<R>(
        &self,
        key: &K,
        read: impl FnOnce(Option<&SharedNodeSlice>) -> R,
    ) -> R {
        match &self.entries {
            Some(entries) => read(
                entries
                    .read()
                    .expect("container map lock poisoned")
                    .get(key),
            ),
            None => read(None),
        }
    }
    /// A callback-free ordered projection. No map/header references escape.
    pub(super) fn map_entries<R>(&self, mut read: impl FnMut(&K, &SharedNodeSlice) -> R) -> Vec<R> {
        match &self.entries {
            Some(entries) => entries
                .read()
                .expect("container map lock poisoned")
                .iter()
                .map(|(key, value)| read(key, value))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Go clear(map) preserves the actual map allocation and retained slices.
    pub(super) fn clear(&self) {
        if let Some(entries) = &self.entries {
            entries
                .write()
                .expect("container map lock poisoned")
                .clear();
        }
    }
    /// Retain the map allocation while reading each next key/value live. New
    /// keys before the cursor may be skipped, as Go permits for map insertion.
    pub(super) fn live_entries(&self) -> impl Iterator<Item = (K, SharedNodeSlice)> + use<K> {
        let map = self.clone();
        let mut previous: Option<K> = None;
        std::iter::from_fn(move || {
            let entries = map
                .entries
                .as_ref()?
                .read()
                .expect("container map lock poisoned");
            let entry = match previous.as_ref() {
                Some(key) => entries
                    .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
                    .next(),
                None => entries.iter().next(),
            }
            .map(|(key, value)| (key.clone(), value.clone()));
            if let Some((key, _)) = &entry {
                previous = Some(key.clone());
            }
            entry
        })
    }
    /// Missing key is None; present nil is Some(header with is_nil()).
    pub(super) fn get(&self, key: &K) -> Option<SharedNodeSlice> {
        self.entries
            .as_ref()?
            .read()
            .expect("container map lock poisoned")
            .get(key)
            .cloned()
    }
    pub(super) fn entries(&self) -> Vec<(K, SharedNodeSlice)> {
        self.entries
            .as_ref()
            .map(|entries| {
                entries
                    .read()
                    .expect("container map lock poisoned")
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(super) fn replace(
        &self,
        key: K,
        value: SharedNodeSlice,
    ) -> Result<Option<SharedNodeSlice>, ContainerSliceError> {
        let entries = self
            .entries
            .as_ref()
            .ok_or(ContainerSliceError::NilMapAssignment)?;
        Ok(entries
            .write()
            .expect("container map lock poisoned")
            .insert(key, value))
    }
    pub(super) fn remove(&self, key: &K) -> Option<SharedNodeSlice> {
        self.entries
            .as_ref()?
            .write()
            .expect("container map lock poisoned")
            .remove(key)
    }
    pub(super) fn append_node(
        &self,
        key: K,
        node: NodeId,
    ) -> Result<SharedNodeSlice, ContainerSliceError> {
        let entries = self
            .entries
            .as_ref()
            .ok_or(ContainerSliceError::NilMapAssignment)?;
        let mut entries = entries.write().expect("container map lock poisoned");
        let previous = entries.get(&key).cloned().unwrap_or_default();
        let appended = previous.append_node(node)?;
        entries.insert(key, appended.clone());
        Ok(appended)
    }
    pub(super) fn clone_allocated(&self) -> Result<Self, ContainerSliceError> {
        let result = Self::new();
        for (key, value) in self.entries() {
            result.replace(key, value.clone_allocated()?)?;
        }
        Ok(result)
    }

    /// Detach an independent Rust candidate's storage without changing its
    /// headers, stale slots, or relationships among overlapping headers.
    /// This is neither source CopyEntitiesFrom (Clone aliases) nor source
    /// full Clone (clone_allocated makes each value with cap=len). The whole
    /// allocation is copied here; rollback still captures only cap windows.
    pub(super) fn fork_storage(&self) -> Result<Self, ContainerSliceError> {
        let Some(original) = &self.entries else {
            return Ok(Self::nil());
        };
        let original = original.read().expect("container map lock poisoned");
        let result = Self::new();
        let mut detached = result
            .entries
            .as_ref()
            .expect("allocated detached map")
            .write()
            .expect("container map lock poisoned");
        // Physical addresses deduplicate allocations only while every source
        // Arc remains alive under the original map guard. They are not
        // reported as synthetic backing identity or pointer equivalence.
        let mut backings: BTreeMap<usize, Arc<NodeSliceBacking>> = BTreeMap::new();
        for (key, header) in original.iter() {
            let backing = if let Some(source) = &header.backing {
                let address = Arc::as_ptr(source) as usize;
                if let Some(existing) = backings.get(&address) {
                    Some(Arc::clone(existing))
                } else {
                    let copied = allocate_backing(source.slots.len())?;
                    copied.slots.copy_from(&source.slots);
                    backings.insert(address, Arc::clone(&copied));
                    Some(copied)
                }
            } else {
                None
            };
            detached.insert(
                key.clone(),
                SharedNodeSlice {
                    backing,
                    start: header.start,
                    len: header.len,
                    cap: header.cap,
                },
            );
        }
        drop(detached);
        Ok(result)
    }

    /// Detach maps and standalone slice headers together, preserving aliases
    /// across different maps, Graph.Nodes and group members in one candidate.
    pub(super) fn fork_storage_batch(
        maps: &[Self],
        headers: &[SharedNodeSlice],
    ) -> Result<(Vec<Self>, Vec<SharedNodeSlice>), ContainerSliceError> {
        // Hold the source map reads only through this callback-free duplication.
        // References preserve original headers without temporary Arc ownership.
        let entries: Vec<_> = maps
            .iter()
            .map(|map| {
                map.entries
                    .as_ref()
                    .map(|entries| entries.read().expect("container map lock poisoned"))
            })
            .collect();
        let mut detached = SharedNodeSlice::fork_borrowed_storage_batch(
            headers.iter().chain(
                entries
                    .iter()
                    .flat_map(|map| map.iter().flat_map(|map| map.values())),
            ),
        )?
        .into_iter();
        let headers = detached.by_ref().take(headers.len()).collect();
        let mut result: Vec<Self> = Vec::new();
        for (index, original) in maps.iter().enumerate() {
            let values: Vec<_> = detached
                .by_ref()
                .take(entries[index].as_ref().map_or(0, |map| map.len()))
                .collect();
            if let Some(prior) = maps[..index]
                .iter()
                .position(|m| m.same_map_allocation(original))
            {
                result.push(result[prior].clone());
            } else if original.is_nil() {
                result.push(Self::nil());
            } else {
                let map = Self::new();
                for ((key, _), value) in
                    entries[index].iter().flat_map(|map| map.iter()).zip(values)
                {
                    map.replace(key.clone(), value)?;
                }
                result.push(map);
            }
        }
        Ok((result, headers))
    }

    pub(super) fn snapshot(&self) -> SharedSliceMapSnapshot<K> {
        let entries = self
            .entries
            .as_ref()
            .map(|entries| {
                let entries = entries.read().expect("container map lock poisoned");
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), value.snapshot()))
                    .collect()
            })
            .unwrap_or_default();
        SharedSliceMapSnapshot {
            original: self.clone(),
            entries,
        }
    }
}

/// The sole production Containers inventory. Numerical ArenaGraph candidates
/// detach storage; CopyEntitiesFrom-style owner views explicitly share_map().
/// Traversal snapshots are temporary adapters, never a second stored authority.
#[derive(Debug, Default)]
pub(super) struct ContainerMap {
    field: super::graph_node_maps::GraphNodeMap,
}

impl Clone for ContainerMap {
    fn clone(&self) -> Self {
        Self::from_shared(
            self.share_map()
                .fork_storage()
                .expect("detach numerical container map"),
        )
    }
}

impl ContainerMap {
    pub(super) fn new() -> Self {
        Self::default()
    }
    pub(super) fn from_shared(slices: SharedSliceMap) -> Self {
        Self {
            field: super::graph_node_maps::GraphNodeMap::from_header(slices),
        }
    }
    pub(super) fn share_map(&self) -> SharedSliceMap {
        self.field.header()
    }
    /// Read one entry of the current map field in place.
    pub(super) fn with_header_entry<R>(
        &self,
        key: &Option<NodeId>,
        read: impl FnOnce(Option<&SharedNodeSlice>) -> R,
    ) -> R {
        self.field.read_header().with_entry(key, read)
    }
    pub(super) fn share_field(&self) -> super::graph_node_maps::GraphNodeMap {
        self.field.share_graph()
    }
    pub(super) fn from_field(field: super::graph_node_maps::GraphNodeMap) -> Self {
        Self { field }
    }
    pub(super) fn replace_map(&self, map: SharedSliceMap) {
        self.field.replace_header(map);
    }
    pub(super) fn snapshot(&self) -> SharedSliceMapSnapshot {
        self.share_map().snapshot()
    }
    pub(super) fn get_header(&self, key: &Option<NodeId>) -> Option<SharedNodeSlice> {
        self.field.read_header().get(key)
    }
    pub(super) fn get(&self, key: &Option<NodeId>) -> Option<Vec<NodeId>> {
        self.field
            .read_header()
            .with_entry(key, |header| header.map(SharedNodeSlice::copied_nodes))
    }
    pub(super) fn nodes(&self, key: &Option<NodeId>) -> Vec<NodeId> {
        self.get(key).expect("missing container entry")
    }
    pub(super) fn contains_key(&self, key: &Option<NodeId>) -> bool {
        self.field
            .read_header()
            .with_entry(key, |header| header.is_some())
    }
    pub(super) fn len(&self) -> usize {
        self.field.read_header().len()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub(super) fn iter(&self) -> std::vec::IntoIter<(Option<NodeId>, Vec<NodeId>)> {
        self.field
            .read_header()
            .map_entries(|key, header| (*key, header.copied_nodes()))
            .into_iter()
    }
    pub(super) fn keys(&self) -> std::vec::IntoIter<Option<NodeId>> {
        self.field
            .read_header()
            .map_entries(|key, _| *key)
            .into_iter()
    }
    pub(super) fn values(&self) -> std::vec::IntoIter<Vec<NodeId>> {
        self.iter()
            .map(|(_, nodes)| nodes)
            .collect::<Vec<_>>()
            .into_iter()
    }
    /// Exact make(len) construction for explicitly projected numerical maps
    /// and fixtures. Production append writers use append_node instead.
    pub(super) fn insert(&self, key: Option<NodeId>, nodes: Vec<NodeId>) {
        self.replace_header(
            key,
            SharedNodeSlice::from_nodes(&nodes).expect("container make"),
        );
    }
    pub(super) fn replace_header(&self, key: Option<NodeId>, header: SharedNodeSlice) {
        self.share_map()
            .replace(key, header)
            .expect("container map assignment");
    }
    pub(super) fn append_node(&self, key: Option<NodeId>, node: NodeId) -> SharedNodeSlice {
        self.share_map()
            .append_node(key, node)
            .expect("container append")
    }
    pub(super) fn remove(&self, key: &Option<NodeId>) -> Option<SharedNodeSlice> {
        self.share_map().remove(key)
    }
    pub(super) fn clear(&self) {
        self.share_map().clear();
    }
    pub(super) fn sort_by_key<T: Ord>(
        &self,
        key: &Option<NodeId>,
        mut sort_key: impl FnMut(&NodeId) -> T,
    ) {
        let Some(header) = self.get_header(key) else {
            return;
        };
        let mut nodes: Vec<_> = header.require_nodes().collect();
        nodes.stable_sort_by_key(|node| sort_key(node));
        for (index, node) in nodes.into_iter().enumerate() {
            header
                .write_capacity_slot(index, Some(node))
                .expect("container sort bound");
        }
    }
}

impl<const N: usize> From<[(Option<NodeId>, Vec<NodeId>); N]> for ContainerMap {
    fn from(entries: [(Option<NodeId>, Vec<NodeId>); N]) -> Self {
        let result = Self::new();
        for (key, nodes) in entries {
            result.insert(key, nodes);
        }
        result
    }
}

impl IntoIterator for &ContainerMap {
    type Item = (Option<NodeId>, Vec<NodeId>);
    type IntoIter = std::vec::IntoIter<Self::Item>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Clone, Debug)]
pub(super) struct SharedSliceMapSnapshot<K = Option<NodeId>> {
    original: SharedSliceMap<K>,
    entries: Vec<(K, SharedNodeSliceSnapshot)>,
}
impl<K: Ord + Clone> SharedSliceMapSnapshot<K> {
    /// Restore original map and each original header's captured capacity
    /// window. Unrelated replacement maps/backings remain alive and untouched.
    pub(super) fn restore(&self) -> SharedSliceMap<K> {
        if let Some(original) = &self.original.entries {
            let mut entries = original.write().expect("container map lock poisoned");
            entries.clear();
            for (key, value) in &self.entries {
                entries.insert(key.clone(), value.restore());
            }
        }
        self.original.clone()
    }
}

// Pinned 64-bit Go 1.27 pointer-slice policy. These are Go source capacities,
// independent of Rust's element layout or native allocator rounding.
const GO_POINTER_BYTES: usize = 8;
const GO_MALLOC_HEADER: usize = 8;
const GO_MIN_SCAN_HEADER: usize = 512;
const GO_MAX_SMALL_SIZE: usize = 32768;
const GO_PAGE_SIZE: usize = 8192;
const GO_SIZE_CLASSES: [usize; 68] = [
    0, 8, 16, 24, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 256, 288, 320,
    352, 384, 416, 448, 480, 512, 576, 640, 704, 768, 896, 1024, 1152, 1280, 1408, 1536, 1792,
    2048, 2304, 2688, 3072, 3200, 3456, 4096, 4864, 5376, 6144, 6528, 6784, 6912, 8192, 9472, 9728,
    10240, 10880, 12288, 13568, 14336, 16384, 18432, 19072, 20480, 21760, 24576, 27264, 28672,
    32768,
];

pub(super) fn check_pointer_capacity(capacity: usize) -> Result<(), ContainerSliceError> {
    let bytes = (capacity as u64)
        .checked_mul(GO_POINTER_BYTES as u64)
        .ok_or(ContainerSliceError::LengthOverflow)?;
    if bytes > (1_u64 << 48) || capacity > isize::MAX as usize {
        return Err(ContainerSliceError::LengthOverflow);
    }
    Ok(())
}

/// Source pointer slices use scanned eight-byte elements.
pub(super) fn go127_pointer_growth_capacity(
    old_cap: usize,
    new_len: usize,
) -> Result<usize, ContainerSliceError> {
    go127_slice_growth_capacity(old_cap, new_len, GO_POINTER_BYTES, true)
}

/// runtime/slice.go nextslicecap followed by element-aware roundupsize.
pub(super) fn go127_slice_growth_capacity(
    old_cap: usize,
    new_len: usize,
    element_bytes: usize,
    contains_pointers: bool,
) -> Result<usize, ContainerSliceError> {
    if new_len <= old_cap {
        return Ok(old_cap);
    }
    let check = |capacity: usize| {
        let bytes = capacity
            .checked_mul(element_bytes)
            .ok_or(ContainerSliceError::LengthOverflow)?;
        if (bytes as u64) > (1_u64 << 48) || capacity > isize::MAX as usize {
            return Err(ContainerSliceError::LengthOverflow);
        }
        Ok(())
    };
    check(new_len)?;
    let double = old_cap.checked_mul(2);
    let mut next = if double.is_none_or(|double| new_len > double) {
        new_len
    } else if old_cap < 256 {
        double.expect("checked double")
    } else {
        let mut next = old_cap;
        while next < new_len {
            let increment = next.checked_add(3 * 256).map(|value| value >> 2);
            let Some(grown) = increment.and_then(|increment| next.checked_add(increment)) else {
                next = new_len;
                break;
            };
            next = grown;
        }
        next
    };
    let bytes = next
        .checked_mul(element_bytes)
        .ok_or(ContainerSliceError::LengthOverflow)?;
    let rounded = if bytes <= GO_MAX_SMALL_SIZE - GO_MALLOC_HEADER {
        let header = if contains_pointers && bytes > GO_MIN_SCAN_HEADER {
            GO_MALLOC_HEADER
        } else {
            0
        };
        let request = bytes
            .checked_add(header)
            .ok_or(ContainerSliceError::LengthOverflow)?;
        GO_SIZE_CLASSES
            .iter()
            .copied()
            .find(|&size| size >= request)
            .expect("small request has a size class")
            - header
    } else {
        bytes
            .checked_add(GO_PAGE_SIZE - 1)
            .ok_or(ContainerSliceError::LengthOverflow)?
            & !(GO_PAGE_SIZE - 1)
    };
    next = rounded / element_bytes;
    check(next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(value: u32) -> Option<NodeId> {
        Some(NodeId(value))
    }

    #[test]
    fn production_map_readers_follow_actual_backing_and_map_replacement() {
        let map = ContainerMap::new();
        let header = SharedNodeSlice::with_len_cap(2, 4).unwrap();
        header.write_capacity_slot(0, n(1)).unwrap();
        header.write_capacity_slot(1, n(2)).unwrap();
        map.replace_header(None, header.clone());
        let owner_view = map.share_map();
        let mut range = header.iter_slots();
        assert_eq!(range.next(), Some(n(1)));
        header.write_capacity_slot(1, n(3)).unwrap();
        assert_eq!(range.next(), Some(n(3)));
        assert_eq!(map.nodes(&None), [NodeId(1), NodeId(3)]);
        let appended = map.append_node(None, NodeId(4));
        assert!(header.same_backing_allocation(&appended));
        assert_eq!(header.len(), 2);
        assert_eq!(owner_view.get(&None).unwrap().len(), 3);
        map.insert(None, vec![NodeId(5)]);
        assert_eq!(owner_view.get(&None).unwrap().visible_slots(), [n(5)]);
        assert_eq!(header.visible_slots(), [n(1), n(3)]);
        map.clear();
        assert!(owner_view.get(&None).is_none());
        assert_eq!(header.read_capacity_slot(2), Some(n(4)));
    }

    #[test]
    fn production_numerical_clone_detaches_storage_but_retains_header_shape() {
        let map = ContainerMap::new();
        let backing = SharedNodeSlice::with_len_cap(4, 6).unwrap();
        for index in 0..4 {
            backing
                .write_capacity_slot(index, n(index as u32 + 1))
                .unwrap();
        }
        map.replace_header(None, backing.reslice_with_capacity(1, 3, 5).unwrap());
        map.replace_header(Some(NodeId(9)), backing.clone());
        let candidate = map.clone();
        let local = candidate.get_header(&None).unwrap();
        assert_eq!((local.offset(), local.len(), local.capacity()), (1, 2, 4));
        assert!(!local.same_backing_allocation(&backing));
        assert!(local.same_backing_allocation(&candidate.get_header(&Some(NodeId(9))).unwrap()));
        local.write_capacity_slot(0, n(7)).unwrap();
        assert_eq!(candidate.nodes(&None), [NodeId(7), NodeId(3)]);
        assert_eq!(map.nodes(&None), [NodeId(2), NodeId(3)]);
    }

    #[test]
    fn production_nil_slots_remain_visible_and_are_not_dropped_by_traversal() {
        let map = ContainerMap::new();
        map.replace_header(
            None,
            SharedNodeSlice::from_slots(&[n(1), None, n(2)]).unwrap(),
        );
        assert_eq!(
            map.get_header(&None).unwrap().visible_slots(),
            [n(1), None, n(2)]
        );
        assert!(std::panic::catch_unwind(|| map.nodes(&None)).is_err());
        assert_eq!(map.get_header(&None).unwrap().len(), 3);
    }

    #[test]
    fn nil_absent_allocated_empty_and_spare_empty_are_distinct() {
        let map = SharedSliceMap::<Option<NodeId>>::new();
        assert!(map.get(&None).is_none());
        map.replace(None, SharedNodeSlice::nil()).unwrap();
        assert!(map.get(&None).unwrap().is_nil());
        let empty = SharedNodeSlice::allocated_empty();
        assert!(!empty.is_nil());
        assert_eq!((empty.len(), empty.capacity()), (0, 0));
        assert!(empty.data_address().is_some());
        let spare = SharedNodeSlice::with_len_cap(0, 4).unwrap();
        assert_eq!(spare.capacity_slots(), [None; 4]);
        assert!(!spare.same_backing_allocation(&empty));
        assert!(SharedSliceMap::<Option<NodeId>>::nil().is_nil());
        assert!(
            SharedSliceMap::<Option<NodeId>>::nil()
                .replace(None, empty)
                .is_err()
        );
    }

    #[test]
    fn shared_map_replacement_does_not_retarget_copied_old_header() {
        let map = SharedSliceMap::<Option<NodeId>>::new();
        let first = SharedNodeSlice::from_slots(&[n(1), n(2)]).unwrap();
        map.replace(None, first.clone()).unwrap();
        let view = map.clone();
        let copied = view.get(&None).unwrap();
        let replacement = SharedNodeSlice::from_slots(&[n(7)]).unwrap();
        map.replace(None, replacement.clone()).unwrap();
        assert!(map.same_map_allocation(&view));
        assert_eq!(map.map_address(), view.map_address());
        assert!(
            view.get(&None)
                .unwrap()
                .same_backing_allocation(&replacement)
        );
        assert!(copied.same_backing_allocation(&first));
        assert_eq!(copied.visible_slots(), [n(1), n(2)]);
        first.write_capacity_slot(1, n(9)).unwrap();
        assert_eq!(copied.get(1), Some(n(9)));
        assert_eq!(view.get(&None).unwrap().visible_slots(), [n(7)]);
    }

    #[test]
    fn append_in_spare_capacity_changes_real_slots_but_only_returned_header_len() {
        let slice = SharedNodeSlice::with_len_cap(2, 4).unwrap();
        slice.write_capacity_slot(0, n(1)).unwrap();
        slice.write_capacity_slot(1, n(2)).unwrap();
        slice.write_capacity_slot(2, n(88)).unwrap();
        let old_header = slice.clone();
        let old_address = slice.data_address();
        let appended = slice.append_node(NodeId(3)).unwrap();
        assert!(appended.same_backing_allocation(&old_header));
        assert_eq!(appended.data_address(), old_address);
        assert_eq!((old_header.len(), appended.len()), (2, 3));
        assert_eq!(old_header.capacity_slots(), [n(1), n(2), n(3), None]);
        assert_eq!(old_header.reslice(0, 3).unwrap().get(2), Some(n(3)));
    }

    #[test]
    fn overflow_append_retains_old_backing_and_initializes_new_tail() {
        let old = SharedNodeSlice::from_slots(&[n(1), n(2)]).unwrap();
        let appended = old.append_node(NodeId(3)).unwrap();
        assert!(!old.same_backing_allocation(&appended));
        assert_ne!(old.data_address(), appended.data_address());
        assert_eq!(old.capacity_slots(), [n(1), n(2)]);
        assert_eq!(appended.capacity_slots(), [n(1), n(2), n(3), None]);
    }

    #[test]
    fn offset_and_restricted_capacity_use_actual_slot_addresses() {
        let backing = SharedNodeSlice::from_slots(&[n(9), n(1), n(2), n(3), n(8)]).unwrap();
        let offset = backing.reslice(1, 3).unwrap();
        let restricted = backing.reslice_with_capacity(1, 3, 3).unwrap();
        assert_eq!(
            (offset.offset(), offset.len(), offset.capacity()),
            (1, 2, 4)
        );
        assert_eq!(
            (restricted.offset(), restricted.len(), restricted.capacity()),
            (1, 2, 2)
        );
        assert_eq!(offset.data_address(), backing.slot_address(1));
        assert!(offset.same_backing_allocation(&backing));
        let in_place = offset.append_node(NodeId(4)).unwrap();
        assert_eq!(in_place.data_address(), offset.data_address());
        assert_eq!(backing.visible_slots(), [n(9), n(1), n(2), n(4), n(8)]);
        let grown = restricted.append_node(NodeId(5)).unwrap();
        assert!(!grown.same_backing_allocation(&backing));
        assert_eq!(grown.visible_slots(), [n(1), n(2), n(5)]);
        assert_eq!(backing.visible_slots(), [n(9), n(1), n(2), n(4), n(8)]);
        assert!(backing.reslice_with_capacity(2, 4, 3).is_err());
    }

    #[test]
    fn remove_first_keeps_duplicates_and_source_stale_tail() {
        let slice = SharedNodeSlice::from_slots(&[n(1), n(2), n(1), n(4)]).unwrap();
        let old_header = slice.clone();
        let removed = slice.remove_first_slot(n(1));
        assert!(removed.same_backing_allocation(&slice));
        assert_eq!(removed.visible_slots(), [n(2), n(1), n(4)]);
        assert_eq!(old_header.visible_slots(), [n(2), n(1), n(4), n(4)]);
        assert_eq!(removed.capacity_slots(), [n(2), n(1), n(4), n(4)]);
    }

    #[test]
    fn clear_only_repairs_visible_region_and_preserves_spare_tail() {
        let all = SharedNodeSlice::from_slots(&[n(1), n(2), n(88), n(99)]).unwrap();
        let visible = all.reslice(0, 2).unwrap();
        visible.clear_visible();
        assert_eq!(all.visible_slots(), [None, None, n(88), n(99)]);
    }

    #[test]
    fn snapshot_repairs_captured_window_and_leaves_outside_writes() {
        let all = SharedNodeSlice::from_slots(&[n(10), n(11), n(12), n(13), n(14), n(15)]).unwrap();
        let window = all.reslice_with_capacity(1, 3, 4).unwrap();
        let snapshot = window.snapshot();
        all.write_capacity_slot(0, n(80)).unwrap();
        all.write_capacity_slot(2, n(82)).unwrap();
        all.write_capacity_slot(4, n(84)).unwrap();
        let restored = snapshot.restore();
        assert_eq!(restored.data_address(), window.data_address());
        assert_eq!(
            (restored.offset(), restored.len(), restored.capacity()),
            (1, 2, 3)
        );
        assert_eq!(
            all.visible_slots(),
            [n(80), n(11), n(12), n(13), n(84), n(15)]
        );
    }

    #[test]
    fn map_snapshot_restores_original_allocations_not_unrelated_replacements() {
        let all = SharedNodeSlice::from_slots(&[n(9), n(1), n(2), n(33), n(44)]).unwrap();
        let window = all.reslice_with_capacity(1, 3, 4).unwrap();
        let map = SharedSliceMap::<Option<NodeId>>::new();
        map.replace(None, window.clone()).unwrap();
        map.replace(Some(NodeId(7)), SharedNodeSlice::nil())
            .unwrap();
        let view = map.clone();
        let snapshot = map.snapshot();
        let retained_new = SharedNodeSlice::from_slots(&[n(90), n(91)]).unwrap();
        map.replace(None, retained_new.clone()).unwrap();
        map.remove(&Some(NodeId(7)));
        map.replace(Some(NodeId(8)), SharedNodeSlice::allocated_empty())
            .unwrap();
        all.write_capacity_slot(2, n(82)).unwrap();
        all.write_capacity_slot(4, n(84)).unwrap();
        let unrelated_map = SharedSliceMap::<Option<NodeId>>::new();
        unrelated_map.replace(None, retained_new.clone()).unwrap();
        let restored = snapshot.restore();
        assert!(restored.same_map_allocation(&map));
        assert_eq!(restored.map_address(), view.map_address());
        assert_eq!(
            view.get(&None).unwrap().data_address(),
            window.data_address()
        );
        assert!(view.get(&Some(NodeId(7))).unwrap().is_nil());
        assert!(view.get(&Some(NodeId(8))).is_none());
        assert_eq!(all.visible_slots(), [n(9), n(1), n(2), n(33), n(84)]);
        assert_eq!(retained_new.visible_slots(), [n(90), n(91)]);
        assert!(
            unrelated_map
                .get(&None)
                .unwrap()
                .same_backing_allocation(&retained_new)
        );
    }

    #[test]
    fn overlapping_snapshot_windows_repair_the_same_actual_backing() {
        let all = SharedNodeSlice::from_slots(&[n(1), n(2), n(3), n(4), n(5), n(6)]).unwrap();
        let map = SharedSliceMap::<u8>::new();
        map.replace(1, all.reslice_with_capacity(1, 2, 4).unwrap())
            .unwrap();
        map.replace(2, all.reslice_with_capacity(2, 4, 5).unwrap())
            .unwrap();
        let snapshot = map.snapshot();
        for index in 0..6 {
            all.write_capacity_slot(index, n(90 + index as u32))
                .unwrap();
        }
        snapshot.restore();
        assert_eq!(all.visible_slots(), [n(90), n(2), n(3), n(4), n(5), n(95)]);
        assert!(
            map.get(&1)
                .unwrap()
                .same_backing_allocation(&map.get(&2).unwrap())
        );
    }

    #[test]
    fn detached_fork_preserves_overlaps_restricted_windows_and_all_stale_slots() {
        let all = SharedNodeSlice::from_slots(&[n(9), n(1), n(2), n(3), n(88), n(99)]).unwrap();
        let map = SharedSliceMap::<u8>::new();
        map.replace(1, all.reslice_with_capacity(1, 3, 4).unwrap())
            .unwrap();
        map.replace(2, all.reslice_with_capacity(2, 4, 5).unwrap())
            .unwrap();
        map.replace(3, all.clone()).unwrap();
        map.replace(4, SharedNodeSlice::nil()).unwrap();
        map.replace(5, SharedNodeSlice::allocated_empty()).unwrap();
        map.replace(6, SharedNodeSlice::allocated_empty()).unwrap();
        let copy = map.fork_storage().unwrap();
        assert!(!copy.same_map_allocation(&map));
        let copied_first = copy.get(&1).unwrap();
        let copied_second = copy.get(&2).unwrap();
        let copied_all = copy.get(&3).unwrap();
        assert_eq!(
            (
                copied_first.offset(),
                copied_first.len(),
                copied_first.capacity()
            ),
            (1, 2, 3)
        );
        assert_eq!(copied_first.capacity_slots(), [n(1), n(2), n(3)]);
        assert_eq!(copied_second.capacity_slots(), [n(2), n(3), n(88)]);
        assert!(copied_first.same_backing_allocation(&copied_second));
        assert!(copied_first.same_backing_allocation(&copied_all));
        assert!(!copied_first.same_backing_allocation(&all));
        assert_eq!(copied_first.data_address(), copied_all.slot_address(1));
        assert_eq!(copied_second.data_address(), copied_all.slot_address(2));
        assert_eq!(
            copied_all.capacity_slots(),
            [n(9), n(1), n(2), n(3), n(88), n(99)]
        );
        copied_first.write_capacity_slot(1, n(7)).unwrap();
        assert_eq!(copied_second.get(0), Some(n(7)));
        assert_eq!(all.visible_slots(), [n(9), n(1), n(2), n(3), n(88), n(99)]);
        assert!(copy.get(&4).unwrap().is_nil());
        let empty = copy.get(&5).unwrap();
        assert!(!empty.is_nil());
        assert_eq!((empty.len(), empty.capacity()), (0, 0));
        assert!(empty.same_backing_allocation(&copy.get(&6).unwrap()));
        assert!(!empty.same_backing_allocation(&map.get(&5).unwrap()));
        assert!(SharedSliceMap::<u8>::nil().fork_storage().unwrap().is_nil());
    }

    #[test]
    fn member_batch_fork_preserves_nil_windows_spare_slots_and_overlaps_without_parent_writes() {
        let all = SharedNodeSlice::from_slots(&[n(90), n(1), None, n(3), n(88), n(99)]).unwrap();
        let first = all.reslice_with_capacity(1, 3, 4).unwrap();
        let second = all.reslice_with_capacity(2, 4, 5).unwrap();
        let zero = all.reslice_with_capacity(4, 4, 4).unwrap();
        let empty = SharedNodeSlice::allocated_empty();
        let original = [
            first,
            second,
            all.clone(),
            zero,
            SharedNodeSlice::nil(),
            empty.clone(),
            empty,
        ];
        let copied = SharedNodeSlice::fork_storage_batch(&original).unwrap();
        for (old, new) in original.iter().zip(&copied) {
            assert_eq!(
                (new.is_nil(), new.offset(), new.len(), new.capacity()),
                (old.is_nil(), old.offset(), old.len(), old.capacity())
            );
            assert_eq!(new.capacity_slots(), old.capacity_slots());
            assert!(!old.same_backing_allocation(new));
        }
        assert!(copied[0].same_backing_allocation(&copied[1]));
        assert!(copied[0].same_backing_allocation(&copied[2]));
        assert!(copied[0].same_backing_allocation(&copied[3]));
        assert!(copied[5].same_backing_allocation(&copied[6]));
        assert_eq!(copied[3].data_address(), copied[2].slot_address(4));
        copied[0].write_capacity_slot(1, n(7)).unwrap();
        assert_eq!(copied[1].get(0), Some(n(7)));
        assert_eq!(copied[2].get(2), Some(n(7)));
        assert_eq!(all.visible_slots(), [n(90), n(1), None, n(3), n(88), n(99)]);
        copied[2].write_capacity_slot(5, None).unwrap();
        assert_eq!(all.read_capacity_slot(5), Some(n(99)));
        assert!(SharedNodeSlice::fork_storage_batch(&[]).unwrap().is_empty());
    }

    #[test]
    fn independent_full_clone_is_allocated_cap_len_and_does_not_share_values() {
        let map = SharedSliceMap::<Option<NodeId>>::new();
        let source = SharedNodeSlice::with_len_cap(2, 4).unwrap();
        source.write_capacity_slot(0, n(1)).unwrap();
        source.write_capacity_slot(1, n(2)).unwrap();
        source.write_capacity_slot(2, n(88)).unwrap();
        map.replace(None, source.clone()).unwrap();
        map.replace(Some(NodeId(1)), SharedNodeSlice::nil())
            .unwrap();
        map.replace(
            Some(NodeId(2)),
            SharedNodeSlice::with_len_cap(0, 4).unwrap(),
        )
        .unwrap();
        let copy = map.clone_allocated().unwrap();
        assert!(!copy.same_map_allocation(&map));
        let nodes = copy.get(&None).unwrap();
        assert_eq!((nodes.len(), nodes.capacity()), (2, 2));
        assert!(!nodes.same_backing_allocation(&source));
        source.write_capacity_slot(0, n(9)).unwrap();
        assert_eq!(nodes.visible_slots(), [n(1), n(2)]);
        for key in [Some(NodeId(1)), Some(NodeId(2))] {
            let empty = copy.get(&key).unwrap();
            assert!(!empty.is_nil());
            assert_eq!((empty.len(), empty.capacity()), (0, 0));
        }
    }

    #[test]
    fn pointer_growth_translates_scanned_go_allocator_rounding() {
        assert_eq!(go127_pointer_growth_capacity(0, 1).unwrap(), 1);
        assert_eq!(go127_pointer_growth_capacity(2, 3).unwrap(), 4);
        assert_eq!(go127_pointer_growth_capacity(32, 33).unwrap(), 64);
        assert_eq!(go127_pointer_growth_capacity(64, 65).unwrap(), 143);
        assert_eq!(go127_pointer_growth_capacity(256, 257).unwrap(), 607);
        assert_eq!(go127_pointer_growth_capacity(4096, 4097).unwrap(), 6144);
        let old = SharedNodeSlice::with_len_cap(64, 64).unwrap();
        let grown = old.append_node(NodeId(1)).unwrap();
        assert_eq!((grown.len(), grown.capacity()), (65, 143));
        assert_eq!(grown.get(64), Some(n(1)));
        assert!(grown.capacity_slots()[65..].iter().all(Option::is_none));
        assert!(go127_pointer_growth_capacity(0, usize::MAX).is_err());
    }

    #[test]
    fn real_shared_allocations_support_send_sync_and_joined_read_workers() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SharedNodeSlice>();
        assert_send_sync::<SharedSliceMap>();
        let map = SharedSliceMap::<Option<NodeId>>::new();
        map.replace(None, SharedNodeSlice::from_slots(&[n(1), n(2)]).unwrap())
            .unwrap();
        std::thread::scope(|scope| {
            let view = &map;
            scope.spawn(move || assert_eq!(view.get(&None).unwrap().visible_slots(), [n(1), n(2)]));
            let view = &map;
            scope.spawn(move || assert_eq!(view.get(&None).unwrap().capacity(), 2));
        });
        map.get(&None)
            .unwrap()
            .write_capacity_slot(0, n(7))
            .unwrap();
        assert_eq!(map.get(&None).unwrap().visible_slots(), [n(7), n(2)]);
    }
}

#[cfg(test)]
mod copied_node_contract {
    use super::*;
    #[test]
    fn copied_window_keeps_duplicates_and_observes_alias_writes_and_replacement() {
        let map = ContainerMap::new();
        let backing =
            SharedNodeSlice::from_nodes(&[NodeId(9), NodeId(1), NodeId(1), NodeId(3), NodeId(8)])
                .unwrap();
        let window = backing.reslice(1, 4).unwrap();
        map.replace_header(None, window.clone());
        let prior = map.get(&None).unwrap();
        assert_eq!(prior, vec![NodeId(1), NodeId(1), NodeId(3)]);
        // edge_length.go:907-910 loads the pointer after the item poll.
        // Taking a reader must not snapshot the value before an alias write.
        let mut readers = window.borrowed_slot_readers().readers();
        assert_eq!(readers.len(), 3);
        assert_eq!(readers.next().unwrap().get(), Some(NodeId(1)));
        let second = readers.next().unwrap();
        backing.write_capacity_slot(2, None).unwrap();
        assert_eq!(second.get(), None);
        backing.write_capacity_slot(2, Some(NodeId(7))).unwrap();
        assert_eq!(second.get(), Some(NodeId(7)));
        assert_eq!(
            map.get(&None).unwrap(),
            vec![NodeId(1), NodeId(7), NodeId(3)]
        );
        map.replace_header(None, SharedNodeSlice::from_nodes(&[NodeId(4)]).unwrap());
        assert_eq!(map.get(&None).unwrap(), vec![NodeId(4)]);
        assert_eq!(window.copied_nodes(), vec![NodeId(1), NodeId(7), NodeId(3)]);
        assert_eq!(prior, vec![NodeId(1), NodeId(1), NodeId(3)]);
        assert_eq!(readers.next().unwrap().get(), Some(NodeId(3)));
        assert!(
            readers.next().is_none(),
            "spare capacity is outside the header"
        );
    }
    #[test]
    fn nil_recovery_does_not_poison_backing_and_empty_entries_stay_present() {
        let map = ContainerMap::new();
        assert!(map.get(&None).is_none());
        for header in [SharedNodeSlice::nil(), SharedNodeSlice::allocated_empty()] {
            map.replace_header(None, header);
            assert_eq!(map.get(&None), Some(vec![]));
        }
        let header = SharedNodeSlice::from_slots(&[Some(NodeId(1)), None]).unwrap();
        map.replace_header(None, header.clone());
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| map.get(&None))).is_err());
        header.write_capacity_slot(1, Some(NodeId(2))).unwrap();
        assert_eq!(map.get(&None), Some(vec![NodeId(1), NodeId(2)]));
        assert_eq!(
            map.iter().collect::<Vec<_>>(),
            vec![(None, vec![NodeId(1), NodeId(2)])]
        );
    }
}
