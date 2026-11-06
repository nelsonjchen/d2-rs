// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Actual shared geo.Point allocations; copies retain the allocation and live
//! coordinates. Numerical copies detach once per actual alias class.
use super::Point;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
// Coordinates are stored as f64 bit patterns. Writers run on the layout
// thread only; rayon route workers only read, so relaxed loads and stores
// (plain ldr/str on arm64) replace a lock round trip per read.
#[derive(Debug)]
struct PointCell {
    x: AtomicU64,
    y: AtomicU64,
}
#[derive(Clone, Debug)]
pub(super) struct SharedOwnerPoint(Arc<PointCell>);
impl SharedOwnerPoint {
    pub(super) fn new(point: Point) -> Self {
        Self(Arc::new(PointCell {
            x: AtomicU64::new(point.x.to_bits()),
            y: AtomicU64::new(point.y.to_bits()),
        }))
    }
    pub(super) fn get(&self) -> Point {
        Point {
            x: f64::from_bits(self.0.x.load(Relaxed)),
            y: f64::from_bits(self.0.y.load(Relaxed)),
        }
    }
    pub(super) fn set(&self, point: Point) {
        self.0.x.store(point.x.to_bits(), Relaxed);
        self.0.y.store(point.y.to_bits(), Relaxed);
    }
    pub(super) fn same_allocation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn point_address(&self) -> usize {
        &self.0.x as *const AtomicU64 as usize
    }
    pub(super) fn allocation_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
    pub(super) fn translate(&self, dx: f64, dy: f64) {
        let point = self.get();
        self.set(Point {
            x: point.x + dx,
            y: point.y + dy,
        });
    }
}

// Source pointer equality concerns allocation identity, never coordinates.
impl PartialEq for SharedOwnerPoint {
    fn eq(&self, other: &Self) -> bool {
        self.same_allocation(other)
    }
}
impl Eq for SharedOwnerPoint {}
