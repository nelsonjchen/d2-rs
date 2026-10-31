// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 src/compress/flate/huffman_code.go.
// Copyright 2009 The Go Authors. All rights reserved. BSD-3-Clause; see
// THIRD_PARTY_NOTICES.md.

//! Length-limited Huffman code construction (`huffman_code.go`).

use std::sync::OnceLock;

pub(crate) const MAX_BITS_LIMIT: usize = 16;
/// number of valid literals
pub(crate) const LITERAL_COUNT: usize = 286;

/// `hcode` is a huffman code with a bit code and bit length:
/// `length | code << 8`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Hcode(u32);

impl Hcode {
    /// `len` returns the length of the code in bits.
    #[inline]
    pub(crate) fn len(self) -> u8 {
        self.0 as u8
    }

    /// `code64` returns the code as a uint64.
    #[inline]
    pub(crate) fn code64(self) -> u64 {
        u64::from(self.0 >> 8)
    }

    /// `zero` returns true if the code is unset.
    #[inline]
    pub(crate) fn zero(self) -> bool {
        self.0 == 0
    }

    /// `newhcode` combines a code and length into an hcode.
    #[inline]
    fn new(code: u16, length: u8) -> Self {
        Hcode(u32::from(length) | (u32::from(code) << 8))
    }
}

/// `literalNode`.
#[derive(Clone, Copy, Debug, Default)]
struct LiteralNode {
    literal: u16,
    freq: u16,
}

/// `maxNode`.
fn max_node() -> LiteralNode {
    LiteralNode {
        literal: u16::MAX,
        freq: u16::MAX,
    }
}

/// A `levelInfo` describes the state of the constructed tree for a given depth.
#[derive(Clone, Copy, Debug, Default)]
struct LevelInfo {
    /// Our level.
    level: i32,
    /// The frequency of the last node at this level
    last_freq: i32,
    /// The frequency of the next character to add to this level
    next_char_freq: i32,
    /// The frequency of the next pair (from level below) to add to this level.
    /// Only valid if the "needed" value of the next lower level is 0.
    next_pair_freq: i32,
    /// The number of chains remaining to generate for this level before moving
    /// up to the next level
    needed: i32,
}

/// `reverseBits` returns the b-bit reversal of x.
#[inline]
fn reverse_bits(x: u16, b: u8) -> u16 {
    (x << ((16 - u32::from(b)) & 15)).reverse_bits()
}

/// `huffmanEncoder`.
#[derive(Clone)]
pub(crate) struct HuffmanEncoder {
    pub(crate) codes: Vec<Hcode>,
    bit_count: [i32; 17],
    /// freqcache is a reusable buffer with the longest possible frequency table.
    freqcache: Vec<LiteralNode>,
}

impl HuffmanEncoder {
    /// `newHuffmanEncoder`.
    pub(crate) fn new(size: usize) -> Self {
        Self {
            codes: vec![Hcode::default(); size],
            bit_count: [0; 17],
            freqcache: vec![LiteralNode::default(); LITERAL_COUNT + 1],
        }
    }

    /// `bitLength` returns the number of bits needed to encode freq.
    pub(crate) fn bit_length(&self, freq: &[u16]) -> i64 {
        let mut total: i64 = 0;
        for (i, &f) in freq.iter().enumerate() {
            if f != 0 {
                total += i64::from(f) * i64::from(self.codes[i].len());
            }
        }
        total
    }

    /// `bitLengthRaw` will return the number of bits needed to encode b.
    /// For unset codes 1 bit/entry will be added.
    pub(crate) fn bit_length_raw(&self, b: &[u8]) -> i64 {
        let mut total: i64 = 0;
        for &f in b {
            total += i64::from(self.codes[f as usize].len()).max(1);
        }
        total
    }

    /// `canEncodeLen` returns the number of bits to encode freq.
    /// It returns math.MaxInt32 if freq cannot be encoded.
    pub(crate) fn can_encode_len(&self, freq: &[u16]) -> i64 {
        let mut total: i64 = 0;
        for (i, &f) in freq.iter().enumerate() {
            if f != 0 {
                let code = self.codes[i];
                if code.zero() {
                    return i64::from(i32::MAX);
                }
                total += i64::from(f) * i64::from(code.len());
            }
        }
        total
    }

    /// `bitCounts` sets bit_count[i] to the number of literals that should be
    /// encoded using i bits and returns the used length (maxBits+1).
    ///
    /// Only called when count >= 3. `list[..n]` is in order of increasing
    /// frequency; `list[n]` receives the maxNode sentinel.
    fn bit_counts(&mut self, list: &mut [LiteralNode], n: usize, mut max_bits: i32) -> usize {
        if max_bits >= MAX_BITS_LIMIT as i32 {
            panic!("flate: maxBits too large");
        }
        let n = n as i32;
        let list = &mut list[0..(n as usize + 1)];
        list[n as usize] = max_node();

        // The tree can't have greater depth than n - 1, no matter what.
        if max_bits > n - 1 {
            max_bits = n - 1;
        }

        // A bogus "Level 0" whose sole purpose is so that
        // level1.prev.needed==0.
        let mut levels = [LevelInfo::default(); MAX_BITS_LIMIT];
        // leafCounts[i][j] is the number of literals at the left
        // of the level j ancestor.
        let mut leaf_counts = [[0i32; MAX_BITS_LIMIT]; MAX_BITS_LIMIT];

        for level in 1..=max_bits {
            let lu = level as usize;
            levels[lu] = LevelInfo {
                level,
                last_freq: i32::from(list[1].freq),
                next_char_freq: i32::from(list[2].freq),
                next_pair_freq: i32::from(list[0].freq) + i32::from(list[1].freq),
                needed: 0,
            };
            leaf_counts[lu][lu] = 2;
            if level == 1 {
                levels[lu].next_pair_freq = i32::MAX;
            }
        }

        // We need a total of 2*n - 2 items at top level and have already generated 2.
        levels[max_bits as usize].needed = 2 * n - 4;

        let mut level = max_bits as usize;
        while level < 16 {
            let l = levels[level];
            if l.next_pair_freq == i32::MAX && l.next_char_freq == i32::MAX {
                // We've run out of both leafs and pairs.
                levels[level].needed = 0;
                levels[level + 1].next_pair_freq = i32::MAX;
                level += 1;
                continue;
            }

            let prev_freq = l.last_freq;
            if l.next_char_freq < l.next_pair_freq {
                // The next item on this row is a leaf node.
                let nn = leaf_counts[level][level] + 1;
                levels[level].last_freq = l.next_char_freq;
                // Lower leafCounts are the same of the previous node.
                leaf_counts[level][level] = nn;
                let e = list[nn as usize];
                levels[level].next_char_freq = if e.literal < u16::MAX {
                    i32::from(e.freq)
                } else {
                    i32::MAX
                };
            } else {
                // The next item on this row is a pair from the previous row.
                levels[level].last_freq = l.next_pair_freq;
                // Take leaf counts from the lower level, except counts[level] remains the same.
                let save = leaf_counts[level][level];
                leaf_counts[level] = leaf_counts[level - 1];
                leaf_counts[level][level] = save;
                levels[(l.level - 1) as usize].needed = 2;
            }

            levels[level].needed -= 1;
            if levels[level].needed == 0 {
                // We've done everything we need to do for this level.
                if levels[level].level == max_bits {
                    // All done!
                    break;
                }
                let lvl = levels[level].level as usize;
                levels[lvl + 1].next_pair_freq = prev_freq + levels[level].last_freq;
                level += 1;
            } else {
                // If we stole from below, move down temporarily to replenish it.
                while levels[level - 1].needed > 0 {
                    level -= 1;
                }
            }
        }

        // Somethings is wrong if at the end, the top level is null or hasn't used
        // all of the leaves.
        if leaf_counts[max_bits as usize][max_bits as usize] != n {
            panic!("leafCounts[maxBits][maxBits] != n");
        }

        let mb = max_bits as usize;
        let mut bits = 1usize;
        let counts = &leaf_counts[mb];
        let mut level = mb;
        while level > 0 {
            // chain.leafCount gives the number of literals requiring at least "bits"
            // bits to encode.
            self.bit_count[bits] = counts[level] - counts[level - 1];
            bits += 1;
            level -= 1;
        }
        mb + 1
    }

    /// `assignEncodingAndSize` assigns bit counts and encodings to the leaves
    /// as specified in RFC 1951 3.2.2.
    fn assign_encoding_and_size(&mut self, bit_count_len: usize, list: &mut [LiteralNode]) {
        let mut code: u16 = 0;
        let mut list_len = list.len();
        for n in 0..bit_count_len {
            let bits = self.bit_count[n];
            code <<= 1;
            if n == 0 || bits == 0 {
                continue;
            }
            // The literals list[len(list)-bits] .. list[len(list)-bits]
            // are encoded using "bits" bits, and get the values
            // code, code + 1, ....  The code values are
            // assigned in literal order (not frequency order).
            let start = list_len - bits as usize;
            let chunk = &mut list[start..list_len];
            // slices.SortFunc by literal: literals are unique, so any correct
            // sort yields Go's order.
            chunk.sort_unstable_by_key(|node| node.literal);
            for node in chunk.iter() {
                self.codes[node.literal as usize] =
                    Hcode::new(reverse_bits(code, n as u8), n as u8);
                code = code.wrapping_add(1);
            }
            list_len = start;
        }
    }

    /// `generate` rewrites h to be the Huffman code for the given frequency
    /// count. freq[i] is the frequency of literal i, and maxBits is the
    /// maximum number of bits to use for any literal.
    pub(crate) fn generate(&mut self, freq: &[u16], max_bits: i32) {
        let mut list = std::mem::take(&mut self.freqcache);
        // Number of non-zero literals
        let mut count = 0usize;
        // Set list to be the set of all non-zero literals and their frequencies
        for (i, &f) in freq.iter().enumerate() {
            if f != 0 {
                list[count] = LiteralNode {
                    literal: i as u16,
                    freq: f,
                };
                count += 1;
            } else {
                self.codes[i] = Hcode(0);
            }
        }
        list[count] = LiteralNode::default();

        if count <= 2 {
            // Handle the small cases here, because they are awkward for the
            // general case code. With two or fewer literals, everything has
            // bit length 1.
            for (i, node) in list[..count].iter().enumerate() {
                // "list" is in order of increasing literal value.
                self.codes[node.literal as usize] = Hcode::new(i as u16, 1);
            }
            self.freqcache = list;
            return;
        }
        // Go sorts by freq<<10 + literal: a total order over unique literals,
        // so any correct sort (Go's pdqsort included) gives the same result.
        list[..count].sort_unstable_by_key(|n| (u32::from(n.freq) << 10) + u32::from(n.literal));

        // Get the number of literals for each bit count
        let bit_count_len = self.bit_counts(&mut list, count, max_bits);
        // And do the assignment
        self.assign_encoding_and_size(bit_count_len, &mut list[..count]);
        self.freqcache = list;
    }
}

/// `generateFixedLiteralEncoding` returns the encoder for the fixed literal table.
fn generate_fixed_literal_encoding() -> HuffmanEncoder {
    let mut h = HuffmanEncoder::new(LITERAL_COUNT);
    for ch in 0..LITERAL_COUNT as u16 {
        let (bits, size): (u16, u8) = if ch < 144 {
            // size 8, 000110000  .. 10111111
            (ch + 48, 8)
        } else if ch < 256 {
            // size 9, 110010000 .. 111111111
            (ch + 400 - 144, 9)
        } else if ch < 280 {
            // size 7, 0000000 .. 0010111
            (ch - 256, 7)
        } else {
            // size 8, 11000000 .. 11000111
            (ch + 192 - 280, 8)
        };
        h.codes[ch as usize] = Hcode::new(reverse_bits(bits, size), size);
    }
    h
}

/// `generateFixedOffsetEncoding`.
fn generate_fixed_offset_encoding() -> HuffmanEncoder {
    let mut h = HuffmanEncoder::new(30);
    for (ch, c) in h.codes.iter_mut().enumerate() {
        *c = Hcode::new(reverse_bits(ch as u16, 5), 5);
    }
    h
}

/// `fixedLiteralEncoding`.
pub(crate) fn fixed_literal_encoding() -> &'static HuffmanEncoder {
    static ENC: OnceLock<HuffmanEncoder> = OnceLock::new();
    ENC.get_or_init(generate_fixed_literal_encoding)
}

/// `fixedOffsetEncoding`.
pub(crate) fn fixed_offset_encoding() -> &'static HuffmanEncoder {
    static ENC: OnceLock<HuffmanEncoder> = OnceLock::new();
    ENC.get_or_init(generate_fixed_offset_encoding)
}

/// `histogram` accumulates the byte histogram of b into h (Go's
/// histogramSplit only reorders the counting).
pub(crate) fn histogram(b: &[u8], h: &mut [u16]) {
    let h = &mut h[..256];
    for &t in b {
        h[t as usize] = h[t as usize].wrapping_add(1);
    }
}
