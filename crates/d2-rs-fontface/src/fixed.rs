// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2015 The Go Authors. All rights reserved.
// Rust translation of golang.org/x/image v0.45.0 math/fixed (the 26.6 subset
// used by D2's fontface package).

/// `fixed.Int26_6`: a signed 26.6 fixed-point number (Go `int32`).
pub type Int26_6 = i32;

/// `fixed.I`: the integer `value` as a 26.6 value.
pub const fn i(value: i32) -> Int26_6 {
    value.wrapping_shl(6)
}

/// `Int26_6.Floor`.
pub const fn floor(value: Int26_6) -> i32 {
    value >> 6
}

/// `Int26_6.Round` (ties up).
pub const fn round(value: Int26_6) -> i32 {
    value.wrapping_add(0x20) >> 6
}

/// `Int26_6.Ceil`.
pub const fn ceil(value: Int26_6) -> i32 {
    value.wrapping_add(0x3f) >> 6
}

/// `fixed.Point26_6`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point26_6 {
    pub x: Int26_6,
    pub y: Int26_6,
}

/// `fixed.Rectangle26_6`: Min is inclusive, Max exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rectangle26_6 {
    pub min: Point26_6,
    pub max: Point26_6,
}

impl Rectangle26_6 {
    /// `Rectangle26_6.Empty`.
    pub fn empty(&self) -> bool {
        self.min.x >= self.max.x || self.min.y >= self.max.y
    }

    /// `Rectangle26_6.Union`.
    pub fn union(mut self, s: Rectangle26_6) -> Rectangle26_6 {
        if self.empty() {
            return s;
        }
        if s.empty() {
            return self;
        }
        if self.min.x > s.min.x {
            self.min.x = s.min.x;
        }
        if self.min.y > s.min.y {
            self.min.y = s.min.y;
        }
        if self.max.x < s.max.x {
            self.max.x = s.max.x;
        }
        if self.max.y < s.max.y {
            self.max.y = s.max.y;
        }
        self
    }
}
