// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2009 The Go Authors. All rights reserved.
// Rust translation Copyright 2026 Nelson Chen.
// Go 1.27 byte-slice backing storage and append growth, used by Goldmark.
// Complete Go BSD terms are in THIRD_PARTY_NOTICES.md.
use std::cell::{Ref, RefCell};
use std::rc::Rc;

/// Shared mutable byte backing with a distinct slice length and capacity.
/// Cloning copies the slice header, preserving aliases without copying bytes.
#[derive(Clone, Debug)]
pub struct Bytes {
    backing: Rc<RefCell<Vec<u8>>>,
    start: usize,
    len: usize,
    capacity: usize,
}

impl Bytes {
    pub fn new(bytes: &[u8]) -> Self {
        Self::with_capacity(bytes, bytes.len())
    }

    pub fn with_capacity(bytes: &[u8], capacity: usize) -> Self {
        assert!(capacity >= bytes.len());
        let mut backing = vec![0; capacity];
        backing[..bytes.len()].copy_from_slice(bytes);
        Self {
            backing: Rc::new(RefCell::new(backing)),
            start: 0,
            len: bytes.len(),
            capacity,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn bytes(&self) -> Ref<'_, [u8]> {
        Ref::map(self.backing.borrow(), |v| {
            &v[self.start..self.start + self.len]
        })
    }
    /// Slice bounds may extend to capacity, as with Go's two-index slicing.
    pub fn slice(&self, start: usize, stop: usize) -> Self {
        assert!(start <= stop && stop <= self.capacity);
        Self {
            backing: self.backing.clone(),
            start: self.start + start,
            len: stop - start,
            capacity: self.capacity - start,
        }
    }
    pub fn set(&self, index: usize, byte: u8) {
        assert!(index < self.len);
        self.backing.borrow_mut()[self.start + index] = byte;
    }
    pub(super) fn append_byte(&self, byte: u8) -> Self {
        let mut result = if self.len < self.capacity {
            self.clone()
        } else {
            Self::with_capacity(
                &self.bytes(),
                grow_byte_capacity(self.len + 1, self.capacity),
            )
        };
        result.backing.borrow_mut()[result.start + result.len] = byte;
        result.len += 1;
        result
    }
}

// runtime/slice.go nextslicecap followed by runtime/msize.go roundupsize(noscan).
fn grow_byte_capacity(new_len: usize, old: usize) -> usize {
    let mut next = old.saturating_mul(2);
    if new_len > next {
        next = new_len;
    } else if old >= 256 {
        next = old;
        while next < new_len {
            next = next.checked_add((next + 768) >> 2).unwrap_or(new_len);
        }
    }
    const CLASSES: &[usize] = &[
        0, 8, 16, 24, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 256, 288,
        320, 352, 384, 416, 448, 480, 512, 576, 640, 704, 768, 896, 1024, 1152, 1280, 1408, 1536,
        1792, 2048, 2304, 2688, 3072, 3200, 3456, 4096, 4864, 5376, 6144, 6528, 6784, 6912, 8192,
        9472, 9728, 10240, 10880, 12288, 13568, 14336, 16384, 18432, 19072, 20480, 21760, 24576,
        27264, 28672, 32768,
    ];
    if next <= 32760 {
        *CLASSES.iter().find(|n| **n >= next).unwrap()
    } else {
        next.checked_add(8191).map_or(next, |n| n & !8191)
    }
}

// Serialization records values only; deserialization creates independent backing.
// Shared allocation identity is tracked separately by diagnostic observers.
impl serde::Serialize for Bytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&*self.bytes(), serializer)
    }
}
impl<'de> serde::Deserialize<'de> for Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = <Vec<u8> as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Self::new(&value))
    }
}
