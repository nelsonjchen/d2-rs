// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 src/compress/flate/token.go.
// Copyright 2009 The Go Authors. All rights reserved. BSD-3-Clause; see
// THIRD_PARTY_NOTICES.md.

//! LZ77 tokens with running histograms (`token.go`).

use crate::huffman_bit_writer::{
    END_BLOCK_MARKER, LENGTH_EXTRA_BITS, OFFSET_CODE_COUNT, OFFSET_EXTRA_BITS,
};
use crate::huffman_code::LITERAL_COUNT;

// Token is a compound value:
// bits 0-16  xoffset = offset - MIN_OFFSET_SIZE, or literal - 16 bits
// bits 16-22 offset code - 5 bits
// bits 22-30 xlength = length - MIN_MATCH_LENGTH - 8 bits
// bits 30-32 type, 0 = literal  1=EOF  2=Match   3=Unused - 2 bits
pub(crate) const LENGTH_SHIFT: u32 = 22;
const OFFSET_MASK: u32 = (1 << LENGTH_SHIFT) - 1;
pub(crate) const MATCH_TYPE: u32 = 1 << 30;
pub(crate) const MATCH_OFFSET_ONLY_MASK: u32 = 0xffff;

/// When true, reproduce the fused multiply-adds that Go's arm64 backend
/// emits for `EstimatedBits`/`mFastLog2` (ARM64.rules FMADDS/FNMSUBS). Go on
/// amd64 (GOAMD64 < v3) rounds each operation separately; set this to false
/// to model that target. The pinned oracle toolchain is go1.27.0
/// darwin/arm64.
pub(crate) const GO_FUSED_FMA: bool = true;

/// The length code for length X (MIN_MATCH_LENGTH <= X <= MAX_MATCH_LENGTH)
/// is LENGTH_CODES[length - MIN_MATCH_LENGTH]. LENGTH_CODES1 is the same,
/// starting at 1; OFFSET_CODES14 is OFFSET_CODES with 14 added.
pub(crate) static LENGTH_CODES: [u8; 256] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 12, 12, 13, 13, 13, 13, 14, 14, 14,
    14, 15, 15, 15, 15, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17, 17, 17, 17, 18, 18, 18,
    18, 18, 18, 18, 18, 19, 19, 19, 19, 19, 19, 19, 19, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20,
    20, 20, 20, 20, 20, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 22, 22, 22,
    22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23,
    23, 23, 23, 23, 23, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24,
    24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25,
    25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 26, 26, 26,
    26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26,
    26, 26, 26, 26, 26, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27,
    27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 28,
];

pub(crate) static LENGTH_CODES1: [u8; 256] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 13, 13, 14, 14, 14, 14, 15, 15,
    15, 15, 16, 16, 16, 16, 17, 17, 17, 17, 17, 17, 17, 17, 18, 18, 18, 18, 18, 18, 18, 18, 19, 19,
    19, 19, 19, 19, 19, 19, 20, 20, 20, 20, 20, 20, 20, 20, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21,
    21, 21, 21, 21, 21, 21, 22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 22, 23, 23,
    23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24,
    24, 24, 24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25,
    25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26,
    26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 27, 27,
    27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27,
    27, 27, 27, 27, 27, 27, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 29,
];

pub(crate) static OFFSET_CODES: [u32; 256] = [
    0, 1, 2, 3, 4, 4, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9,
    10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15,
];

pub(crate) static OFFSET_CODES14: [u32; 256] = [
    14, 15, 16, 17, 18, 18, 19, 19, 20, 20, 20, 20, 21, 21, 21, 21, 22, 22, 22, 22, 22, 22, 22, 22,
    23, 23, 23, 23, 23, 23, 23, 23, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24,
    25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 25, 26, 26, 26, 26, 26, 26, 26, 26,
    26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26,
    27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27,
    27, 27, 27, 27, 27, 27, 27, 27, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29,
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29,
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29,
];

/// A token is a token that will be written to output stream (Go `token`).
pub(crate) type Token = u32;

/// `tokens`: tokens are compound values as described above. Histograms are
/// created as tokens are added. A full block is allocated.
pub(crate) struct Tokens {
    /// codes 256->maxnumlit
    pub(crate) extra_hist: [u16; 32],
    /// offset codes
    pub(crate) off_hist: [u16; 32],
    /// codes 0->255
    pub(crate) lit_hist: [u16; 256],
    n_filled: i64,
    /// Must be able to contain maxStoreBlockSize (Go uint16).
    pub(crate) n: u16,
    pub(crate) tokens: Vec<Token>,
}

impl Tokens {
    pub(crate) fn new() -> Self {
        Self {
            extra_hist: [0; 32],
            off_hist: [0; 32],
            lit_hist: [0; 256],
            n_filled: 0,
            n: 0,
            tokens: vec![0; 65536],
        }
    }

    /// `Reset` resets the tokens and histograms.
    pub(crate) fn reset(&mut self) {
        if self.n == 0 {
            return;
        }
        self.n = 0;
        self.n_filled = 0;
        self.lit_hist = [0; 256];
        self.extra_hist = [0; 32];
        self.off_hist = [0; 32];
    }

    /// `AddLiteral` adds a single literal to the tokens.
    #[inline]
    pub(crate) fn add_literal(&mut self, lit: u8) {
        self.tokens[self.n as usize] = Token::from(lit);
        self.lit_hist[lit as usize] = self.lit_hist[lit as usize].wrapping_add(1);
        self.n = self.n.wrapping_add(1);
    }

    /// `EstimatedBits` returns an estimated minimum size for the optimal
    /// compression of t. Minimum 1 bit is assigned per symbol. Maximum 15
    /// bits are assigned per symbol.
    pub(crate) fn estimated_bits(&self) -> i64 {
        let mut shannon: f32 = 0.0;
        let mut bits: i64 = 0;
        let mut n_matches: i64 = 0;
        let total = i64::from(self.n) + self.n_filled;
        if total > 0 {
            let inv_total = 1.0f32 / total as f32;
            for &v in self.lit_hist.iter() {
                if v > 0 {
                    let n = f32::from(v);
                    shannon = shannon_add(shannon, sym_bits(n * inv_total), n);
                }
            }
            // Just add 15 for EOB
            shannon += 15.0;
            for (i, &v) in self.extra_hist[1..LITERAL_COUNT - 256].iter().enumerate() {
                if v > 0 {
                    let n = f32::from(v);
                    shannon = shannon_add(shannon, sym_bits(n * inv_total), n);
                    bits += i64::from(LENGTH_EXTRA_BITS[i & 31]) * i64::from(v);
                    n_matches += i64::from(v);
                }
            }
        }
        if n_matches > 0 {
            let inv_total = 1.0f32 / n_matches as f32;
            for (i, &v) in self.off_hist[..OFFSET_CODE_COUNT].iter().enumerate() {
                if v > 0 {
                    let n = f32::from(v);
                    shannon = shannon_add(shannon, sym_bits(n * inv_total), n);
                    bits += i64::from(OFFSET_EXTRA_BITS[i & 31]) * i64::from(v);
                }
            }
        }
        // Go int(float32): truncation toward zero.
        shannon as i64 + bits
    }

    /// `AddMatch` adds a match to the tokens.
    #[inline]
    pub(crate) fn add_match(&mut self, xlength: u32, mut xoffset: u32) {
        let o_code = offset_code(xoffset);
        xoffset |= o_code << 16;

        let e = LENGTH_CODES1[(xlength as u8) as usize] as usize;
        self.extra_hist[e] = self.extra_hist[e].wrapping_add(1);
        let o = (o_code & 31) as usize;
        self.off_hist[o] = self.off_hist[o].wrapping_add(1);
        self.tokens[self.n as usize] = MATCH_TYPE | xlength << LENGTH_SHIFT | xoffset;
        self.n = self.n.wrapping_add(1);
    }

    /// `AddMatchLong` adds a match to the tokens, potentially longer than max
    /// match length. Length should NOT have the base subtracted, only offset
    /// should.
    #[inline]
    pub(crate) fn add_match_long(&mut self, mut xlength: i32, mut xoffset: u32) {
        let oc = offset_code(xoffset);
        xoffset |= oc << 16;
        while xlength > 0 {
            let mut xl = xlength;
            if xl > 258 {
                // We need to have at least baseMatchLength left over for next loop.
                if xl > 258 + BASE_MATCH_LENGTH {
                    xl = 258;
                } else {
                    xl = 258 - BASE_MATCH_LENGTH;
                }
            }
            xlength -= xl;
            xl -= BASE_MATCH_LENGTH;
            let e = LENGTH_CODES1[(xl as u8) as usize] as usize;
            self.extra_hist[e] = self.extra_hist[e].wrapping_add(1);
            let o = (oc & 31) as usize;
            self.off_hist[o] = self.off_hist[o].wrapping_add(1);
            self.tokens[self.n as usize] = MATCH_TYPE | (xl as u32) << LENGTH_SHIFT | xoffset;
            self.n = self.n.wrapping_add(1);
        }
    }

    /// `AddEOB` adds an end of block marker to the tokens.
    #[inline]
    pub(crate) fn add_eob(&mut self) {
        self.tokens[self.n as usize] = END_BLOCK_MARKER as Token;
        self.extra_hist[0] = self.extra_hist[0].wrapping_add(1);
        self.n = self.n.wrapping_add(1);
    }

    /// `Slice` returns the tokens.
    #[inline]
    pub(crate) fn slice(&self) -> &[Token] {
        &self.tokens[..self.n as usize]
    }
}

const BASE_MATCH_LENGTH: i32 = 3;

/// `emitLiterals` writes a literal chunk.
#[inline]
pub(crate) fn emit_literals(dst: &mut Tokens, lit: &[u8]) {
    for &v in lit {
        dst.tokens[dst.n as usize] = Token::from(v);
        dst.lit_hist[v as usize] = dst.lit_hist[v as usize].wrapping_add(1);
        dst.n = dst.n.wrapping_add(1);
    }
}

/// `min(15, max(1, -mFastLog2(x)))` (Go float32 min/max; no NaNs arise).
#[inline]
fn sym_bits(x: f32) -> f32 {
    let v = -m_fast_log2(x);
    let v = if v > 1.0 { v } else { 1.0 };
    if v < 15.0 { v } else { 15.0 }
}

/// `shannon += b * n`.
#[inline]
fn shannon_add(shannon: f32, b: f32, n: f32) -> f32 {
    if GO_FUSED_FMA {
        b.mul_add(n, shannon)
    } else {
        shannon + b * n
    }
}

/// `mFastLog2` returns a fast approximation of log2(val).
/// From https://stackoverflow.com/a/28730362.
#[inline]
#[allow(clippy::excessive_precision)] // Go's literals, kept verbatim.
fn m_fast_log2(val: f32) -> f32 {
    let mut ux = val.to_bits() as i32;
    let mut log2 = (((ux >> 23) & 255) - 128) as f32;
    ux &= -0x7f800001;
    ux = ux.wrapping_add(127 << 23);
    let uval = f32::from_bits(ux as u32);
    // ((-0.34484843)*uval+2.02466578)*uval - 0.67487759
    let poly = if GO_FUSED_FMA {
        (-0.34484843f32)
            .mul_add(uval, 2.02466578f32)
            .mul_add(uval, -0.67487759f32)
    } else {
        ((-0.34484843f32) * uval + 2.02466578f32) * uval - 0.67487759f32
    };
    log2 += poly;
    log2
}

/// `token.offset` returns the offset of a match token.
#[inline]
pub(crate) fn token_offset(t: Token) -> u32 {
    t & OFFSET_MASK
}

/// `token.length` returns the length of a match token.
#[inline]
pub(crate) fn token_length(t: Token) -> u8 {
    (t >> LENGTH_SHIFT) as u8
}

/// `lengthCode` converts a match length to its code.
#[inline]
pub(crate) fn length_code(len: u8) -> u8 {
    LENGTH_CODES[len as usize]
}

/// `offsetCode` returns the offset code corresponding to a specific offset.
#[inline]
pub(crate) fn offset_code(off: u32) -> u32 {
    if off < OFFSET_CODES.len() as u32 {
        return OFFSET_CODES[(off as u8) as usize];
    }
    OFFSET_CODES14[((off >> 7) as u8) as usize]
}
