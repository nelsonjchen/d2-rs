// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Interior-mutable cell for the shared Go pointer fields of the graph model.
//!
//! The recovered Go code reads and writes these fields through plain shared
//! pointers. Its parallel phases (seed workers, EdgeLength scoring, route
//! flavors) only read shared fields or write state they own, so no access
//! needs a lock. `GoCell` mirrors that: release builds use an unsynchronized
//! `UnsafeCell`. Tests, debug builds and the `checked-cells` feature track
//! borrows atomically and panic on any conflicting access, so the corpus run
//! of a checked build is what establishes that no read races a write.
//! Fields that are genuinely written concurrently keep a real lock.
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::LockResult;

#[cfg(any(test, debug_assertions, feature = "checked-cells"))]
use std::sync::atomic::{AtomicIsize, Ordering};

#[derive(Default)]
pub(crate) struct GoCell<T: ?Sized> {
    /// Readers when positive, -1 while written.
    #[cfg(any(test, debug_assertions, feature = "checked-cells"))]
    borrows: AtomicIsize,
    value: UnsafeCell<T>,
}

// SAFETY: see the module documentation; the checked build enforces that no
// access conflicts across threads (or within one).
unsafe impl<T: ?Sized + Send> Send for GoCell<T> {}
unsafe impl<T: ?Sized + Send + Sync> Sync for GoCell<T> {}
// Like std's RwLock: a panic mid-write leaves a value readers may observe.
impl<T: ?Sized> std::panic::UnwindSafe for GoCell<T> {}
impl<T: ?Sized> std::panic::RefUnwindSafe for GoCell<T> {}

impl<T> GoCell<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self {
            #[cfg(any(test, debug_assertions, feature = "checked-cells"))]
            borrows: AtomicIsize::new(0),
            value: UnsafeCell::new(value),
        }
    }
    pub(crate) fn into_inner(self) -> LockResult<T> {
        Ok(self.value.into_inner())
    }
}

impl<T: ?Sized> GoCell<T> {
    /// Shared access. Returns `LockResult` so call sites keep the
    /// `std::sync::RwLock` shape.
    pub(crate) fn read(&self) -> LockResult<GoCellRef<'_, T>> {
        #[cfg(any(test, debug_assertions, feature = "checked-cells"))]
        if self.borrows.fetch_add(1, Ordering::Acquire) < 0 {
            panic!("GoCell read while written: {}", std::any::type_name::<T>());
        }
        // SAFETY: no writer is active (checked builds verify it).
        Ok(GoCellRef {
            cell: self,
            value: unsafe { &*self.value.get() },
        })
    }
    pub(crate) fn write(&self) -> LockResult<GoCellMut<'_, T>> {
        #[cfg(any(test, debug_assertions, feature = "checked-cells"))]
        if self
            .borrows
            .compare_exchange(0, -1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            panic!("GoCell written while borrowed: {}", std::any::type_name::<T>());
        }
        // SAFETY: no other access is active (checked builds verify it).
        Ok(GoCellMut {
            cell: self,
            value: unsafe { &mut *self.value.get() },
        })
    }
    pub(crate) fn get_mut(&mut self) -> LockResult<&mut T> {
        Ok(self.value.get_mut())
    }
}

impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for GoCell<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Like std's RwLock, report rather than conflict with a writer.
        #[cfg(any(test, debug_assertions, feature = "checked-cells"))]
        if self.borrows.load(Ordering::Acquire) < 0 {
            return f.write_str("GoCell(<written>)");
        }
        let value = self.read().expect("GoCell");
        f.debug_tuple("GoCell").field(&&*value).finish()
    }
}

pub(crate) struct GoCellRef<'a, T: ?Sized> {
    #[cfg_attr(not(any(test, debug_assertions, feature = "checked-cells")), allow(dead_code))]
    cell: &'a GoCell<T>,
    value: &'a T,
}
pub(crate) struct GoCellMut<'a, T: ?Sized> {
    #[cfg_attr(not(any(test, debug_assertions, feature = "checked-cells")), allow(dead_code))]
    cell: &'a GoCell<T>,
    value: &'a mut T,
}

#[cfg(any(test, debug_assertions, feature = "checked-cells"))]
impl<T: ?Sized> Drop for GoCellRef<'_, T> {
    fn drop(&mut self) {
        self.cell.borrows.fetch_sub(1, Ordering::Release);
    }
}
#[cfg(any(test, debug_assertions, feature = "checked-cells"))]
impl<T: ?Sized> Drop for GoCellMut<'_, T> {
    fn drop(&mut self) {
        self.cell.borrows.store(0, Ordering::Release);
    }
}

impl<T: ?Sized> Deref for GoCellRef<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value
    }
}
impl<T: ?Sized> Deref for GoCellMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value
    }
}
impl<T: ?Sized> DerefMut for GoCellMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}
impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for GoCellRef<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::GoCell;

    #[test]
    fn shared_reads_coexist_and_release() {
        let cell = GoCell::new(3);
        let first = cell.read().unwrap();
        let second = cell.read().unwrap();
        assert_eq!(*first + *second, 6);
        drop((first, second));
        *cell.write().unwrap() = 4;
        assert_eq!(*cell.read().unwrap(), 4);
    }

    #[test]
    #[should_panic(expected = "GoCell written while borrowed")]
    fn write_during_read_panics() {
        let cell = GoCell::new(0);
        let _reader = cell.read().unwrap();
        let _ = cell.write();
    }

    #[test]
    #[should_panic(expected = "GoCell read while written")]
    fn read_during_write_panics() {
        let cell = GoCell::new(0);
        let _writer = cell.write().unwrap();
        let _ = cell.read();
    }
}
