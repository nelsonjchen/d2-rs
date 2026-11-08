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

//! Nullable shared pointer slices for raw edges and abductions. Header copies retain
//! backing; append/remove and exact capacity-window restoration follow the
//! same pinned Go pointer policy as container_slices.rs. Entity IDs keep their
//! types; an edge identity is never encoded as a node identity.

use super::container_slices::{
    ContainerSliceError, check_pointer_capacity, go127_pointer_growth_capacity,
};
use super::owner_points::SharedOwnerPoint;
use super::{EdgeId, OwnerAbductionId};
use crate::internal::go_cell::GoCell;
use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, OnceLock};

pub(super) trait OwnerPointer: Clone + Eq + fmt::Debug + Send + Sync + 'static {
    type Slots: SlotStore<Self>;
    fn zero_backing() -> Arc<PointerSliceBacking<Self>>;
}

/// Backing storage for Go pointer slots. Writers run only on the layout
/// thread; rayon route workers only read. Bulk operations default to
/// per-slot loads and stores; a locked store overrides them to take one lock.
pub(super) trait SlotStore<T>: Send + Sync + Sized {
    fn nil_slots(len: usize) -> Result<Self, ContainerSliceError>;
    fn len(&self) -> usize;
    fn load(&self, index: usize) -> Option<T>;
    fn store(&self, index: usize, value: Option<T>);
    /// Visit slots in order until `visit` returns true; report whether it did.
    fn scan(&self, range: Range<usize>, visit: &mut dyn FnMut(&Option<T>) -> bool) -> bool {
        range.into_iter().any(|index| visit(&self.load(index)))
    }
    /// `scan` over the whole range with a statically dispatched visitor: the
    /// same slot reads (and, for a locked store, the same single lock).
    fn for_each_in(&self, range: Range<usize>, mut visit: impl FnMut(&Option<T>)) {
        for index in range {
            visit(&self.load(index));
        }
    }
    fn copy_out(&self, range: Range<usize>) -> Vec<Option<T>> {
        range.map(|index| self.load(index)).collect()
    }
    fn copy_in(&self, start: usize, values: &[Option<T>])
    where
        T: Clone,
    {
        for (offset, value) in values.iter().enumerate() {
            self.store(start + offset, value.clone());
        }
    }
    fn fill_nil(&self, range: Range<usize>) {
        for index in range {
            self.store(index, None);
        }
    }
    /// Go's append(prefix, suffix...) removal: shift the tail left over the
    /// first match. Returns whether a slot matched.
    fn remove_first(&self, range: Range<usize>, value: &Option<T>) -> bool
    where
        T: Eq,
    {
        let Some(index) = range.clone().position(|index| self.load(index) == *value) else {
            return false;
        };
        for slot in range.start + index..range.end - 1 {
            self.store(slot, self.load(slot + 1));
        }
        true
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    fn slot_ptr(&self, index: usize) -> usize;
    /// Count of slot writes so far. Every mutation of a slot advances it, so
    /// an unchanged count means unchanged slot contents.
    fn writes(&self) -> u64;
}

/// Integer pointer identities, held as relaxed atomics (plain ldr/str on
/// arm64). A slot holds `id + 1`; 0 is a nil pointer.
pub(super) trait AtomicPointer: Copy {
    fn to_slot(self) -> u64;
    fn from_slot(slot: u64) -> Self;
}
impl AtomicPointer for EdgeId {
    fn to_slot(self) -> u64 {
        u64::from(self.0) + 1
    }
    fn from_slot(slot: u64) -> Self {
        EdgeId((slot - 1) as u32)
    }
}
impl AtomicPointer for OwnerAbductionId {
    fn to_slot(self) -> u64 {
        (self.0 as u64)
            .checked_add(1)
            .expect("abduction id fits a slot")
    }
    fn from_slot(slot: u64) -> Self {
        OwnerAbductionId((slot - 1) as usize)
    }
}

pub(super) struct AtomicSlots(Box<[AtomicU64]>, AtomicU64);
impl<T: AtomicPointer + Send + Sync> SlotStore<T> for AtomicSlots {
    fn nil_slots(len: usize) -> Result<Self, ContainerSliceError> {
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(len)
            .map_err(|error| ContainerSliceError::Allocation(error.to_string()))?;
        slots.extend((0..len).map(|_| AtomicU64::new(0)));
        Ok(Self(slots.into_boxed_slice(), AtomicU64::new(0)))
    }
    fn writes(&self) -> u64 {
        self.1.load(Relaxed)
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn load(&self, index: usize) -> Option<T> {
        match self.0[index].load(Relaxed) {
            0 => None,
            slot => Some(T::from_slot(slot)),
        }
    }
    fn for_each_in(&self, range: Range<usize>, mut visit: impl FnMut(&Option<T>)) {
        // Go ranges over the captured backing window. Borrow that window
        // once, but load each pointer only when reached so alias writes to
        // later slots stay visible to the visitor.
        for slot in &self.0[range] {
            let value = match slot.load(Relaxed) {
                0 => None,
                value => Some(T::from_slot(value)),
            };
            visit(&value);
        }
    }
    fn store(&self, index: usize, value: Option<T>) {
        self.0[index].store(value.map_or(0, T::to_slot), Relaxed);
        self.1.fetch_add(1, Relaxed);
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    fn slot_ptr(&self, index: usize) -> usize {
        self.0.as_ptr().wrapping_add(index) as usize
    }
}

/// Pointer objects that are themselves shared allocations stay behind one
/// lock, taken once per bulk operation.
pub(super) struct LockedSlots<T>(GoCell<Box<[Option<T>]>>, AtomicU64);
impl<T: Clone + Eq + Send + Sync> SlotStore<T> for LockedSlots<T> {
    fn nil_slots(len: usize) -> Result<Self, ContainerSliceError> {
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(len)
            .map_err(|error| ContainerSliceError::Allocation(error.to_string()))?;
        slots.resize(len, None);
        Ok(Self(
            GoCell::new(slots.into_boxed_slice()),
            AtomicU64::new(0),
        ))
    }
    fn writes(&self) -> u64 {
        self.1.load(Relaxed)
    }
    fn len(&self) -> usize {
        self.read().len()
    }
    fn load(&self, index: usize) -> Option<T> {
        self.read()[index].clone()
    }
    fn store(&self, index: usize, value: Option<T>) {
        self.write()[index] = value;
    }
    fn scan(&self, range: Range<usize>, visit: &mut dyn FnMut(&Option<T>) -> bool) -> bool {
        self.read()[range].iter().any(visit)
    }
    fn for_each_in(&self, range: Range<usize>, mut visit: impl FnMut(&Option<T>)) {
        for slot in &self.read()[range] {
            visit(slot);
        }
    }
    fn copy_out(&self, range: Range<usize>) -> Vec<Option<T>> {
        self.read()[range].to_vec()
    }
    // Empty windows return before borrowing: the zero-capacity backing is one
    // process-wide allocation shared by every graph and thread.
    fn copy_in(&self, start: usize, values: &[Option<T>]) {
        if values.is_empty() {
            return;
        }
        self.write()[start..start + values.len()].clone_from_slice(values);
    }
    fn fill_nil(&self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        self.write()[range].fill(None);
    }
    fn remove_first(&self, range: Range<usize>, value: &Option<T>) -> bool {
        if range.is_empty() {
            return false;
        }
        let mut slots = self.write();
        let Some(index) = slots[range.clone()].iter().position(|slot| slot == value) else {
            return false;
        };
        for slot in range.start + index..range.end - 1 {
            slots[slot] = slots[slot + 1].clone();
        }
        true
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    fn slot_ptr(&self, index: usize) -> usize {
        self.read().as_ptr().wrapping_add(index) as usize
    }
}
impl<T> LockedSlots<T> {
    fn read(&self) -> crate::internal::go_cell::GoCellRef<'_, Box<[Option<T>]>> {
        self.0.read().expect("owner pointer backing lock poisoned")
    }
    fn write(&self) -> crate::internal::go_cell::GoCellMut<'_, Box<[Option<T>]>> {
        // Every mutable borrow counts as a write.
        self.1.fetch_add(1, Relaxed);
        self.0.write().expect("owner pointer backing lock poisoned")
    }
}

impl OwnerPointer for EdgeId {
    type Slots = AtomicSlots;
    fn zero_backing() -> Arc<PointerSliceBacking<Self>> {
        static ZERO: OnceLock<Arc<PointerSliceBacking<EdgeId>>> = OnceLock::new();
        Arc::clone(ZERO.get_or_init(|| {
            Arc::new(PointerSliceBacking {
                slots: SlotStore::<EdgeId>::nil_slots(1).expect("sentinel slot"),
                non_nil_window: Default::default(),
            })
        }))
    }
}
impl OwnerPointer for SharedOwnerPoint {
    type Slots = LockedSlots<SharedOwnerPoint>;
    fn zero_backing() -> Arc<PointerSliceBacking<Self>> {
        static ZERO: OnceLock<Arc<PointerSliceBacking<SharedOwnerPoint>>> = OnceLock::new();
        Arc::clone(ZERO.get_or_init(|| {
            Arc::new(PointerSliceBacking {
                slots: SlotStore::<SharedOwnerPoint>::nil_slots(0).expect("sentinel slot"),
                non_nil_window: Default::default(),
            })
        }))
    }
}
pub(super) type SharedPointSlice = SharedOwnerPointerSlice<SharedOwnerPoint>;
pub(super) type SharedPointSliceSnapshot = SharedOwnerPointerSliceSnapshot<SharedOwnerPoint>;

pub(super) type SharedEdgeSlice = SharedOwnerPointerSlice<EdgeId>;
pub(super) type SharedEdgeSliceSnapshot = SharedOwnerPointerSliceSnapshot<EdgeId>;
impl OwnerPointer for OwnerAbductionId {
    type Slots = AtomicSlots;
    fn zero_backing() -> Arc<PointerSliceBacking<Self>> {
        static ZERO: OnceLock<Arc<PointerSliceBacking<OwnerAbductionId>>> = OnceLock::new();
        Arc::clone(ZERO.get_or_init(|| {
            Arc::new(PointerSliceBacking {
                slots: SlotStore::<OwnerAbductionId>::nil_slots(1).expect("sentinel slot"),
                non_nil_window: Default::default(),
            })
        }))
    }
}
pub(super) type SharedAbductionSlice = SharedOwnerPointerSlice<OwnerAbductionId>;
pub(super) struct PointerSliceBacking<T: OwnerPointer> {
    // Every slot is initialized; no read ever enters Vec's uninitialized
    // spare capacity. The fixed Box allocation never grows or moves.
    slots: T::Slots,
    /// The last window `assert_values_non_nil` found free of nil slots, as
    /// (slot write count, start, len). While the count is unchanged those
    /// slots are unchanged, so the window needs no rescan.
    non_nil_window: std::sync::Mutex<Option<(u64, usize, usize)>>,
}

fn allocated_zero_backing<T: OwnerPointer>() -> Arc<PointerSliceBacking<T>> {
    T::zero_backing()
}

fn allocate_backing<T: OwnerPointer>(
    capacity: usize,
) -> Result<Arc<PointerSliceBacking<T>>, ContainerSliceError> {
    if capacity == 0 {
        return Ok(allocated_zero_backing());
    }
    check_pointer_capacity(capacity)?;
    Ok(Arc::new(PointerSliceBacking {
        slots: T::Slots::nil_slots(capacity)?,
        non_nil_window: Default::default(),
    }))
}

/// Clone copies a header and aliases its actual backing allocation. It never
/// clones the backing values. Default is a genuine nil slice.
#[derive(Clone)]
pub(super) struct SharedOwnerPointerSlice<T: OwnerPointer> {
    backing: Option<Arc<PointerSliceBacking<T>>>,
    start: usize,
    len: usize,
    cap: usize,
}

// Callback-free coordinate reads do not retain a pointer allocation. Keep
// nullable-slot and bounds distinctions while borrowing the existing backing.
impl SharedPointSlice {
    pub(super) fn coordinate_slot(&self, index: usize) -> Option<Option<super::Point>> {
        if index >= self.len {
            return None;
        }
        let backing = self.backing.as_ref().expect("positive length has backing");
        let slots = backing.slots.read();
        Some(
            slots[self.start + index]
                .as_ref()
                .map(SharedOwnerPoint::get),
        )
    }
}

impl<T: OwnerPointer> fmt::Debug for SharedOwnerPointerSlice<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedOwnerPointerSlice<T>")
            .field("nil", &self.is_nil())
            .field("start", &self.start)
            .field("len", &self.len)
            .field("cap", &self.cap)
            .finish()
    }
}

impl<T: OwnerPointer> SharedOwnerPointerSlice<T> {
    pub(super) fn nil() -> Self {
        Self::default()
    }

    pub(super) fn allocated_empty() -> Self {
        Self {
            backing: Some(allocated_zero_backing()),
            ..Self::default()
        }
    }

    /// Source make([]*Pointer, len, cap), whose logical cap is exact even when
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

    pub(super) fn from_slots(slots: &[Option<T>]) -> Result<Self, ContainerSliceError> {
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

    pub(super) fn from_values(nodes: &[T]) -> Result<Self, ContainerSliceError> {
        let result = Self::with_len_cap(nodes.len(), nodes.len())?;
        if !nodes.is_empty() {
            let values = nodes.iter().cloned().map(Some).collect::<Vec<_>>();
            result
                .backing
                .as_ref()
                .expect("allocated slice")
                .slots
                .copy_in(0, &values);
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
    pub(super) fn iter_slots(&self) -> impl Iterator<Item = Option<T>> + use<T> {
        let header = self.clone();
        (0..header.len).map(move |index| header.get(index).expect("captured slice bound"))
    }

    /// Borrow an already retained header, reading each live slot when reached.
    /// No backing guard survives a next() call, so aliases may change slots.
    pub(super) fn borrowed_slots(&self) -> impl Iterator<Item = Option<T>> + '_ {
        (0..self.len).map(|index| self.get(index).expect("borrowed slice bound"))
    }

    // Only for walks that cannot call back into a writer of this backing.
    // Visits the visible slots in order without copying the header.
    pub(super) fn for_each_visible(&self, visit: impl FnMut(&Option<T>)) {
        if let Some(backing) = &self.backing {
            backing
                .slots
                .for_each_in(self.start..self.start + self.len, visit);
        }
    }

    pub(super) fn require_values(&self) -> impl Iterator<Item = T> + use<T> {
        self.iter_slots()
            .map(|slot| slot.expect("nil owner pointer slot"))
    }

    /// None is out of bounds; Some(None) is an actual nil pointer slot.
    pub(super) fn get(&self, index: usize) -> Option<Option<T>> {
        (index < self.len)
            .then(|| self.read_capacity_slot(index))
            .flatten()
    }

    pub(super) fn read_capacity_slot(&self, index: usize) -> Option<Option<T>> {
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
        value: Option<T>,
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

    /// Preserve the value reader's raw-pointer precondition without a copy.
    pub(super) fn assert_values_non_nil(&self) {
        let Some(backing) = &self.backing else { return };
        // A window verified since the backing's last slot write cannot hold
        // a nil now; only a changed backing is rescanned (and panics alike).
        let writes = backing.slots.writes();
        let window = Some((writes, self.start, self.len));
        let mut verified = backing
            .non_nil_window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *verified == window {
            return;
        }
        backing
            .slots
            .scan(self.start..self.start + self.len, &mut |slot| {
                slot.as_ref().expect("nil owner pointer");
                false
            });
        *verified = window;
    }

    /// Ordinary value consumers hold one read lock for their transient image.
    /// Callback-visible source range walks continue using iter_slots instead.
    pub(super) fn read_values(&self) -> Vec<T> {
        let Some(backing) = &self.backing else {
            return Vec::new();
        };
        let mut values = Vec::with_capacity(self.len);
        backing
            .slots
            .scan(self.start..self.start + self.len, &mut |slot| {
                values.push(slot.clone().expect("nil owner pointer"));
                false
            });
        values
    }
    /// `read_values` for a consumer that only reduces the values: visit each
    /// one in order without collecting a transient image.
    pub(super) fn for_each_value(&self, mut visit: impl FnMut(&T)) {
        let Some(backing) = &self.backing else {
            return;
        };
        // Node.deltaToGuarded ranges directly over Node.Edges. This walk
        // cannot stop early, so keep its visitor statically dispatched as
        // well as its slot store, just like for_each_visible.
        backing
            .slots
            .for_each_in(self.start..self.start + self.len, |slot| {
                visit(slot.as_ref().expect("nil owner pointer"));
            });
    }
    pub(super) fn contains_value(&self, value: &T) -> bool {
        let Some(backing) = &self.backing else {
            return false;
        };
        backing
            .slots
            .scan(self.start..self.start + self.len, &mut |slot| {
                slot.as_ref() == Some(value)
            })
    }

    /// Filter a transient value image against this live header with one
    /// backing read. No caller callbacks execute while the lock is held.
    pub(super) fn retain_members(&self, values: &mut Vec<T>) {
        let Some(backing) = &self.backing else {
            values.clear();
            return;
        };
        let visible = self.start..self.start + self.len;
        values.retain(|value| {
            backing
                .slots
                .scan(visible.clone(), &mut |slot| slot.as_ref() == Some(value))
        });
    }

    pub(super) fn visible_slots(&self) -> Vec<Option<T>> {
        self.slot_window(self.len)
    }
    pub(super) fn capacity_slots(&self) -> Vec<Option<T>> {
        self.slot_window(self.cap)
    }

    fn slot_window(&self, length: usize) -> Vec<Option<T>> {
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
    pub(super) fn append(&self, values: &[Option<T>]) -> Result<Self, ContainerSliceError> {
        self.append_with_buffer(values, 0)
    }
    /// codec.go local edge inventory uses growsliceBuf(...,4), then moveSlice.
    pub(super) fn from_go127_buffered_appends(values: &[T]) -> Result<Self, ContainerSliceError> {
        let mut header = Self::allocated_empty();
        for value in values {
            header = header.append_with_buffer(&[Some(value.clone())], 4)?;
        }
        Ok(header)
    }
    fn append_with_buffer(
        &self,
        values: &[Option<T>],
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

    pub(super) fn append_value(&self, node: T) -> Result<Self, ContainerSliceError> {
        self.append(&[Some(node)])
    }

    /// node.go1072–1080 uses append(prefix, suffix...), not slices.Delete:
    /// remove only the first match, shift in place, retain the stale tail.
    pub(super) fn remove_first_slot(&self, value: Option<T>) -> Self {
        if self.len == 0 {
            return self.clone();
        }
        let removed = self
            .backing
            .as_ref()
            .expect("positive length has backing")
            .slots
            .remove_first(self.start..self.start + self.len, &value);
        if !removed {
            return self.clone();
        }
        Self {
            len: self.len - 1,
            ..self.clone()
        }
    }

    pub(super) fn clear_visible(&self) {
        if let Some(backing) = &self.backing {
            backing.slots.fill_nil(self.start..self.start + self.len);
        }
    }

    /// Independent graph cloning uses make(len), including a nonnil empty
    /// result for nil/empty input. It is distinct from header Clone above.
    pub(super) fn clone_allocated(&self) -> Result<Self, ContainerSliceError> {
        Self::from_slots(&self.visible_slots())
    }

    /// Detach one header, including its complete backing and stale slots.
    /// A single allocation needs no batch alias table or temporary slot images.
    pub(super) fn fork_storage(&self) -> Result<Self, ContainerSliceError> {
        let backing = if let Some(source) = &self.backing {
            let copied = allocate_backing::<T>(source.slots.len())?;
            for index in 0..source.slots.len() {
                copied.slots.store(index, source.slots.load(index));
            }
            Some(copied)
        } else {
            None
        };
        Ok(Self {
            backing,
            start: self.start,
            len: self.len,
            cap: self.cap,
        })
    }

    /// `*self = source.fork_storage()?` into this header's own backing when
    /// it is uniquely owned and as long as the source backing. False means
    /// nothing was changed; the shared zero backing is never written.
    pub(super) fn fork_storage_into(&mut self, source: &Self) -> bool {
        let (Some(target), Some(from)) = (self.backing.as_mut(), source.backing.as_ref()) else {
            return false;
        };
        let Some(target) = Arc::get_mut(target) else {
            return false;
        };
        let len = from.slots.len();
        if len == 0 || target.slots.len() != len {
            return false;
        }
        for index in 0..len {
            target.slots.store(index, from.slots.load(index));
        }
        self.start = source.start;
        self.len = source.len;
        self.cap = source.cap;
        true
    }

    /// Independent numerical candidates retain source header shape and all
    /// stale backing slots. Fork the complete inventory together so overlapping
    /// member headers still alias within the candidate, without sharing writes
    /// with its parent. This is separate from copying a live range header.
    pub(super) fn fork_storage_batch(headers: &[Self]) -> Result<Vec<Self>, ContainerSliceError> {
        Self::fork_storage_batch_map(headers, Clone::clone)
    }

    /// Detach backing once per actual allocation while mapping pointer objects.
    /// Route points use this to preserve point aliases across different slices
    /// and Node.TopLeft in a numerical graph clone.
    pub(super) fn fork_storage_batch_map(
        headers: &[Self],
        mut map_pointer: impl FnMut(&T) -> T,
    ) -> Result<Vec<Self>, ContainerSliceError> {
        // Each original Arc remains alive in headers for this entire walk;
        // its actual allocation address is only a local deduplication key.
        let mut backings: BTreeMap<usize, Arc<PointerSliceBacking<T>>> = BTreeMap::new();
        headers
            .iter()
            .map(|header| {
                let backing = if let Some(source) = &header.backing {
                    let address = Arc::as_ptr(source) as usize;
                    if let Some(existing) = backings.get(&address) {
                        Some(Arc::clone(existing))
                    } else {
                        // Only numerical pointer remapping occurs here; it cannot
                        // replace source slots or invoke layout callbacks.
                        let mut values = Vec::with_capacity(source.slots.len());
                        source.slots.scan(0..source.slots.len(), &mut |slot| {
                            values.push(slot.as_ref().map(&mut map_pointer));
                            false
                        });
                        let copied = allocate_backing::<T>(values.len())?;
                        SlotStore::<T>::copy_in(&copied.slots, 0, &values);
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

    pub(super) fn snapshot(&self) -> SharedOwnerPointerSliceSnapshot<T> {
        SharedOwnerPointerSliceSnapshot {
            original: self.clone(),
            capacity_values: self.capacity_slots(),
        }
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
        Some(backing.slots.slot_ptr(self.start))
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn slot_address(&self, index: usize) -> Option<usize> {
        (index < self.cap)
            .then(|| {
                self.backing
                    .as_ref()
                    .map(|backing| backing.slots.slot_ptr(self.start + index))
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
pub(super) struct SharedOwnerPointerSliceSnapshot<T: OwnerPointer> {
    original: SharedOwnerPointerSlice<T>,
    capacity_values: Vec<Option<T>>,
}

impl<T: OwnerPointer> SharedOwnerPointerSliceSnapshot<T> {
    /// Repair exactly [start,start+cap), never the entire Box. Writes through
    /// aliases outside every captured header's window must survive rollback.
    pub(super) fn restore(&self) -> SharedOwnerPointerSlice<T> {
        if let Some(backing) = &self.original.backing {
            backing
                .slots
                .copy_in(self.original.start, &self.capacity_values);
        }
        self.original.clone()
    }
}

impl<T: OwnerPointer> Default for SharedOwnerPointerSlice<T> {
    fn default() -> Self {
        Self {
            backing: None,
            start: 0,
            len: 0,
            cap: 0,
        }
    }
}

#[cfg(test)]
#[path = "owner_incidents_tests.rs"]
mod tests;
