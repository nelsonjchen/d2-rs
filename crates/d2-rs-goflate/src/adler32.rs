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
            for &x in p {
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

/// `adler32.Checksum`.
pub fn checksum(data: &[u8]) -> u32 {
    let mut d = Digest::new();
    d.write(data);
    d.sum32()
}
