// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 src/compress/flate/level2.go.
// Copyright 2026 The Go Authors. All rights reserved. BSD-3-Clause; see
// THIRD_PARTY_NOTICES.md.

//! Level 2: like level 1, but with a larger table.

use crate::deflatefast::{
    BASE_MATCH_OFFSET, BUFFER_RESET, FastGen, MAX_MATCH_OFFSET, TableEntry, hash_len, load_le32,
    load_le64, shift_offset,
};
use crate::token::{Tokens, emit_literals};

/// Bits used in level 2 table
const L2_TABLE_BITS: u8 = 17;
/// Size of the level 2 table
const L2_TABLE_SIZE: usize = 1 << L2_TABLE_BITS;

/// `fastEncL2`.
pub(crate) struct FastEncL2 {
    pub(crate) fast_gen: FastGen,
    table: Vec<TableEntry>,
}

impl FastEncL2 {
    pub(crate) fn new() -> Self {
        Self {
            fast_gen: FastGen::new(),
            table: vec![TableEntry::default(); L2_TABLE_SIZE],
        }
    }

    pub(crate) fn encode(&mut self, dst: &mut Tokens, src: &[u8]) {
        const INPUT_MARGIN: i32 = 12 - 1;
        const MIN_NON_LITERAL_BLOCK_SIZE: usize = 1 + 1 + INPUT_MARGIN as usize;
        const HASH_BYTES: u8 = 5;

        // Protect against e.cur wraparound.
        while self.fast_gen.cur >= BUFFER_RESET {
            if self.fast_gen.hist.is_empty() {
                self.table.fill(TableEntry::default());
                self.fast_gen.cur = MAX_MATCH_OFFSET;
                break;
            }
            // Shift down everything in the table that isn't already too far away.
            let min_off = self.fast_gen.cur + self.fast_gen.hist.len() as i32 - MAX_MATCH_OFFSET;
            for e in self.table.iter_mut() {
                e.offset = shift_offset(e.offset, min_off, self.fast_gen.cur);
            }
            self.fast_gen.cur = MAX_MATCH_OFFSET;
        }

        let mut s = self.fast_gen.add_block(src);

        if src.len() < MIN_NON_LITERAL_BLOCK_SIZE {
            // We do not fill the token table.
            // This will be picked up by caller.
            dst.n = src.len() as u16;
            return;
        }

        // Override src
        let src = &self.fast_gen.hist[..];
        let cur = self.fast_gen.cur;
        let table = &mut self.table;

        // nextEmit is where in src the next emitLiterals should start from.
        let mut next_emit = s;

        // sLimit is when to stop looking for offset/length copies.
        let s_limit = src.len() as i32 - INPUT_MARGIN;

        let mut cv = load_le64(src, s);
        'outer: loop {
            // When should we start skipping if we haven't found matches in a long while.
            const SKIP_LOG: i32 = 5;
            const DO_EVERY: i32 = 2;

            let mut next_s = s;
            let mut candidate: TableEntry;
            loop {
                let mut next_hash = hash_len(cv, L2_TABLE_BITS, HASH_BYTES);
                s = next_s;
                next_s = s + DO_EVERY + ((s - next_emit) >> SKIP_LOG);
                if next_s > s_limit {
                    break 'outer;
                }
                candidate = table[next_hash as usize];
                let mut now = load_le64(src, next_s);
                table[next_hash as usize] = TableEntry { offset: s + cur };
                next_hash = hash_len(now, L2_TABLE_BITS, HASH_BYTES);

                let mut offset = s - (candidate.offset - cur);
                if offset < MAX_MATCH_OFFSET && cv as u32 == load_le32(src, candidate.offset - cur)
                {
                    table[next_hash as usize] = TableEntry {
                        offset: next_s + cur,
                    };
                    break;
                }

                // Do one right away...
                cv = now;
                s = next_s;
                next_s += 1;
                candidate = table[next_hash as usize];
                now >>= 8;
                table[next_hash as usize] = TableEntry { offset: s + cur };

                offset = s - (candidate.offset - cur);
                if offset < MAX_MATCH_OFFSET && cv as u32 == load_le32(src, candidate.offset - cur)
                {
                    break;
                }
                cv = now;
            }

            // A 4-byte match has been found. We'll later see if more than 4 bytes match.
            loop {
                // Extend the 4-byte match as long as possible.
                let mut t = candidate.offset - cur;
                let mut l = FastGen::match_len_long(s + 4, t + 4, src) + 4;

                // Extend backwards
                while t > 0 && s > next_emit && src[(t - 1) as usize] == src[(s - 1) as usize] {
                    s -= 1;
                    t -= 1;
                    l += 1;
                }
                if next_emit < s {
                    emit_literals(dst, &src[next_emit as usize..s as usize]);
                }

                dst.add_match_long(l, (s - t - BASE_MATCH_OFFSET) as u32);
                s += l;
                next_emit = s;
                if next_s >= s {
                    s = next_s + 1;
                }

                if s >= s_limit {
                    // Index first pair after match end.
                    if ((s + l + 8) as usize) < src.len() {
                        let cv = load_le64(src, s);
                        table[hash_len(cv, L2_TABLE_BITS, HASH_BYTES) as usize] =
                            TableEntry { offset: s + cur };
                    }
                    break 'outer;
                }

                // Store every second hash in-between, but offset by 1.
                let mut i = s - l + 2;
                while i < s - 5 {
                    let mut x = load_le64(src, i);
                    let mut next_hash = hash_len(x, L2_TABLE_BITS, HASH_BYTES);
                    table[next_hash as usize] = TableEntry { offset: cur + i };
                    // Skip one
                    x >>= 16;
                    next_hash = hash_len(x, L2_TABLE_BITS, HASH_BYTES);
                    table[next_hash as usize] = TableEntry {
                        offset: cur + i + 2,
                    };
                    // Skip one
                    x >>= 16;
                    next_hash = hash_len(x, L2_TABLE_BITS, HASH_BYTES);
                    table[next_hash as usize] = TableEntry {
                        offset: cur + i + 4,
                    };
                    i += 7;
                }

                // We could immediately start working at s now, but to improve
                // compression we first update the hash table at s-2 to s.
                let x = load_le64(src, s - 2);
                let o = cur + s - 2;
                let prev_hash = hash_len(x, L2_TABLE_BITS, HASH_BYTES);
                let prev_hash2 = hash_len(x >> 8, L2_TABLE_BITS, HASH_BYTES);
                table[prev_hash as usize] = TableEntry { offset: o };
                table[prev_hash2 as usize] = TableEntry { offset: o + 1 };
                let curr_hash = hash_len(x >> 16, L2_TABLE_BITS, HASH_BYTES);
                candidate = table[curr_hash as usize];
                table[curr_hash as usize] = TableEntry { offset: o + 2 };

                let offset = s - (candidate.offset - cur);
                if offset > MAX_MATCH_OFFSET
                    || (x >> 16) as u32 != load_le32(src, candidate.offset - cur)
                {
                    cv = x >> 24;
                    s += 1;
                    break;
                }
            }
        }

        // emitRemainder:
        if (next_emit as usize) < src.len() {
            // If nothing was added, don't encode literals.
            if dst.n == 0 {
                return;
            }
            emit_literals(dst, &src[next_emit as usize..]);
        }
    }
}
