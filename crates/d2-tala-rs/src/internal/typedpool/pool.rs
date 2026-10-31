// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source-shaped typed pool.

use std::sync::Mutex;

/// A small type-safe pool for reusable temporary values.
///
/// The upstream implementation wraps `sync.Pool`.  Rust keeps the same
/// get/put boundary while using an explicit mutex-backed store so behavior is
/// available on native and WASM targets.  The pool is diagnostic/algorithmic
/// infrastructure; callers must never rely on which previously returned value
/// is selected.
pub(crate) struct Pool<T> {
    values: Mutex<Vec<T>>,
    new_value: Box<dyn Fn() -> T + Send + Sync>,
}

impl<T> Pool<T> {
    /// Creates a pool whose allocator supplies a fresh value when empty.
    pub(crate) fn new(new_value: impl Fn() -> T + Send + Sync + 'static) -> Self {
        Self {
            values: Mutex::new(Vec::new()),
            new_value: Box::new(new_value),
        }
    }

    /// Takes a pooled value or invokes the allocator.
    pub(crate) fn get(&self) -> T {
        self.values
            .lock()
            .expect("typed TALA pool mutex poisoned")
            .pop()
            .unwrap_or_else(|| (self.new_value)())
    }

    /// Returns a value to the pool.
    pub(crate) fn put(&self, value: T) {
        self.values
            .lock()
            .expect("typed TALA pool mutex poisoned")
            .push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::Pool;

    #[test]
    fn reuses_values_without_exposing_storage_type() {
        let pool = Pool::new(|| String::from("fresh"));
        assert_eq!(pool.get(), "fresh");
        pool.put(String::from("reused"));
        assert_eq!(pool.get(), "reused");
    }
}
