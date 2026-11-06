// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 src/compress/flate/level5.go.
// Copyright 2026 The Go Authors. All rights reserved. BSD-3-Clause; see
// THIRD_PARTY_NOTICES.md.

//! Level 5: like level 4, but two candidates are tested for long matches,
//! and the end of the best match is checked for a longer one.

use crate::deflate::MAX_MATCH_LENGTH;
use crate::deflatefast::{
    BASE_MATCH_OFFSET, BUFFER_RESET, FastGen, HASH_LONG_BYTES, MAX_MATCH_OFFSET, TABLE_BITS,
    TABLE_SIZE, TableEntry, TableEntryPrev, hash_len, load_le32, load_le64, shift_offset,
};
use crate::token::{Tokens, emit_literals};

/// `eLong.cur, eLong.prev = entry, eLong.cur`.
#[inline]
pub(crate) fn push_long(e: &mut TableEntryPrev, entry: TableEntry) {
    e.prev = e.cur;
    e.cur = entry;
}

/// Shift a two-candidate entry for the wraparound guard (levels 5 and 6).
#[inline]
pub(crate) fn shift_prev_entry(v: &mut TableEntryPrev, min_off: i32, cur: i32) {
    if v.cur.offset <= min_off {
        v.cur.offset = 0;
        v.prev.offset = 0;
    } else {
        v.cur.offset = v.cur.offset - cur + MAX_MATCH_OFFSET;
        v.prev.offset = shift_offset(v.prev.offset, min_off, cur);
    }
}

/// `fastEncL5`.
pub(crate) struct FastEncL5 {
    pub(crate) fast_gen: FastGen,
    table: Vec<TableEntry>,
    b_table: Vec<TableEntryPrev>,
}

impl FastEncL5 {
    pub(crate) fn new() -> Self {
        Self {
            fast_gen: FastGen::new(),
            table: vec![TableEntry::default(); TABLE_SIZE],
            b_table: vec![TableEntryPrev::default(); TABLE_SIZE],
        }
    }

    pub(crate) fn encode(&mut self, dst: &mut Tokens, src: &[u8]) {
        const INPUT_MARGIN: i32 = 12 - 1;
        const MIN_NON_LITERAL_BLOCK_SIZE: usize = 1 + 1 + INPUT_MARGIN as usize;
        const HASH_SHORT_BYTES: u8 = 4;

        // Protect against e.cur wraparound.
        while self.fast_gen.cur >= BUFFER_RESET {
            if self.fast_gen.hist.is_empty() {
                self.table.fill(TableEntry::default());
                self.b_table.fill(TableEntryPrev::default());
                self.fast_gen.cur = MAX_MATCH_OFFSET;
                break;
            }
            // Shift down everything in the table that isn't already too far away.
            let min_off = self.fast_gen.cur + self.fast_gen.hist.len() as i32 - MAX_MATCH_OFFSET;
            for e in self.table.iter_mut() {
                e.offset = shift_offset(e.offset, min_off, self.fast_gen.cur);
            }
            for v in self.b_table.iter_mut() {
                shift_prev_entry(v, min_off, self.fast_gen.cur);
            }
            self.fast_gen.cur = MAX_MATCH_OFFSET;
        }

        let mut s = self.fast_gen.add_block(src);

        // This check isn't in the Snappy implementation, but there, the caller
        // instead of the callee handles this case.
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
        let b_table = &mut self.b_table;

        // nextEmit is where in src the next emitLiterals should start from.
        let mut next_emit = s;

        // sLimit is when to stop looking for offset/length copies.
        let s_limit = src.len() as i32 - INPUT_MARGIN;

        let mut cv = load_le64(src, s);
        'outer: loop {
            const SKIP_LOG: i32 = 6;
            const DO_EVERY: i32 = 1;

            let mut next_s = s;
            let mut l: i32 = 0;
            let mut t: i32;
            loop {
                let mut next_hash_s = hash_len(cv, TABLE_BITS, HASH_SHORT_BYTES) as usize;
                let mut next_hash_l = hash_len(cv, TABLE_BITS, HASH_LONG_BYTES) as usize;

                s = next_s;
                next_s = s + DO_EVERY + ((s - next_emit) >> SKIP_LOG);
                if next_s > s_limit {
                    break 'outer;
                }
                // Fetch a short+long candidate
                let s_candidate = table[next_hash_s];
                let mut l_candidate = b_table[next_hash_l];
                let next = load_le64(src, next_s);
                let entry = TableEntry { offset: s + cur };
                table[next_hash_s] = entry;
                push_long(&mut b_table[next_hash_l], entry);

                next_hash_s = hash_len(next, TABLE_BITS, HASH_SHORT_BYTES) as usize;
                next_hash_l = hash_len(next, TABLE_BITS, HASH_LONG_BYTES) as usize;

                t = l_candidate.cur.offset - cur;
                if s - t < MAX_MATCH_OFFSET {
                    if cv as u32 == load_le32(src, t) {
                        // Store the next match
                        table[next_hash_s] = TableEntry {
                            offset: next_s + cur,
                        };
                        push_long(
                            &mut b_table[next_hash_l],
                            TableEntry {
                                offset: next_s + cur,
                            },
                        );

                        let t2 = l_candidate.prev.offset - cur;
                        if s - t2 < MAX_MATCH_OFFSET && cv as u32 == load_le32(src, t2) {
                            l = FastGen::match_len_limited(s + 4, t + 4, src) + 4;
                            let ml1 = FastGen::match_len_limited(s + 4, t2 + 4, src) + 4;
                            if ml1 > l {
                                t = t2;
                                l = ml1;
                                break;
                            }
                        }
                        break;
                    }
                    t = l_candidate.prev.offset - cur;
                    if s - t < MAX_MATCH_OFFSET && cv as u32 == load_le32(src, t) {
                        // Store the next match
                        table[next_hash_s] = TableEntry {
                            offset: next_s + cur,
                        };
                        push_long(
                            &mut b_table[next_hash_l],
                            TableEntry {
                                offset: next_s + cur,
                            },
                        );
                        break;
                    }
                }

                t = s_candidate.offset - cur;
                if s - t < MAX_MATCH_OFFSET && cv as u32 == load_le32(src, t) {
                    // Found a 4 match...
                    l = FastGen::match_len_limited(s + 4, t + 4, src) + 4;
                    l_candidate = b_table[next_hash_l];
                    // Store the next match

                    table[next_hash_s] = TableEntry {
                        offset: next_s + cur,
                    };
                    push_long(
                        &mut b_table[next_hash_l],
                        TableEntry {
                            offset: next_s + cur,
                        },
                    );

                    // If the next long is a candidate, use that...
                    let mut t2 = l_candidate.cur.offset - cur;
                    if next_s - t2 < MAX_MATCH_OFFSET {
                        if load_le32(src, t2) == next as u32 {
                            let ml = FastGen::match_len_limited(next_s + 4, t2 + 4, src) + 4;
                            if ml > l {
                                t = t2;
                                s = next_s;
                                l = ml;
                                break;
                            }
                        }
                        // If the previous long is a candidate, use that...
                        t2 = l_candidate.prev.offset - cur;
                        if next_s - t2 < MAX_MATCH_OFFSET && load_le32(src, t2) == next as u32 {
                            let ml = FastGen::match_len_limited(next_s + 4, t2 + 4, src) + 4;
                            if ml > l {
                                t = t2;
                                s = next_s;
                                l = ml;
                                break;
                            }
                        }
                    }
                    break;
                }
                cv = next;
            }

            if l == 0 {
                // Extend the 4-byte match as long as possible.
                l = FastGen::match_len_long(s + 4, t + 4, src) + 4;
            } else if l == MAX_MATCH_LENGTH as i32 {
                l += FastGen::match_len_long(s + l, t + l, src);
            }

            // Try to locate a better match by checking the end of best match...
            let s_at = s + l;
            if l < 30 && s_at < s_limit {
                // Allow some bytes at the beginning to mismatch.
                const SKIP_BEGINNING: i32 = 2;
                let e_long = b_table
                    [hash_len(load_le64(src, s_at), TABLE_BITS, HASH_LONG_BYTES) as usize]
                    .cur
                    .offset;
                let t2 = e_long - cur - l + SKIP_BEGINNING;
                let s2 = s + SKIP_BEGINNING;
                let off = s2 - t2;
                if t2 >= 0 && off < MAX_MATCH_OFFSET && off > 0 {
                    let l2 = FastGen::match_len_long(s2, t2, src);
                    if l2 > l {
                        t = t2;
                        l = l2;
                        s = s2;
                    }
                }
            }

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
                break 'outer;
            }

            // Store every 3rd hash in-between.
            const HASH_EVERY: i32 = 3;
            let mut i = s - l + 1;
            if i < s - 1 {
                let mut cv = load_le64(src, i);
                let mut t = TableEntry { offset: i + cur };
                table[hash_len(cv, TABLE_BITS, HASH_SHORT_BYTES) as usize] = t;
                push_long(
                    &mut b_table[hash_len(cv, TABLE_BITS, HASH_LONG_BYTES) as usize],
                    t,
                );

                // Do an long at i+1
                cv >>= 8;
                t = TableEntry {
                    offset: t.offset + 1,
                };
                push_long(
                    &mut b_table[hash_len(cv, TABLE_BITS, HASH_LONG_BYTES) as usize],
                    t,
                );

                // We only have enough bits for a short entry at i+2
                cv >>= 8;
                t = TableEntry {
                    offset: t.offset + 1,
                };
                table[hash_len(cv, TABLE_BITS, HASH_SHORT_BYTES) as usize] = t;

                // Skip one - otherwise we risk hitting 's'
                i += 4;
                while i < s - 1 {
                    let cv = load_le64(src, i);
                    let t = TableEntry { offset: i + cur };
                    let t2 = TableEntry {
                        offset: t.offset + 1,
                    };
                    push_long(
                        &mut b_table[hash_len(cv, TABLE_BITS, HASH_LONG_BYTES) as usize],
                        t,
                    );
                    table[hash_len(cv >> 8, TABLE_BITS, HASH_SHORT_BYTES) as usize] = t2;
                    i += HASH_EVERY;
                }
            }

            // We could immediately start working at s now, but to improve
            // compression we first update the hash table at s-1 and at s.
            let x = load_le64(src, s - 1);
            let o = cur + s - 1;
            let prev_hash_s = hash_len(x, TABLE_BITS, HASH_SHORT_BYTES) as usize;
            let prev_hash_l = hash_len(x, TABLE_BITS, HASH_LONG_BYTES) as usize;
            table[prev_hash_s] = TableEntry { offset: o };
            push_long(&mut b_table[prev_hash_l], TableEntry { offset: o });
            cv = x >> 8;
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
