// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 src/compress/flate/level1.go.
// Copyright 2026 The Go Authors. All rights reserved. BSD-3-Clause; see
// THIRD_PARTY_NOTICES.md.

//! Level 1 (BestSpeed): a single small table with 5 byte hashes.

use crate::deflatefast::{
    BASE_MATCH_LENGTH, BASE_MATCH_OFFSET, BUFFER_RESET, FastGen, MAX_MATCH_OFFSET, TABLE_BITS,
    TABLE_SIZE, TableEntry, hash_len, load_le32, load_le64, shift_offset,
};
use crate::token::{
    LENGTH_CODES1, LENGTH_SHIFT, MATCH_TYPE, Token, Tokens, emit_literals, offset_code,
};

/// `fastEncL1`.
pub(crate) struct FastEncL1 {
    pub(crate) fast_gen: FastGen,
    table: Vec<TableEntry>,
}

impl FastEncL1 {
    pub(crate) fn new() -> Self {
        Self {
            fast_gen: FastGen::new(),
            table: vec![TableEntry::default(); TABLE_SIZE],
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
            const SKIP_LOG: i32 = 5;
            const DO_EVERY: i32 = 2;

            let mut next_s;
            let mut candidate: TableEntry;
            let mut t: i32;
            loop {
                let mut next_hash = hash_len(cv, TABLE_BITS, HASH_BYTES);
                candidate = table[next_hash as usize];
                next_s = s + DO_EVERY + ((s - next_emit) >> SKIP_LOG);
                if next_s > s_limit {
                    break 'outer;
                }

                let mut now = load_le64(src, next_s);
                table[next_hash as usize] = TableEntry { offset: s + cur };
                next_hash = hash_len(now, TABLE_BITS, HASH_BYTES);
                t = candidate.offset - cur;
                if s - t < MAX_MATCH_OFFSET && cv as u32 == load_le32(src, t) {
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

                t = candidate.offset - cur;
                if s - t < MAX_MATCH_OFFSET && cv as u32 == load_le32(src, t) {
                    table[next_hash as usize] = TableEntry {
                        offset: next_s + cur,
                    };
                    break;
                }
                cv = now;
                s = next_s;
            }

            // A 4-byte match has been found. We'll later see if more than 4 bytes
            // match. But, prior to the match, src[nextEmit:s] are unmatched. Emit
            // them as literal bytes.
            loop {
                // Invariant: we have a 4-byte match at s, and no need to emit any
                // literal bytes prior to s.

                // Extend the 4-byte match as long as possible.
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

                // Save the match found. Same as 'dst.AddMatchLong(l, uint32(s-t-baseMatchOffset))'
                let mut x_offset = (s - t - BASE_MATCH_OFFSET) as u32;
                let mut x_length = l;
                let oc = offset_code(x_offset);
                x_offset |= oc << 16;
                while x_length > 0 {
                    let mut xl = x_length;
                    if xl > 258 {
                        if xl > 258 + BASE_MATCH_LENGTH {
                            xl = 258;
                        } else {
                            xl = 258 - BASE_MATCH_LENGTH;
                        }
                    }
                    x_length -= xl;
                    xl -= BASE_MATCH_LENGTH;
                    let e = LENGTH_CODES1[(xl as u8) as usize] as usize;
                    dst.extra_hist[e] = dst.extra_hist[e].wrapping_add(1);
                    dst.off_hist[oc as usize] = dst.off_hist[oc as usize].wrapping_add(1);
                    dst.tokens[dst.n as usize] =
                        (MATCH_TYPE | (xl as u32) << LENGTH_SHIFT | x_offset) as Token;
                    dst.n = dst.n.wrapping_add(1);
                }
                s += l;
                next_emit = s;
                if next_s >= s {
                    s = next_s + 1;
                }
                if s >= s_limit {
                    // Index first pair after match end.
                    if ((s + l + 8) as usize) < src.len() {
                        let cv = load_le64(src, s);
                        table[hash_len(cv, TABLE_BITS, HASH_BYTES) as usize] =
                            TableEntry { offset: s + cur };
                    }
                    break 'outer;
                }

                // We could immediately start working at s now, but to improve
                // compression we first update the hash table at s-2 and at s.
                let mut x = load_le64(src, s - 2);
                let o = cur + s - 2;
                let prev_hash = hash_len(x, TABLE_BITS, HASH_BYTES);
                table[prev_hash as usize] = TableEntry { offset: o };
                x >>= 16;
                let curr_hash = hash_len(x, TABLE_BITS, HASH_BYTES);
                candidate = table[curr_hash as usize];
                table[curr_hash as usize] = TableEntry { offset: o + 2 };

                t = candidate.offset - cur;
                if s - t > MAX_MATCH_OFFSET || x as u32 != load_le32(src, t) {
                    cv = x >> 8;
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
