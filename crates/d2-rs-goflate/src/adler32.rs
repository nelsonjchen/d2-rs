// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 src/hash/adler32/adler32.go.
// Copyright 2009 The Go Authors. All rights reserved. BSD-3-Clause; see
// THIRD_PARTY_NOTICES.md.

//! Adler-32 checksum (RFC 1950), as used by the zlib trailer.

/// mod is the largest prime that is less than 65536.
const MOD: u32 = 65521;
/// nmax is the largest n such that
/// 255 * n * (n+1) / 2 + (n+1) * (mod-1) <= 2^32-1.
const NMAX: usize = 5552;

/// Go `adler32.digest` (s2 << 16 | s1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Digest(u32);

impl Default for Digest {
    fn default() -> Self {
        Self::new()
    }
}

impl Digest {
    /// `adler32.New` / `Reset`.
    pub fn new() -> Self {
        Digest(1)
    }

    /// `digest.Write`.
    pub fn write(&mut self, mut p: &[u8]) {
        let (mut s1, mut s2) = (self.0 & 0xffff, self.0 >> 16);
        while !p.is_empty() {
            let q;
            if p.len() > NMAX {
                (p, q) = p.split_at(NMAX);
            } else {
                q = &[];
            }
            #[cfg(target_arch = "aarch64")]
            let tail = {
                let (wide, rest) = p.as_chunks::<32>();
                neon_blocks(&mut s1, &mut s2, wide);
                rest
            };
            #[cfg(not(target_arch = "aarch64"))]
            let tail = p;
            // Sixteen serial steps add 16 * s1 + sum((16 - i) * x[i]) to s2
            // and sum(x[i]) to s1. Every partial sum stays below the serial
            // loop's final values, so the result is identical.
            let (blocks, remainder) = tail.as_chunks::<16>();
            for block in blocks {
                let (mut sum, mut weighted) = (0_u32, 0_u32);
                for (index, &x) in block.iter().enumerate() {
                    sum += u32::from(x);
                    weighted += (16 - index as u32) * u32::from(x);
                }
                s2 += 16 * s1 + weighted;
                s1 += sum;
            }
            for &x in remainder {
                s1 += u32::from(x);
                s2 += s1;
            }
            s1 %= MOD;
            s2 %= MOD;
            p = q;
        }
        self.0 = s2 << 16 | s1;
    }

    /// `digest.Sum32`.
    pub fn sum32(&self) -> u32 {
        self.0
    }
}

/// The serial steps over 32-byte blocks (at most `NMAX` bytes in all), with
/// NEON lanes. Over n blocks, s2 gains 32 * n * s1 plus 32 times each block's
/// preceding byte sum plus sum((32 - i) * x[i]) per block, and s1 gains the
/// byte sum. Every lane holds a part of the serial loop's unreduced sums, so
/// no lane wraps and the reduced result is identical.
#[cfg(target_arch = "aarch64")]
fn neon_blocks(s1: &mut u32, s2: &mut u32, blocks: &[[u8; 32]]) {
    use std::arch::aarch64::*;
    if blocks.is_empty() {
        return;
    }
    debug_assert!(blocks.len() * 32 <= NMAX);
    // SAFETY: NEON is part of the aarch64 baseline; every load reads 16
    // bytes inside a 32-byte block.
    unsafe {
        let mut byte_sums = vdupq_n_u32(0);
        let mut prior_sums = vdupq_n_u32(0);
        // Per-position byte totals; at most NMAX / 32 * 255 fits in u16.
        let mut columns = [vdupq_n_u16(0); 4];
        for block in blocks {
            let low = vld1q_u8(block.as_ptr());
            let high = vld1q_u8(block.as_ptr().add(16));
            prior_sums = vaddq_u32(prior_sums, byte_sums);
            byte_sums = vpadalq_u16(byte_sums, vpadalq_u8(vpaddlq_u8(low), high));
            columns[0] = vaddw_u8(columns[0], vget_low_u8(low));
            columns[1] = vaddw_u8(columns[1], vget_high_u8(low));
            columns[2] = vaddw_u8(columns[2], vget_low_u8(high));
            columns[3] = vaddw_u8(columns[3], vget_high_u8(high));
        }
        let mut weighted = vshlq_n_u32::<5>(prior_sums);
        const WEIGHTS: [u16; 32] = {
            let mut weights = [0; 32];
            let mut index = 0;
            while index < 32 {
                weights[index] = 32 - index as u16;
                index += 1;
            }
            weights
        };
        for (column, weights) in columns.iter().zip(WEIGHTS.as_chunks::<8>().0) {
            let weights = vld1q_u16(weights.as_ptr());
            weighted = vmlal_u16(weighted, vget_low_u16(*column), vget_low_u16(weights));
            weighted = vmlal_high_u16(weighted, *column, weights);
        }
        *s2 += 32 * blocks.len() as u32 * *s1 + vaddvq_u32(weighted);
        *s1 += vaddvq_u32(byte_sums);
    }
}

/// `adler32.Checksum`.
pub fn checksum(data: &[u8]) -> u32 {
    let mut d = Digest::new();
    d.write(data);
    d.sum32()
}

#[cfg(test)]
mod tests {
    /// RFC 1950's definition, reduced after every byte.
    fn reference(data: &[u8]) -> u32 {
        let (mut s1, mut s2) = (1_u32, 0_u32);
        for &x in data {
            s1 = (s1 + u32::from(x)) % super::MOD;
            s2 = (s2 + s1) % super::MOD;
        }
        s2 << 16 | s1
    }

    #[test]
    fn blocked_sums_match_the_definition() {
        let data = (0..3 * super::NMAX + 37)
            .map(|index| {
                if index % 3 == 0 {
                    255
                } else {
                    (index * 7) as u8
                }
            })
            .collect::<Vec<_>>();
        for len in [
            0,
            1,
            15,
            16,
            17,
            255,
            super::NMAX,
            super::NMAX + 1,
            data.len(),
        ] {
            assert_eq!(
                super::checksum(&data[..len]),
                reference(&data[..len]),
                "{len}"
            );
        }
        let mut digest = super::Digest::new();
        for part in data.chunks(1000) {
            digest.write(part);
        }
        assert_eq!(digest.sum32(), reference(&data));
        assert_eq!(super::checksum(&[255; 20000]), reference(&[255; 20000]));
    }

    /// Every block/remainder split, including whole wide blocks of maximal
    /// bytes at the reduction boundary, after a digest carrying large sums.
    #[test]
    fn wide_blocks_match_the_definition() {
        let data = (0..2 * super::NMAX + 99)
            .map(|index| (index * 131 % 256) as u8)
            .collect::<Vec<_>>();
        let lengths = (0..100).chain(super::NMAX - 40..super::NMAX + 40);
        for len in lengths.clone() {
            assert_eq!(
                super::checksum(&data[..len]),
                reference(&data[..len]),
                "{len}"
            );
            let ones = vec![255; len];
            assert_eq!(super::checksum(&ones), reference(&ones), "255 x {len}");
        }
        let prefix = vec![255; 3 * super::NMAX - 1];
        for len in lengths {
            let mut digest = super::Digest::new();
            digest.write(&prefix);
            digest.write(&data[..len]);
            let whole = [prefix.as_slice(), &data[..len]].concat();
            assert_eq!(digest.sum32(), reference(&whole), "prefixed {len}");
        }
    }
}
