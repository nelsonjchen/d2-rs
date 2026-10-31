// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Go []*OVGEdge headers: copies retain backing, not a frozen list of slots.
use super::ovg_objects::ObjectRoot;
use std::sync::{Arc, Weak};
use crate::internal::go_cell::{GoCell};

/// A slot value and its backing-array form. Slots never retain an owner
/// root (a header supplies it on each read), so the stored form can omit
/// that handle.
pub(super) trait SlotValue: Clone {
    type Stored: Clone + std::fmt::Debug;
    fn retained(stored: &Self::Stored, owner: Option<Arc<ObjectRoot>>) -> Self;
    fn stored(&self) -> Self::Stored;
    /// `stored` for a value the caller gives up, without a handle copy.
    fn into_stored(self) -> Self::Stored {
        self.stored()
    }
}
impl SlotValue for usize {
    type Stored = usize;
    fn retained(stored: &usize, _owner: Option<Arc<ObjectRoot>>) -> Self {
        *stored
    }
    fn stored(&self) -> usize {
        *self
    }
}
#[derive(Debug)]
pub(super) struct EdgeHeader<T: SlotValue = usize> {
    backing: Option<Arc<GoCell<Box<[Option<T::Stored>]>>>>,
    owner: Weak<ObjectRoot>,
    retained_owner: Option<Arc<ObjectRoot>>,
    // Header counts are OVG edge-slot counts; u32 keeps the header (and so
    // every OVG node object) a word smaller.
    len: u32,
    capacity: u32,
}
impl<T: SlotValue> Default for EdgeHeader<T> {
    fn default() -> Self {
        Self {
            backing: None,
            owner: Weak::new(),
            retained_owner: None,
            len: 0,
            capacity: 0,
        }
    }
}
impl<T: SlotValue> Clone for EdgeHeader<T> {
    fn clone(&self) -> Self {
        Self {
            backing: self.backing.clone(),
            owner: self.owner.clone(),
            retained_owner: self.retained_owner.clone().or_else(|| self.owner.upgrade()),
            len: self.len,
            capacity: self.capacity,
        }
    }
}
impl<T: SlotValue> EdgeHeader<T> {
    pub(super) fn owner_root(&self) -> Option<Arc<ObjectRoot>> {
        self.retained_owner.clone().or_else(|| self.owner.upgrade())
    }
    pub(super) fn set_owner(&mut self, owner: Weak<ObjectRoot>) {
        self.owner = owner;
        self.retained_owner = None;
    }

    pub(super) fn len(&self) -> usize {
        self.len as usize
    }
    pub(super) fn capacity(&self) -> usize {
        self.capacity as usize
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(super) fn is_nil(&self) -> bool {
        self.backing.is_none()
    }
    pub(super) fn get(&self, index: usize) -> Option<T> {
        assert!(index < self.len(), "OVG edge slice index out of range");
        self.backing
            .as_ref()
            .unwrap()
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[index]
            .as_ref()
            .map(|value| {
                T::retained(value, self.retained_owner.clone().or_else(|| self.owner.upgrade()))
            })
    }
    pub(super) fn push(&mut self, edge: T) {
        if self.len == self.capacity {
            let capacity = crate::internal::container_slices::go127_pointer_growth_capacity(
                self.capacity(),
                self.len() + 1,
            )
            .expect("OVG edge pointer slice capacity");
            let mut slots = vec![None; capacity].into_boxed_slice();
            // A retained copy's stored form is the slot itself; copy straight
            // from one borrow of the old backing.
            self.with_slots(|old| {
                for (slot, value) in slots.iter_mut().zip(old) {
                    slot.clone_from(value);
                }
            });
            self.backing = Some(Arc::new(GoCell::new(slots)));
            self.capacity = u32::try_from(capacity).expect("OVG edge pointer slice capacity");
        }
        self.backing
            .as_ref()
            .unwrap()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[self.len()] = Some(edge.into_stored());
        self.len += 1;
    }
    /// Borrow slots for a walk that neither mutates the backing nor invokes
    /// caller callbacks. The caller keeps the owning OVG node/root alive.
    pub(super) fn with_slots<R>(&self, read: impl FnOnce(&[Option<T::Stored>]) -> R) -> R {
        match &self.backing {
            Some(backing) => {
                let slots = backing
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                read(&slots[..self.len()])
            }
            None => read(&[]),
        }
    }

    pub(super) fn iter_slots(&self) -> impl Iterator<Item = Option<T>> + '_ {
        (0..self.len()).map(|index| self.get(index))
    }
    #[cfg(test)]
    pub(super) fn set(&self, index: usize, edge: Option<T>) {
        assert!(index < self.len());
        self.backing
            .as_ref()
            .unwrap()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[index] = edge.map(|v| v.stored());
    }
}
impl<T: SlotValue + serde::Serialize> serde::Serialize for EdgeHeader<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.iter_slots().collect::<Vec<_>>().serialize(serializer)
    }
}
impl<T: SlotValue + PartialEq<usize>, const N: usize> PartialEq<[usize; N]> for EdgeHeader<T> {
    fn eq(&self, other: &[usize; N]) -> bool {
        self.len() == N
            && self
                .iter_slots()
                .zip(other)
                .all(|(a, b)| a.is_some_and(|a| a == *b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_range_reads_live_slots_and_keeps_its_length_after_append() {
        let mut current = EdgeHeader::default();
        current.push(10);
        current.push(20);
        current.push(30);
        let captured = current.clone();
        assert_eq!(captured.iter_slots().next(), Some(Some(10)));
        current.set(1, Some(99));
        assert_eq!(captured.iter_slots().nth(1), Some(Some(99)));
        current.push(40);
        assert_eq!(captured.len(), 3);
        assert_eq!(current.len(), 4);
        // Growth replaces the current backing, while the captured header keeps
        // the earlier allocation and remains independently writable.
        let old = current.clone();
        current.push(50);
        current.set(0, None);
        assert_eq!(old.iter_slots().next(), Some(Some(10)));
        assert_eq!(current.iter_slots().next(), Some(None));
        assert!(!captured.is_nil());
    }
}
