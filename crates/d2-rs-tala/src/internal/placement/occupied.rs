// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Source sizelessOptimizer.occupied: a shared map keyed by exact geo.Point
//! values. Capturing its rollback value does not itself arm restoration.

use super::limits::{OptimizationError, OptimizationWorkGuard};
use super::{NodeId, Point};
use crate::internal::go_cell::GoCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ComparablePointKey(u64, u64);

impl ComparablePointKey {
    fn new(point: Point) -> Option<Self> {
        // Go float equality never finds a NaN key, including the identical
        // payload. Both signed zeros compare equal; infinities remain keys.
        if point.x.is_nan() || point.y.is_nan() {
            return None;
        }
        let bits = |value: f64| if value == 0.0 { 0 } else { value.to_bits() };
        Some(Self(bits(point.x), bits(point.y)))
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    point: Point,
    node: Option<NodeId>,
}

#[derive(Debug, Default)]
struct MapState {
    slots: Vec<Option<Entry>>,
    free: Vec<usize>,
    comparable: HashMap<ComparablePointKey, usize>,
    len: usize,
}

impl MapState {
    fn insert(&mut self, entry: Entry) {
        let key = ComparablePointKey::new(entry.point);
        if let Some(index) = key.and_then(|key| self.comparable.get(&key).copied()) {
            // Go NeedKeyUpdate overwrites the stored key as well as its value.
            // Updating +0 with -0 retains one key and the latest input bits.
            self.slots[index] = Some(entry);
            return;
        }
        let index = if let Some(index) = self.free.pop() {
            self.slots[index] = Some(entry);
            index
        } else {
            self.slots.push(Some(entry));
            self.slots.len() - 1
        };
        if let Some(key) = key {
            self.comparable.insert(key, index);
        }
        self.len += 1;
    }

    fn lookup(&self, point: Point) -> Option<Option<NodeId>> {
        let key = ComparablePointKey::new(point)?;
        let index = *self.comparable.get(&key)?;
        self.slots[index].map(|entry| entry.node)
    }

    fn delete(&mut self, point: Point) {
        let Some(key) = ComparablePointKey::new(point) else {
            return;
        };
        let Some(index) = self.comparable.remove(&key) else {
            return;
        };
        self.slots[index] = None;
        self.free.push(index);
        self.len -= 1;
    }

    fn clear(&mut self) {
        self.slots.fill(None);
        self.free = (0..self.slots.len()).rev().collect();
        self.comparable.clear();
        self.len = 0;
    }
}

/// Clone is a shallow map-reference copy. Default is the source nil map;
/// new() allocates a distinct, nonnil empty map.
#[derive(Clone, Debug, Default)]
pub(super) struct OccupiedPointMap {
    state: Option<Arc<GoCell<MapState>>>,
}

#[cfg(feature = "diagnostic-traces")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OccupiedCapturePhase {
    BeforeStep,
    AfterCopy,
    Complete,
}

// Both ordinary and owner-aware capture use this one cursor. Diagnostic
// observations and their operands are absent from ordinary builds.
trait OccupiedCaptureDriver {
    fn step(&mut self) -> Result<(), OptimizationError>;

    #[cfg(feature = "diagnostic-traces")]
    fn observe(
        &self,
        _phase: OccupiedCapturePhase,
        _entry: Option<Entry>,
        _original: &OccupiedPointMap,
        _copied: &OccupiedPointMap,
    ) {
    }
}

struct ClosureCaptureDriver<F>(F);
impl<F: FnMut() -> Result<(), OptimizationError>> OccupiedCaptureDriver
    for ClosureCaptureDriver<F>
{
    fn step(&mut self) -> Result<(), OptimizationError> {
        (self.0)()
    }
}

#[cfg(feature = "diagnostic-traces")]
struct OwnerCaptureDriver<'a, 'context> {
    guard: &'a mut OptimizationWorkGuard<'context>,
    owner: &'a mut crate::internal::ArenaGraph,
}

#[cfg(feature = "diagnostic-traces")]
impl OccupiedCaptureDriver for OwnerCaptureDriver<'_, '_> {
    fn step(&mut self) -> Result<(), OptimizationError> {
        self.guard.step_with_owner(self.owner)
    }

    fn observe(
        &self,
        phase: OccupiedCapturePhase,
        entry: Option<Entry>,
        original: &OccupiedPointMap,
        copied: &OccupiedPointMap,
    ) {
        self.guard.observe_occupied_capture(
            self.owner,
            phase,
            entry.map(|entry| entry.point),
            entry.and_then(|entry| entry.node),
            original,
            copied,
        );
    }
}

impl OccupiedPointMap {
    pub(super) fn new() -> Self {
        Self {
            state: Some(Arc::new(GoCell::new(MapState::default()))),
        }
    }

    pub(super) fn is_nil(&self) -> bool {
        self.state.is_none()
    }

    pub(super) fn len(&self) -> usize {
        self.state.as_ref().map_or(0, |state| {
            state.read().expect("occupied map lock poisoned").len
        })
    }

    pub(super) fn lookup(&self, point: Point) -> Option<Option<NodeId>> {
        self.state
            .as_ref()?
            .read()
            .expect("occupied map lock poisoned")
            .lookup(point)
    }

    pub(super) fn insert(&self, point: Point, node: Option<NodeId>) {
        let state = self.state.as_ref().expect("assignment to entry in nil map");
        state
            .write()
            .expect("occupied map lock poisoned")
            .insert(Entry { point, node });
    }

    pub(super) fn delete(&self, point: Point) {
        if let Some(state) = &self.state {
            state
                .write()
                .expect("occupied map lock poisoned")
                .delete(point);
        }
    }

    pub(super) fn clear(&self) {
        if let Some(state) = &self.state {
            state.write().expect("occupied map lock poisoned").clear();
        }
    }

    pub(super) fn same_map_allocation(&self, other: &Self) -> bool {
        match (&self.state, &other.state) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// sizeless_optimizer.go122–129 loads each yielded pair before Step and
    /// copies it only after an accepted charge. The cursor retains the map
    /// allocation, reads future slots live and holds no lock across Context.
    /// Map order is unspecified. Newly inserted entries may be skipped; no
    /// comparable key is yielded twice after removal/reinsertion.
    pub(super) fn capture(
        &self,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<OccupiedPointSnapshot, OptimizationError> {
        self.capture_with_step(|| guard.step())
    }

    /// The owner transaction supplies the actual accepted Step boundary so
    /// diagnostic contexts can borrow its genuine mutable arena at the poll.
    /// The copied pair remains loaded before that Step and inserted afterward.
    pub(super) fn capture_with_step(
        &self,
        step: impl FnMut() -> Result<(), OptimizationError>,
    ) -> Result<OccupiedPointSnapshot, OptimizationError> {
        self.capture_with_driver(&mut ClosureCaptureDriver(step))
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn capture_with_owner(
        &self,
        guard: &mut OptimizationWorkGuard<'_>,
        owner: &mut crate::internal::ArenaGraph,
    ) -> Result<OccupiedPointSnapshot, OptimizationError> {
        self.capture_with_driver(&mut OwnerCaptureDriver { guard, owner })
    }

    fn capture_with_driver(
        &self,
        driver: &mut impl OccupiedCaptureDriver,
    ) -> Result<OccupiedPointSnapshot, OptimizationError> {
        // Source allocates a nonnil copied map even when the original is nil.
        // Its shared storage survives actual diagnostic observer aliases.
        let copied = Self::new();
        if let Some(state) = &self.state {
            let end = state
                .read()
                .expect("occupied map lock poisoned")
                .slots
                .len();
            let mut yielded = HashSet::new();
            for index in 0..end {
                let entry = state
                    .read()
                    .expect("occupied map lock poisoned")
                    .slots
                    .get(index)
                    .copied()
                    .flatten();
                let Some(entry) = entry else {
                    continue;
                };
                if let Some(key) = ComparablePointKey::new(entry.point)
                    && !yielded.insert(key)
                {
                    continue;
                }
                #[cfg(feature = "diagnostic-traces")]
                driver.observe(OccupiedCapturePhase::BeforeStep, Some(entry), self, &copied);
                driver.step()?;
                copied.insert(entry.point, entry.node);
                #[cfg(feature = "diagnostic-traces")]
                driver.observe(OccupiedCapturePhase::AfterCopy, Some(entry), self, &copied);
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        driver.observe(OccupiedCapturePhase::Complete, None, self, &copied);
        Ok(OccupiedPointSnapshot {
            original: self.clone(),
            copied,
        })
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn observed_entries(&self) -> Vec<(Point, Option<NodeId>)> {
        self.state.as_ref().map_or_else(Vec::new, |state| {
            state
                .read()
                .expect("occupied map lock poisoned")
                .slots
                .iter()
                .flatten()
                .map(|entry| (entry.point, entry.node))
                .collect()
        })
    }

    /// Diagnostic control's second real range selects one currently unvisited
    /// reflexive key. No complete inventory or future yielded order is copied.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn find_unvisited_point(&self, seen: &[Point]) -> Option<Point> {
        let state = self
            .state
            .as_ref()?
            .read()
            .expect("occupied map lock poisoned");
        state.slots.iter().flatten().find_map(|entry| {
            let point = entry.point;
            (!point.x.is_nan()
                && !point.y.is_nan()
                && !seen
                    .iter()
                    .any(|seen| seen.x == point.x && seen.y == point.y))
            .then_some(point)
        })
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn map_address(&self) -> Option<usize> {
        self.state.as_ref().map(|state| Arc::as_ptr(state) as usize)
    }
}

/// Inert until the enclosing complete graph and occupied captures arm their
/// rollback. Ordinary Drop preserves mutations and replacement map fields.
#[derive(Debug)]
pub(super) struct OccupiedPointSnapshot {
    original: OccupiedPointMap,
    copied: OccupiedPointMap,
}

impl OccupiedPointSnapshot {
    pub(super) fn restore(&self, target: &mut OccupiedPointMap) {
        if let Some(state) = &self.original.state {
            let mut state = state.write().expect("occupied map lock poisoned");
            state.clear();
            let copied = self
                .copied
                .state
                .as_ref()
                .expect("occupied snapshot always has an allocated copy")
                .read()
                .expect("occupied map lock poisoned");
            for entry in copied.slots.iter().flatten() {
                state.insert(*entry);
            }
        }
        *target = self.original.clone();
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn observed_entries(&self) -> Vec<(Point, Option<NodeId>)> {
        self.copied.observed_entries()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::{Cell, RefCell};

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    struct QuietContext;
    impl Context for QuietContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            None
        }
    }

    #[test]
    fn exact_float_keys_preserve_fractions_infinities_and_latest_zero_bits() {
        let map = OccupiedPointMap::new();
        map.insert(point(0.25, 0.5), Some(NodeId(1)));
        map.insert(point(0.4, 0.5), Some(NodeId(2)));
        map.insert(point(f64::INFINITY, f64::NEG_INFINITY), Some(NodeId(3)));
        map.insert(point(0.0, 1.0), Some(NodeId(4)));
        map.insert(point(-0.0, 1.0), Some(NodeId(5)));
        assert_eq!(map.len(), 4);
        assert_eq!(map.lookup(point(0.25, 0.5)), Some(Some(NodeId(1))));
        assert_eq!(map.lookup(point(0.4, 0.5)), Some(Some(NodeId(2))));
        assert_eq!(
            map.lookup(point(f64::INFINITY, f64::NEG_INFINITY)),
            Some(Some(NodeId(3)))
        );
        assert_eq!(map.lookup(point(0.0, 1.0)), Some(Some(NodeId(5))));
        let key = map
            .observed_entries()
            .into_iter()
            .find(|(_, node)| *node == Some(NodeId(5)))
            .unwrap()
            .0;
        assert_eq!(key.x.to_bits(), (-0.0f64).to_bits());
        map.delete(point(0.0, 1.0));
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn present_nil_value_is_distinct_from_absence_and_captured_without_rejection() {
        let original = OccupiedPointMap::new();
        let key = point(1.25, 2.5);
        original.insert(key, None);
        assert_eq!(original.len(), 1);
        assert_eq!(original.lookup(key), Some(None));
        assert_eq!(original.lookup(point(4.0, 5.0)), None);
        let mut guard =
            OptimizationWorkGuard::new(Some(&QuietContext), "sizelessOptimizer.optimize", 1)
                .unwrap();
        let snapshot = original.capture(&mut guard).unwrap();
        assert_eq!(guard.used(), 1);
        assert_eq!(snapshot.observed_entries(), vec![(key, None)]);
        original.insert(key, Some(NodeId(1)));
        let mut target = OccupiedPointMap::default();
        snapshot.restore(&mut target);
        assert!(target.same_map_allocation(&original));
        assert_eq!(original.lookup(key), Some(None));
        original.delete(key);
        assert_eq!(original.len(), 0);
        assert_eq!(original.lookup(key), None);
    }

    #[test]
    fn nan_keys_are_nonreflexive_and_survive_explicit_snapshot_restoration() {
        let map = OccupiedPointMap::new();
        let key = point(f64::from_bits(0x7ff8_1234_5678_9abc), 4.0);
        map.insert(key, Some(NodeId(1)));
        map.insert(key, Some(NodeId(2)));
        assert_eq!(map.len(), 2);
        assert_eq!(map.lookup(key), None);
        map.delete(key);
        assert_eq!(map.len(), 2);
        let mut guard =
            OptimizationWorkGuard::new(Some(&QuietContext), "sizelessOptimizer.optimize", 2)
                .unwrap();
        let snapshot = map.capture(&mut guard).unwrap();
        assert_eq!(guard.used(), 2);
        map.clear();
        let mut target = OccupiedPointMap::new();
        snapshot.restore(&mut target);
        assert!(target.same_map_allocation(&map));
        assert_eq!(map.len(), 2);
        for (actual, _) in map.observed_entries() {
            assert_eq!(actual.x.to_bits(), key.x.to_bits());
        }
        assert_eq!(map.lookup(key), None);
    }

    #[test]
    fn explicit_restore_preserves_original_alias_and_nil_empty_distinction() {
        let original = OccupiedPointMap::new();
        let alias = original.clone();
        let address = alias.map_address();
        original.insert(point(1.25, 2.5), Some(NodeId(1)));
        let mut guard =
            OptimizationWorkGuard::new(Some(&QuietContext), "sizelessOptimizer.optimize", 4)
                .unwrap();
        let snapshot = original.capture(&mut guard).unwrap();
        alias.insert(point(1.25, 2.5), Some(NodeId(2)));
        alias.insert(point(4.0, 5.0), Some(NodeId(3)));
        let replacement = OccupiedPointMap::new();
        replacement.insert(point(10.0, 20.0), Some(NodeId(4)));
        let mut target = replacement.clone();
        snapshot.restore(&mut target);
        assert_eq!(target.map_address(), address);
        assert_eq!(alias.lookup(point(1.25, 2.5)), Some(Some(NodeId(1))));
        assert_eq!(alias.lookup(point(4.0, 5.0)), None);
        assert_eq!(replacement.lookup(point(10.0, 20.0)), Some(Some(NodeId(4))));

        let empty = OccupiedPointMap::new();
        let empty_snapshot = empty.capture(&mut guard).unwrap();
        target = OccupiedPointMap::default();
        empty_snapshot.restore(&mut target);
        assert!(!target.is_nil() && target.same_map_allocation(&empty));
        let nil_snapshot = OccupiedPointMap::default().capture(&mut guard).unwrap();
        nil_snapshot.restore(&mut target);
        assert!(target.is_nil());
        assert_eq!(guard.used(), 1);
    }

    struct MutateAtSecondPoll {
        map: OccupiedPointMap,
        polls: Cell<usize>,
        reject: bool,
    }

    impl Context for MutateAtSecondPoll {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            if poll == 2 {
                // Slot63's key/value has already been loaded. Slot64 remains
                // live and the last key is deleted before it can be yielded.
                self.map.insert(point(63.0, 0.0), Some(NodeId(6300)));
                self.map.insert(point(64.0, 0.0), Some(NodeId(6400)));
                self.map.delete(point(65.0, 0.0));
                if self.reject {
                    return Some(ContextError::Cancelled);
                }
            }
            None
        }
    }

    fn populated_map() -> OccupiedPointMap {
        let map = OccupiedPointMap::new();
        for index in 0..66 {
            map.insert(point(index as f64, 0.0), Some(NodeId(index)));
        }
        map
    }

    #[test]
    fn real_poll_reads_live_future_entries_and_keeps_current_loaded_value() {
        let map = populated_map();
        let context = MutateAtSecondPoll {
            map: map.clone(),
            polls: Cell::new(0),
            reject: false,
        };
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 100).unwrap();
        let snapshot = map.capture(&mut guard).unwrap();
        assert_eq!(context.polls.get(), 2);
        assert_eq!(guard.used(), 65);
        let captured = snapshot.observed_entries();
        assert_eq!(captured.len(), 65);
        assert!(
            captured
                .iter()
                .any(|(key, value)| key.x == 63.0 && *value == Some(NodeId(63)))
        );
        assert!(
            captured
                .iter()
                .any(|(key, value)| key.x == 64.0 && *value == Some(NodeId(6400)))
        );
        assert!(!captured.iter().any(|(key, _)| key.x == 65.0));
        let mut target = OccupiedPointMap::default();
        snapshot.restore(&mut target);
        assert_eq!(map.lookup(point(63.0, 0.0)), Some(Some(NodeId(63))));
        assert_eq!(map.lookup(point(64.0, 0.0)), Some(Some(NodeId(6400))));
        assert_eq!(map.lookup(point(65.0, 0.0)), None);
    }

    struct ReplaceMapAtSecondPoll {
        current: RefCell<OccupiedPointMap>,
        replacement: OccupiedPointMap,
        polls: Cell<usize>,
    }

    impl Context for ReplaceMapAtSecondPoll {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            let poll = self.polls.get() + 1;
            self.polls.set(poll);
            if poll == 2 {
                self.current.replace(self.replacement.clone());
            }
            None
        }
    }

    #[test]
    fn real_poll_map_field_replacement_keeps_original_capture_and_restore_ref() {
        let original = populated_map();
        let replacement = OccupiedPointMap::new();
        replacement.insert(point(1000.0, 2000.0), None);
        let context = ReplaceMapAtSecondPoll {
            current: RefCell::new(original.clone()),
            replacement: replacement.clone(),
            polls: Cell::new(0),
        };
        let retained = context.current.borrow().clone();
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 100).unwrap();
        let snapshot = retained.capture(&mut guard).unwrap();
        assert_eq!(context.polls.get(), 2);
        assert_eq!(guard.used(), 66);
        assert_eq!(snapshot.observed_entries().len(), 66);
        assert!(context.current.borrow().same_map_allocation(&replacement));
        assert!(!context.current.borrow().same_map_allocation(&original));
        let mut target = context.current.into_inner();
        snapshot.restore(&mut target);
        assert!(target.same_map_allocation(&original));
        assert_eq!(target.lookup(point(63.0, 0.0)), Some(Some(NodeId(63))));
        assert_eq!(target.lookup(point(1000.0, 2000.0)), None);
        assert_eq!(replacement.lookup(point(1000.0, 2000.0)), Some(None));
    }

    #[test]
    fn incomplete_capture_and_unarmed_drop_preserve_context_and_alias_edits() {
        let map = populated_map();
        let context = MutateAtSecondPoll {
            map: map.clone(),
            polls: Cell::new(0),
            reject: true,
        };
        let mut guard =
            OptimizationWorkGuard::new(Some(&context), "sizelessOptimizer.optimize", 100).unwrap();
        assert!(map.capture(&mut guard).is_err());
        assert_eq!(guard.used(), 64);
        assert_eq!(context.polls.get(), 2);
        assert_eq!(map.lookup(point(63.0, 0.0)), Some(Some(NodeId(6300))));
        assert_eq!(map.lookup(point(64.0, 0.0)), Some(Some(NodeId(6400))));
        assert_eq!(map.lookup(point(65.0, 0.0)), None);

        let mut guard =
            OptimizationWorkGuard::new(Some(&QuietContext), "sizelessOptimizer.optimize", 100)
                .unwrap();
        let snapshot = map.capture(&mut guard).unwrap();
        map.insert(point(1.0, 0.0), Some(NodeId(999)));
        drop(snapshot);
        assert_eq!(map.lookup(point(1.0, 0.0)), Some(Some(NodeId(999))));
    }

    #[cfg(feature = "diagnostic-traces")]
    #[test]
    fn actual_yield_observer_retains_live_copy_and_loaded_pair_across_real_poll() {
        struct LiveObserver {
            reject: bool,
            polls: Cell<usize>,
            before: Cell<usize>,
            after: Cell<usize>,
            complete: Cell<usize>,
            current: RefCell<Option<(Point, Option<NodeId>)>>,
            aliases: RefCell<Option<(OccupiedPointMap, OccupiedPointMap)>>,
        }
        impl Context for LiveObserver {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                panic!("owner-aware capture must use the actual owner poll")
            }
            fn error_with_owner(
                &self,
                _owner: &mut crate::internal::ArenaGraph,
            ) -> Option<ContextError> {
                self.polls.set(self.polls.get() + 1);
                if self.polls.get() == 2 {
                    let (key, node) = self.current.borrow().unwrap();
                    assert_eq!(key, point(63.0, 0.0));
                    assert_eq!(node, Some(NodeId(63)));
                    let aliases = self.aliases.borrow();
                    let (original, copied) = aliases.as_ref().unwrap();
                    assert_eq!(copied.len(), 63);
                    original.insert(key, Some(NodeId(6300)));
                    original.insert(point(64.0, 0.0), Some(NodeId(6400)));
                    return self.reject.then_some(ContextError::Cancelled);
                }
                None
            }
            fn occupied_capture_state(
                &self,
                _owner: &crate::internal::ArenaGraph,
                phase: OccupiedCapturePhase,
                key: Option<Point>,
                node: Option<NodeId>,
                original: &OccupiedPointMap,
                copied: &OccupiedPointMap,
            ) {
                assert!(!copied.is_nil());
                assert!(!original.same_map_allocation(copied));
                let mut aliases = self.aliases.borrow_mut();
                if let Some((old_original, old_copied)) = aliases.as_ref() {
                    assert!(old_original.same_map_allocation(original));
                    assert!(old_copied.same_map_allocation(copied));
                } else {
                    *aliases = Some((original.clone(), copied.clone()));
                }
                match phase {
                    OccupiedCapturePhase::BeforeStep => {
                        self.before.set(self.before.get() + 1);
                        *self.current.borrow_mut() = Some((key.unwrap(), node));
                        assert_eq!(copied.len(), self.after.get());
                    }
                    OccupiedCapturePhase::AfterCopy => {
                        self.after.set(self.after.get() + 1);
                        assert_eq!(copied.lookup(key.unwrap()), Some(node));
                        assert_eq!(copied.len(), self.after.get());
                    }
                    OccupiedCapturePhase::Complete => {
                        self.complete.set(self.complete.get() + 1);
                        assert!(key.is_none() && node.is_none());
                    }
                }
            }
        }
        for reject in [false, true] {
            let map = populated_map();
            let context = LiveObserver {
                reject,
                polls: Cell::new(0),
                before: Cell::new(0),
                after: Cell::new(0),
                complete: Cell::new(0),
                current: RefCell::new(None),
                aliases: RefCell::new(None),
            };
            let mut owner = crate::internal::ArenaGraph::from_input(&crate::Graph::default());
            let mut guard = OptimizationWorkGuard::new_with_owner(
                Some(&context),
                "sizelessOptimizer.optimize",
                100,
                &mut owner,
            )
            .unwrap();
            let result = map.capture_with_owner(&mut guard, &mut owner);
            assert_eq!(context.polls.get(), 2);
            let aliases = context.aliases.borrow();
            let (_, copied) = aliases.as_ref().unwrap();
            if reject {
                assert!(result.is_err());
                assert_eq!(guard.used(), 64);
                assert_eq!((context.before.get(), context.after.get()), (64, 63));
                assert_eq!(context.complete.get(), 0);
                assert_eq!(copied.lookup(point(63.0, 0.0)), None);
                assert_eq!(map.lookup(point(63.0, 0.0)), Some(Some(NodeId(6300))));
            } else {
                assert_eq!(guard.used(), 66);
                assert_eq!((context.before.get(), context.after.get()), (66, 66));
                assert_eq!(context.complete.get(), 1);
                assert_eq!(copied.lookup(point(63.0, 0.0)), Some(Some(NodeId(63))));
                assert_eq!(copied.lookup(point(64.0, 0.0)), Some(Some(NodeId(6400))));
                let snapshot = result.unwrap();
                map.clear();
                let mut target = OccupiedPointMap::new();
                snapshot.restore(&mut target);
                assert!(target.same_map_allocation(&map));
                assert_eq!(map.lookup(point(63.0, 0.0)), Some(Some(NodeId(63))));
                assert_eq!(map.lookup(point(64.0, 0.0)), Some(Some(NodeId(6400))));
                // The callback's genuine copied-map alias survives restoration.
                assert_eq!(copied.len(), 66);
            }
        }
    }
}
