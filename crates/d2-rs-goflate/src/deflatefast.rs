// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 src/compress/flate/deflatefast.go and
// load_store.go.
// Copyright 2016 The Go Authors. All rights reserved. BSD-3-Clause; see
// THIRD_PARTY_NOTICES.md.

//! Shared state and helpers for the level 1-6 fast encoders
//! (`deflatefast.go`, `load_store.go`).

use crate::deflate::{MAX_MATCH_LENGTH, MAX_STORE_BLOCK_SIZE};
use crate::level1::FastEncL1;
use crate::level2::FastEncL2;
use crate::level3::FastEncL3;
use crate::level4::FastEncL4;
use crate::level5::FastEncL5;
use crate::level6::FastEncL6;
use crate::token::Tokens;

/// tableBits is the number of bits used in the hash table.
pub(crate) const TABLE_BITS: u8 = 15;
/// tableSize is the size of the hash table.
pub(crate) const TABLE_SIZE: usize = 1 << TABLE_BITS;
/// hashLongBytes is the number of bytes used for long table hashes.
pub(crate) const HASH_LONG_BYTES: u8 = 7;
/// baseMatchOffset is the smallest match offset.
pub(crate) const BASE_MATCH_OFFSET: i32 = 1;
/// baseMatchLength is the smallest match length per RFC section 3.2.5.
pub(crate) const BASE_MATCH_LENGTH: i32 = 3;
/// maxMatchOffset is the largest match offset.
pub(crate) const MAX_MATCH_OFFSET: i32 = 1 << 15;
/// allocHistory is the size to preallocate for history.
const ALLOC_HISTORY: usize = MAX_STORE_BLOCK_SIZE * 5;
/// bufferReset is the buffer offset at which the history is reset.
pub(crate) const BUFFER_RESET: i32 =
    ((1i64 << 31) - ALLOC_HISTORY as i64 - MAX_STORE_BLOCK_SIZE as i64 - 1) as i32;

const PRIME3BYTES: u32 = 506832829;
const PRIME4BYTES: u32 = 2654435761;
const PRIME5BYTES: u64 = 889523592379;
const PRIME6BYTES: u64 = 227718039650203;
const PRIME7BYTES: u64 = 58295818150454627;
const PRIME8BYTES: u64 = 0xcf1bbcdcb7a56463;

/// `hash4u` returns the hash of u to fit in a hash table with h bits.
#[inline]
pub(crate) fn hash4u(u: u32, h: u8) -> u32 {
    u.wrapping_mul(PRIME4BYTES) >> (32 - u32::from(h))
}

/// `hashLen` returns a hash of the first n bytes of u, using b output bits.
#[inline]
pub(crate) fn hash_len(u: u64, b: u8, n: u8) -> u32 {
    let b = u32::from(b);
    match n {
        3 => ((u << 8) as u32).wrapping_mul(PRIME3BYTES) >> (32 - b),
        5 => ((u << (64 - 40)).wrapping_mul(PRIME5BYTES) >> (64 - b)) as u32,
        6 => ((u << (64 - 48)).wrapping_mul(PRIME6BYTES) >> (64 - b)) as u32,
        7 => ((u << (64 - 56)).wrapping_mul(PRIME7BYTES) >> (64 - b)) as u32,
        8 => (u.wrapping_mul(PRIME8BYTES) >> (64 - b)) as u32,
        _ => (u as u32).wrapping_mul(PRIME4BYTES) >> (32 - b),
    }
}

/// `loadLE32` (panics like Go on out-of-range or negative indices).
#[inline]
pub(crate) fn load_le32(b: &[u8], i: i32) -> u32 {
    let i = usize::try_from(i).expect("loadLE32 index");
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

/// `loadLE64`.
#[inline]
pub(crate) fn load_le64(b: &[u8], i: i32) -> u64 {
    let i = usize::try_from(i).expect("loadLE64 index");
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}

/// `matchLen` returns the maximum common prefix length of a and b.
/// a must be the shortest of the two.
#[inline]
pub(crate) fn match_len(a: &[u8], b: &[u8]) -> usize {
    let b = &b[..a.len()];
    // Compare eight bytes at a time; the first differing byte of a word is
    // its lowest set XOR byte in little-endian order.
    let mut n = 0;
    for (x, y) in a.as_chunks::<8>().0.iter().zip(b.as_chunks::<8>().0) {
        let diff = u64::from_le_bytes(*x) ^ u64::from_le_bytes(*y);
        if diff != 0 {
            return n + (diff.trailing_zeros() / 8) as usize;
        }
        n += 8;
    }
    n + a[n..]
        .iter()
        .zip(&b[n..])
        .take_while(|(x, y)| x == y)
        .count()
}

/// `tableEntry` stores the offset of a hash match in the input history.
#[derive(Clone, Copy, Default)]
pub(crate) struct TableEntry {
    pub(crate) offset: i32,
}

/// `tableEntryPrev` stores the current and previous offsets for a hash entry.
#[derive(Clone, Copy, Default)]
pub(crate) struct TableEntryPrev {
    pub(crate) cur: TableEntry,
    pub(crate) prev: TableEntry,
}

/// `fastGen` maintains the table for matches, and the previous byte block
/// for level 1 and up.
pub(crate) struct FastGen {
    pub(crate) hist: Vec<u8>,
    /// Go `cap(e.hist)`: 0 until first use, then allocHistory.
    hist_cap: usize,
    pub(crate) cur: i32,
}

impl FastGen {
    pub(crate) fn new() -> Self {
        Self {
            hist: Vec::new(),
            hist_cap: 0,
            cur: MAX_STORE_BLOCK_SIZE as i32,
        }
    }

    /// `addBlock` appends src to the history and returns the offset where
    /// src starts in e.hist.
    pub(crate) fn add_block(&mut self, src: &[u8]) -> i32 {
        // check if we have space already
        if self.hist.len() + src.len() > self.hist_cap {
            if self.hist_cap == 0 {
                self.hist = Vec::with_capacity(ALLOC_HISTORY);
                self.hist_cap = ALLOC_HISTORY;
            } else {
                if self.hist_cap < MAX_MATCH_OFFSET as usize * 2 {
                    panic!("unexpected buffer size");
                }
                // Move down
                let offset = self.hist.len() as i32 - MAX_MATCH_OFFSET;
                self.hist
                    .copy_within(offset as usize..(offset + MAX_MATCH_OFFSET) as usize, 0);
                self.cur += offset;
                self.hist.truncate(MAX_MATCH_OFFSET as usize);
            }
        }
        let s = self.hist.len() as i32;
        self.hist.extend_from_slice(src);
        s
    }

    /// `matchLenLimited` returns the match length between offsets s and t in
    /// src. The maximum length returned is maxMatchLength - 4.
    #[inline]
    pub(crate) fn match_len_limited(s: i32, t: i32, src: &[u8]) -> i32 {
        let (s, t) = (s as usize, t as usize);
        let a = &src[s..(s + MAX_MATCH_LENGTH - 4).min(src.len())];
        match_len(a, &src[t..]) as i32
    }

    /// `matchLenLong` returns the match length between offsets s and t in src.
    #[inline]
    pub(crate) fn match_len_long(s: i32, t: i32, src: &[u8]) -> i32 {
        match_len(&src[s as usize..], &src[t as usize..]) as i32
    }

    /// `reset` resets the encoding table to prepare for a new compression
    /// stream.
    pub(crate) fn reset(&mut self) {
        if self.hist_cap < ALLOC_HISTORY {
            self.hist = Vec::with_capacity(ALLOC_HISTORY);
            self.hist_cap = ALLOC_HISTORY;
        }
        // We offset current position so everything will be out of reach.
        // If we are above the buffer reset it will be cleared anyway since len(hist) == 0.
        if self.cur <= BUFFER_RESET {
            self.cur += MAX_MATCH_OFFSET + self.hist.len() as i32;
        }
        self.hist.clear();
    }
}

/// Shift a table offset down for the `e.cur >= bufferReset` wraparound guard.
#[inline]
pub(crate) fn shift_offset(v: i32, min_off: i32, cur: i32) -> i32 {
    if v <= min_off {
        0
    } else {
        v - cur + MAX_MATCH_OFFSET
    }
}

/// `fastEnc`: the level 1-6 fast encoders.
pub(crate) enum FastEnc {
    L1(Box<FastEncL1>),
    L2(Box<FastEncL2>),
    L3(Box<FastEncL3>),
    L4(Box<FastEncL4>),
    L5(Box<FastEncL5>),
    L6(Box<FastEncL6>),
}

impl FastEnc {
    /// `newFastEnc` returns a fastEnc encoder for the given compression level (1-6).
    pub(crate) fn new(level: i32) -> Self {
        match level {
            1 => FastEnc::L1(Box::new(FastEncL1::new())),
            2 => FastEnc::L2(Box::new(FastEncL2::new())),
            3 => FastEnc::L3(Box::new(FastEncL3::new())),
            4 => FastEnc::L4(Box::new(FastEncL4::new())),
            5 => FastEnc::L5(Box::new(FastEncL5::new())),
            6 => FastEnc::L6(Box::new(FastEncL6::new())),
            _ => panic!("invalid level specified"),
        }
    }

    /// encode src into dst.
    pub(crate) fn encode(&mut self, dst: &mut Tokens, src: &[u8]) {
        match self {
            FastEnc::L1(e) => e.encode(dst, src),
            FastEnc::L2(e) => e.encode(dst, src),
            FastEnc::L3(e) => e.encode(dst, src),
            FastEnc::L4(e) => e.encode(dst, src),
            FastEnc::L5(e) => e.encode(dst, src),
            FastEnc::L6(e) => e.encode(dst, src),
        }
    }

    /// reset the encoder so matches are not made with previous data.
    pub(crate) fn reset(&mut self) {
        match self {
            FastEnc::L1(e) => e.fast_gen.reset(),
            FastEnc::L2(e) => e.fast_gen.reset(),
            FastEnc::L3(e) => e.fast_gen.reset(),
            FastEnc::L4(e) => e.fast_gen.reset(),
            FastEnc::L5(e) => e.fast_gen.reset(),
            FastEnc::L6(e) => e.fast_gen.reset(),
        }
    }
}

#[cfg(test)]
mod tests {
    /// The word-wise prefix length equals Go's byte-by-byte matchLen for
    /// every mismatch position and slice length around the word size.
    #[test]
    fn match_len_is_the_common_prefix_length() {
        for len in 0..40 {
            for mismatch in 0..=len {
                let a: Vec<u8> = (0..len as u8).collect();
                let mut b = a.clone();
                b.extend_from_slice(&[7, 7, 7]);
                if mismatch < len {
                    b[mismatch] ^= 0x80;
                }
                assert_eq!(super::match_len(&a, &b), mismatch, "len {len}");
            }
        }
    }
}
