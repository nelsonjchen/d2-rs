// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Indexed routing queue translated from D2 v0.9.0
//! `internal/routing/priority_queue.go`.
//!
//! The queue is deliberately source-shaped: it is an indexed binary min-heap,
//! equal priorities use a separate insertion counter, and entry storage is
//! retained across searches. The `push`/`pop`/`decrease` names mirror the Go
//! implementation; the `*_guarded` methods expose its complete v0.9.0
//! work-accounting contract for routing integration and qualification.

use std::fmt;

const ENTRY_CHUNK_SIZE: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
// u32 keeps route searches' per-node handle arrays small. A queue holds
// one entry per push, far below u32::MAX in any OVG that fits in memory.
pub(super) struct Handle(u32);

/// A pushed value and its heap position (`NOT_QUEUED` once popped). The
/// priority lives only in the entry's heap slot: push and decrease write
/// both copies and a sift moves a slot whole, so the two always agreed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry<T> {
    value: T,
    // Heap positions are below the entry count, which `Handle` bounds by
    // u32, so they are stored compactly.
    index: u32,
}

/// The heap position of an entry that is not in the heap.
const NOT_QUEUED: u32 = u32::MAX;

/// A heap position as an entry stores it; `NOT_QUEUED` is reserved.
fn heap_position(index: usize) -> u32 {
    u32::try_from(index)
        .ok()
        .filter(|&index| index != NOT_QUEUED)
        .expect("priority queue heap position")
}

/// Errors returned by the source-shaped priority queue.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum PriorityQueueError<E = &'static str> {
    /// The requested operation would exceed the caller's work budget.
    Work(E),
    /// A dequeue was requested without an entry.
    Empty,
    /// A handle does not identify an active entry.
    MissingEntry,
    /// The replacement priority is not strictly smaller.
    NotDecreased { old: f64, new: f64 },
}

impl<E: fmt::Display> fmt::Display for PriorityQueueError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Work(error) => error.fmt(f),
            Self::Empty => f.write_str("cannot dequeue minimum of empty priority queue"),
            Self::MissingEntry => {
                f.write_str("cannot decrease priority: entry is not in the priority queue")
            }
            Self::NotDecreased { old, new } => {
                write!(f, "new priority {new} is not less than old priority {old}")
            }
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for PriorityQueueError<E> {}

/// A heap slot carries its entry's comparison key, so sifting compares
/// adjacent slots instead of dereferencing two entries, as Go's `[]*entry`.
/// Entries are allocated in insertion order and the insertion counter
/// resets with them, so an entry's handle is its insertion order: the
/// equal-priority tie-break compares handles.
#[derive(Clone, Copy, Debug)]
struct Slot {
    priority: f64,
    handle: u32,
}

pub(super) struct PriorityQueue<T> {
    /// Entries in heap order. The entries themselves live in reusable storage.
    items: Vec<Slot>,
    entries: Vec<Entry<T>>,
    entry_cursor: usize,
}

impl<T: Copy> PriorityQueue<T> {
    pub(super) fn new() -> Self {
        Self {
            items: Vec::new(),
            entries: Vec::new(),
            entry_cursor: 0,
        }
    }

    /// Empty the queue while retaining heap and entry storage.
    ///
    /// Handles returned before a reset must not be used afterward. This is
    /// the same lifetime rule as the Go pointer entries and permits the next
    /// search to reuse the previously allocated chunks.
    pub(super) fn reset(&mut self) {
        self.items.clear();
        self.entry_cursor = 0;
    }

    pub(super) fn empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of active entries in the heap.  This is used only by the
    /// diagnostic routing trace; keeping it as a direct accessor avoids
    /// exposing the heap's storage or changing release behavior.
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    /// Source-shaped `priorityQueue.push` without a work budget.
    pub(super) fn push(&mut self, priority: f64, value: T) -> Handle {
        self.push_inner(priority, value)
    }

    /// Compatibility spelling retained for the first Rust router bridge.
    pub(super) fn enqueue(&mut self, priority: f64, value: T) -> Handle {
        self.push(priority, value)
    }

    /// Push after reserving the exact v0.9.0 heap-sift work bound.
    pub(super) fn enqueue_guarded<E>(
        &mut self,
        priority: f64,
        value: T,
        reserve: impl FnOnce(u64) -> Result<(), E>,
    ) -> Result<Handle, PriorityQueueError<E>> {
        reserve(priority_queue_work(self.items.len() + 1, 1)).map_err(PriorityQueueError::Work)?;
        Ok(self.push_inner(priority, value))
    }

    /// Source-shaped guarded spelling used by the routing work-budget seam.
    pub(super) fn push_guarded<E>(
        &mut self,
        priority: f64,
        value: T,
        reserve: impl FnOnce(u64) -> Result<(), E>,
    ) -> Result<Handle, PriorityQueueError<E>> {
        self.enqueue_guarded(priority, value, reserve)
    }

    /// Source-shaped `priorityQueue.pop` without a work budget.
    pub(super) fn pop(&mut self) -> Option<(T, f64)> {
        self.pop_inner().ok().map(EntryValue::into_parts)
    }

    /// Compatibility spelling retained for the first Rust router bridge.
    pub(super) fn dequeue_min(&mut self) -> Option<(T, f64)> {
        self.pop()
    }

    /// Pop after reserving the exact v0.9.0 heap-sift work bound.
    pub(super) fn dequeue_min_guarded<E>(
        &mut self,
        reserve: impl FnOnce(u64) -> Result<(), E>,
    ) -> Result<EntryValue<T>, PriorityQueueError<E>> {
        if self.empty() {
            return Err(PriorityQueueError::Empty);
        }
        reserve(priority_queue_work(self.items.len(), 2)).map_err(PriorityQueueError::Work)?;
        self.pop_inner().map_err(|_| PriorityQueueError::Empty)
    }

    /// Source-shaped guarded spelling used by the routing work-budget seam.
    pub(super) fn pop_guarded<E>(
        &mut self,
        reserve: impl FnOnce(u64) -> Result<(), E>,
    ) -> Result<EntryValue<T>, PriorityQueueError<E>> {
        self.dequeue_min_guarded(reserve)
    }

    /// Source-shaped `priorityQueue.decrease` without a work budget.
    pub(super) fn decrease(&mut self, handle: Handle, priority: f64) -> Result<(), &'static str> {
        self.decrease_key_guarded(handle, priority, |_| Ok::<(), &'static str>(()))
            .map_err(|error| match error {
                PriorityQueueError::Work(error) => error,
                PriorityQueueError::Empty => "cannot decrease priority in an empty priority queue",
                PriorityQueueError::MissingEntry => {
                    "cannot decrease priority: entry is not in the priority queue"
                }
                PriorityQueueError::NotDecreased { .. } => {
                    "new priority is not less than old priority"
                }
            })
    }

    /// Compatibility spelling retained for the first Rust router bridge.
    pub(super) fn decrease_key(
        &mut self,
        handle: Handle,
        priority: f64,
    ) -> Result<(), &'static str> {
        self.decrease(handle, priority)
    }

    /// Lower a queued priority after validating the handle and reserving work.
    pub(super) fn decrease_key_guarded<E>(
        &mut self,
        handle: Handle,
        priority: f64,
        reserve: impl FnOnce(u64) -> Result<(), E>,
    ) -> Result<(), PriorityQueueError<E>> {
        if self.empty() {
            return Err(PriorityQueueError::Empty);
        }
        let (index, old_priority) = self
            .entry(handle)
            .filter(|entry| entry.index != NOT_QUEUED)
            .map(|entry| entry.index as usize)
            .and_then(|index| {
                let slot = self.items.get(index)?;
                (slot.handle == handle.0).then_some((index, slot.priority))
            })
            .ok_or(PriorityQueueError::MissingEntry)?;
        if priority >= old_priority {
            return Err(PriorityQueueError::NotDecreased {
                old: old_priority,
                new: priority,
            });
        }
        reserve(priority_queue_work(index + 1, 1)).map_err(PriorityQueueError::Work)?;
        self.items[index].priority = priority;
        self.sift_up(index);
        Ok(())
    }

    /// Source-shaped guarded spelling used by the routing work-budget seam.
    pub(super) fn decrease_guarded<E>(
        &mut self,
        handle: Handle,
        priority: f64,
        reserve: impl FnOnce(u64) -> Result<(), E>,
    ) -> Result<(), PriorityQueueError<E>> {
        self.decrease_key_guarded(handle, priority, reserve)
    }

    fn push_inner(&mut self, priority: f64, value: T) -> Handle {
        let handle = self.allocate_entry(value);
        let index = self.items.len();
        let entry = self.entry_mut(handle).expect("new queue entry");
        entry.index = heap_position(index);
        let slot = Slot {
            priority,
            handle: handle.0,
        };
        self.items.push(slot);
        self.sift_up(index);
        handle
    }

    fn pop_inner(&mut self) -> Result<EntryValue<T>, ()> {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(QueuePop);
        let Slot {
            priority,
            handle: first,
        } = *self.items.first().ok_or(())?;
        let last = self.items.pop().expect("nonempty queue");
        self.entry_mut(Handle(first)).expect("heap handle").index = NOT_QUEUED;
        if !self.items.is_empty() {
            // sift_down publishes the moved slot's entry index.
            self.items[0] = last;
            self.sift_down(0);
        }
        let entry = *self.entry(Handle(first)).expect("popped queue entry");
        Ok(EntryValue {
            value: entry.value,
            priority,
        })
    }

    fn allocate_entry(&mut self, value: T) -> Handle {
        let handle = Handle(u32::try_from(self.entry_cursor).expect("priority queue handle"));
        let entry = Entry {
            value,
            index: NOT_QUEUED,
        };
        if self.entry_cursor == self.entries.len() {
            if self.entries.len() == self.entries.capacity() {
                self.entries.reserve(ENTRY_CHUNK_SIZE);
            }
            self.entries.push(entry);
        } else {
            self.entries[self.entry_cursor] = entry;
        }
        self.entry_cursor += 1;
        handle
    }

    fn entry(&self, Handle(handle): Handle) -> Option<&Entry<T>> {
        self.entries.get(handle as usize)
    }

    fn entry_mut(&mut self, Handle(handle): Handle) -> Option<&mut Entry<T>> {
        self.entries.get_mut(handle as usize)
    }

    /// Sifting carries the moving slot in a hole and writes each displaced
    /// slot (and its entry index) once. The visited path, comparisons and
    /// final heap and entry indices are those of the swap-based sift.
    fn sift_up(&mut self, mut index: usize) {
        let slot = self.items[index];
        while index > 0 {
            let parent = (index - 1) / 2;
            let parent_slot = self.items[parent];
            if !slot_less(&slot, &parent_slot) {
                break;
            }
            self.items[index] = parent_slot;
            self.entries[parent_slot.handle as usize].index = heap_position(index);
            index = parent;
        }
        self.items[index] = slot;
        self.entries[slot.handle as usize].index = heap_position(index);
    }

    fn sift_down(&mut self, mut index: usize) {
        let len = self.items.len();
        let slot = self.items[index];
        loop {
            let left = index * 2 + 1;
            if left >= len {
                break;
            }
            let right = left + 1;
            let smallest = if right < len && slot_less(&self.items[right], &self.items[left]) {
                right
            } else {
                left
            };
            let child = self.items[smallest];
            if !slot_less(&child, &slot) {
                break;
            }
            self.items[index] = child;
            self.entries[child.handle as usize].index = heap_position(index);
            index = smallest;
        }
        self.items[index] = slot;
        self.entries[slot.handle as usize].index = heap_position(index);
    }
}

#[inline]
fn slot_less(a: &Slot, b: &Slot) -> bool {
    if a.priority != b.priority {
        a.priority < b.priority
    } else {
        a.handle < b.handle
    }
}

/// The value returned by a guarded dequeue, including its priority.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct EntryValue<T> {
    value: T,
    priority: f64,
}

impl<T: Copy> EntryValue<T> {
    pub(super) fn into_parts(self) -> (T, f64) {
        (self.value, self.priority)
    }
}

fn priority_queue_work(upper_bound: usize, units_per_level: u64) -> u64 {
    (usize::BITS - upper_bound.leading_zeros()) as u64 * units_per_level
}

#[cfg(test)]
mod tests {
    use super::{ENTRY_CHUNK_SIZE, PriorityQueue, PriorityQueueError};

    #[test]
    fn upstream_decrease_and_equal_priority_order() {
        let mut queue = PriorityQueue::new();
        queue.enqueue(1.0, 0);
        let second = queue.enqueue(2.0, 1);
        queue.enqueue(1.0, 2);
        queue.decrease_key(second, 1.0).unwrap();
        for value in 0..3 {
            assert_eq!(queue.dequeue_min(), Some((value, 1.0)));
        }
        assert!(queue.dequeue_min().is_none());
        assert!(queue.decrease_key(second, 0.0).is_err());
    }

    #[test]
    fn upstream_mixed_priorities_across_chunk_boundary() {
        let mut queue = PriorityQueue::new();
        let mut expected = Vec::new();
        let mut handles = Vec::new();
        for i in 0..(ENTRY_CHUNK_SIZE * 2 + 1) {
            let priority = ((i * 37) % 100) as f64;
            handles.push(queue.enqueue(priority, i));
            expected.push((i, priority));
        }
        for i in (0..expected.len()).step_by(17) {
            let priority = -((i + 1) as f64);
            queue.decrease_key(handles[i], priority).unwrap();
            expected[i].1 = priority;
        }
        expected.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap().then(a.0.cmp(&b.0)));
        for pair in expected {
            assert_eq!(queue.dequeue_min(), Some(pair));
        }
    }

    #[test]
    fn invalid_decrease_preserves_queue() {
        let mut queue = PriorityQueue::new();
        let first = queue.enqueue(1.0, 1);
        queue.enqueue(2.0, 2);
        assert!(queue.decrease_key(first, 1.0).is_err());
        assert_eq!(queue.dequeue_min(), Some((1, 1.0)));
        assert!(queue.decrease_key(first, 0.0).is_err());
        assert_eq!(queue.dequeue_min(), Some((2, 2.0)));
    }

    #[test]
    fn empty_pop_and_decrease_match_upstream_errors() {
        let mut queue: PriorityQueue<i32> = PriorityQueue::new();
        assert_eq!(
            queue.pop_guarded(|_| Ok::<(), ()>(())).unwrap_err(),
            PriorityQueueError::Empty
        );
        assert_eq!(
            queue
                .decrease_guarded(super::Handle(0), 0.0, |_| Ok::<(), ()>(()))
                .unwrap_err(),
            PriorityQueueError::Empty
        );
    }

    #[test]
    fn guarded_operations_reserve_before_mutation() {
        let mut queue = PriorityQueue::new();
        let rejected = queue
            .push_guarded(1.0, 1, |_| Err::<(), _>("reject"))
            .unwrap_err();
        assert_eq!(rejected, PriorityQueueError::Work("reject"));
        assert!(queue.empty());

        let first = queue.enqueue(1.0, 1);
        let second = queue.enqueue(2.0, 2);
        let rejected = queue.pop_guarded(|_| Err::<(), _>("reject")).unwrap_err();
        assert_eq!(rejected, PriorityQueueError::Work("reject"));
        assert_eq!(
            queue.items.iter().map(|slot| slot.handle).collect::<Vec<_>>(),
            vec![first.0, second.0]
        );
        let rejected = queue
            .decrease_guarded(first, 0.0, |_| Err::<(), _>("reject"))
            .unwrap_err();
        assert_eq!(rejected, PriorityQueueError::Work("reject"));
        assert_eq!(queue.dequeue_min(), Some((1, 1.0)));
    }

    #[test]
    fn reset_reuses_entry_handles_and_order() {
        let mut queue = PriorityQueue::new();
        let first = queue.enqueue(1.0, 1);
        queue.reset();
        let second = queue.enqueue(2.0, 2);
        assert_eq!(first, second);
        assert_eq!(queue.dequeue_min(), Some((2, 2.0)));
    }

    #[test]
    fn guarded_dequeue_returns_value_and_priority() {
        let mut queue = PriorityQueue::new();
        queue.enqueue(3.0, 7);
        let value = queue
            .pop_guarded(|units| {
                assert_eq!(units, 2);
                Ok::<(), ()>(())
            })
            .unwrap()
            .into_parts();
        assert_eq!(value, (7, 3.0));
    }
}
