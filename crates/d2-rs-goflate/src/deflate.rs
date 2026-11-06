// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 src/compress/flate/deflate.go.
// Copyright 2009 The Go Authors. All rights reserved. BSD-3-Clause; see
// THIRD_PARTY_NOTICES.md.

//! The DEFLATE compressor (`deflate.go`): level dispatch, the level 1-6
//! fast path (`deflateFast`), the level 7-9 lazy matcher with hash chains
//! (`deflateLazy`), store and Huffman-only modes.
//!
//! Not translated: preset dictionaries (`NewWriterDict`/`fillWindow`),
//! `Reset`, and the unused `writeBlockSkip`. `compress/zlib` passes a nil
//! dictionary, for which `fillWindow` has no effect.

use std::io::{self, Write};

use crate::deflatefast::{FastEnc, hash4u, load_le32, match_len};
use crate::huffman_bit_writer::{
    HuffmanBitWriter, LENGTH_EXTRA_BITS, OFFSET_EXTRA_BITS, clone_err,
};
use crate::huffman_code::HuffmanEncoder;
use crate::token::{LENGTH_CODES, Tokens, offset_code};

/// `NoCompression`.
pub const NO_COMPRESSION: i32 = 0;
/// `BestSpeed`.
pub const BEST_SPEED: i32 = 1;
/// `BestCompression`.
pub const BEST_COMPRESSION: i32 = 9;
/// `DefaultCompression` (level 6).
pub const DEFAULT_COMPRESSION: i32 = -1;
/// `HuffmanOnly` disables Lempel-Ziv match searching and only performs
/// Huffman entropy encoding.
pub const HUFFMAN_ONLY: i32 = -2;

const LOG_WINDOW_SIZE: i32 = 15;
const WINDOW_SIZE: i32 = 1 << LOG_WINDOW_SIZE;
const WINDOW_MASK: i32 = WINDOW_SIZE - 1;
/// The smallest match that the compressor looks for
const MIN_MATCH_LENGTH: i32 = 4;
/// The longest match for the compressor
pub(crate) const MAX_MATCH_LENGTH: usize = 258;
/// The shortest offset that makes any sense
const MIN_OFFSET_SIZE: i32 = 1;

/// The maximum number of tokens we will encode at the time (levels 7-9).
const MAX_FLATE_BLOCK_TOKENS: u16 = 1 << 15;
pub(crate) const MAX_STORE_BLOCK_SIZE: usize = 65535;
/// After 17 performance degrades
const HASH_BITS: u8 = 17;
const HASH_SIZE: usize = 1 << HASH_BITS;
const HASH_MASK: u32 = (1 << HASH_BITS) - 1;
const MAX_HASH_OFFSET: i32 = 1 << 28;

/// `compressionLevel` holds the parameters for levels 7-9.
#[derive(Clone, Copy, Debug, Default)]
struct CompressionLevel {
    /// "good enough" match length (unused by the Go 1.27 matcher)
    #[allow(dead_code)]
    good: i32,
    /// don't try to find a later, better match above this length
    lazy: i32,
    /// stop looking for a better match above this length
    nice: i32,
    /// maximum number of hash chain entries to search
    chain: i32,
    level: i32,
}

const fn lvl(good: i32, lazy: i32, nice: i32, chain: i32, level: i32) -> CompressionLevel {
    CompressionLevel {
        good,
        lazy,
        nice,
        chain,
        level,
    }
}

/// `levels`.
static LEVELS: [CompressionLevel; 10] = [
    lvl(0, 0, 0, 0, 0), // 0
    // Level 1-6 uses specialized algorithm - values not used
    lvl(0, 0, 0, 0, 1),
    lvl(0, 0, 0, 0, 2),
    lvl(0, 0, 0, 0, 3),
    lvl(0, 0, 0, 0, 4),
    lvl(0, 0, 0, 0, 5),
    lvl(0, 0, 0, 0, 6),
    // Levels 7-9 use increasingly more lazy matching
    // and increasingly stringent conditions for "good enough".
    lvl(8, 12, 16, 24, 7),
    lvl(16, 30, 40, 64, 8),
    lvl(32, 258, 258, 1024, 9),
];

/// `advancedState` contains state for levels 7-9.
struct AdvancedState {
    // deflate state
    length: i32,
    offset: i32,
    max_insert_index: i32,
    chain_head: i32,
    hash_offset: i32,

    /// consecutive literal count; overflows to reset after 64KB.
    literal_counter: u16,

    // input window: unprocessed data is window[index:windowEnd]
    index: i32,
    hash_match: [u32; MAX_MATCH_LENGTH + MIN_MATCH_LENGTH as usize],

    // Input hash chains
    // hashHead[hashValue] contains the largest inputIndex with the specified hash value
    // If hashHead[hashValue] is within the current window, then
    // hashPrev[hashHead[hashValue] & windowMask] contains the previous index
    // with the same hash value.
    hash_head: Vec<i32>,
    hash_prev: Vec<i32>,
}

/// Go's `fill`/`step` function-pointer pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// fillBlock + store
    Store,
    /// fillBlock + deflateHuff
    Huff,
    /// fillBlock + deflateFast
    Fast,
    /// fillDeflate + deflateLazy
    Lazy,
}

/// Sticky compressor error state (Go `d.err`, including `errWriterClosed`).
enum ErrState {
    None,
    Closed,
    Io(io::Error),
}

/// `compressor`.
struct Compressor<W: Write> {
    cl: CompressionLevel,
    mode: Mode,

    /// huffman encoder, with state (level 9 literal cost estimates)
    h: Option<HuffmanEncoder>,
    /// writer for blocks
    w: HuffmanBitWriter<W>,

    /// current window - size depends on encoder level
    window: Vec<u8>,
    /// filled bytes in window
    window_end: i32,
    /// window index where current tokens start
    block_start: i32,
    err: ErrState,

    /// tokens store for each block
    tokens: Tokens,
    /// encoder to use for blocks
    fast: Option<FastEnc>,
    /// chained encoder for level 7-9
    state: Option<Box<AdvancedState>>,

    /// requesting flush
    sync: bool,
    /// if true, still need to process window[index-1].
    byte_available: bool,
}

/// `hash4` returns a hash representation of the first 4 bytes of b.
#[inline]
fn hash4(b: &[u8]) -> u32 {
    hash4u(load_le32(b, 0), HASH_BITS)
}

/// `bulkHash4` sets dst[i] = hash4(b[i:i+4]) for all i <= len(b)-4.
fn bulk_hash4(b: &[u8], dst: &mut [u32]) {
    if b.len() < 4 {
        return;
    }
    let mut hb = load_le32(b, 0);

    dst[0] = hash4u(hb, HASH_BITS);
    let end = b.len() - 4 + 1;
    for i in 1..end {
        hb = (hb >> 8) | u32::from(b[i + 3]) << 24;
        dst[i] = hash4u(hb, HASH_BITS);
    }
}

impl<W: Write> Compressor<W> {
    fn err_result(&self) -> io::Result<()> {
        match &self.err {
            ErrState::Io(e) => Err(clone_err(e)),
            ErrState::Closed => Err(io::Error::other("flate: closed writer")),
            ErrState::None => Ok(()),
        }
    }

    /// `d.err = <result>`.
    fn set_err(&mut self, r: io::Result<()>) {
        self.err = match r {
            Ok(()) => ErrState::None,
            Err(e) => ErrState::Io(e),
        };
    }

    /// `d.w.err` as a result.
    fn writer_err(&self) -> io::Result<()> {
        match &self.w.err {
            Some(e) => Err(clone_err(e)),
            None => Ok(()),
        }
    }

    fn fill(&mut self, b: &[u8]) -> usize {
        match self.mode {
            Mode::Lazy => self.fill_deflate(b),
            _ => self.fill_block(b),
        }
    }

    fn step(&mut self) {
        match self.mode {
            Mode::Store => self.store(),
            Mode::Huff => self.deflate_huff(),
            Mode::Fast => self.deflate_fast(),
            Mode::Lazy => self.deflate_lazy(),
        }
    }

    /// `fillDeflate` will add b to the current window for levels 7-9.
    fn fill_deflate(&mut self, b: &[u8]) -> usize {
        let s = self.state.as_mut().expect("advanced state");
        if s.index >= 2 * WINDOW_SIZE - (MIN_MATCH_LENGTH + MAX_MATCH_LENGTH as i32) {
            // shift the window by windowSize
            self.window
                .copy_within(WINDOW_SIZE as usize..2 * WINDOW_SIZE as usize, 0);
            s.index -= WINDOW_SIZE;
            self.window_end -= WINDOW_SIZE;
            if self.block_start >= WINDOW_SIZE {
                self.block_start -= WINDOW_SIZE;
            } else {
                self.block_start = i32::MAX;
            }
            s.hash_offset += WINDOW_SIZE;
            if s.hash_offset > MAX_HASH_OFFSET {
                let delta = s.hash_offset - 1;
                s.hash_offset -= delta;
                s.chain_head -= delta;
                for v in s.hash_prev.iter_mut() {
                    *v = (*v - delta).max(0);
                }
                for v in s.hash_head.iter_mut() {
                    *v = (*v - delta).max(0);
                }
            }
        }
        self.fill_block(b)
    }

    /// `writeBlock` will write tokens to output. The provided index is where
    /// the block starts in d.window.
    fn write_block(&mut self, index: i32, eof: bool) -> io::Result<()> {
        if index > 0 || eof {
            let window = if self.block_start <= index {
                Some(&self.window[self.block_start as usize..index as usize])
            } else {
                None
            };
            self.block_start = index;
            self.w
                .write_block_dynamic(&mut self.tokens, eof, window, self.sync);
            return self.writer_err();
        }
        Ok(())
    }

    /// `findMatch` finds the longest match starting at pos in the hash chain
    /// starting at prevHead. It searches up to d.chain entries in the chain.
    fn find_match(&self, pos: i32, prev_head: i32, lookahead: i32) -> (i32, i32, bool) {
        let s = self.state.as_ref().expect("advanced state");
        let min_match_look = lookahead.min(MAX_MATCH_LENGTH as i32);

        let win = &self.window[0..(pos + min_match_look) as usize];

        // We quit when we get a match that's at least nice long
        let nice = self.cl.nice.min(win.len() as i32 - pos);

        let mut tries = self.cl.chain;
        let mut length = MIN_MATCH_LENGTH - 1;
        let mut offset = 0;
        let mut ok = false;

        let mut w_end = win[(pos + length) as usize];
        let w_pos = &win[pos as usize..];
        let min_index = (pos - WINDOW_SIZE).max(0);

        // Minimum gain to accept a match.
        let mut c_gain: i64 = 4;

        // Some like it higher (CSV), some like it lower (JSON)
        const BASE_COST: i64 = 3;

        let mut i = prev_head;
        'chain: while tries > 0 {
            'next: {
                if w_end == win[(i + length) as usize] {
                    let n =
                        match_len(&win[i as usize..(i + min_match_look) as usize], w_pos) as i32;
                    if n > length {
                        if self.cl.chain >= 100 {
                            // Calculate gain. Estimates the gains of the new
                            // match compared to emitting as literals.
                            let h = self.h.as_ref().expect("literal cost encoder");
                            let new_gain = h.bit_length_raw(&w_pos[..n as usize])
                                - i64::from(
                                    OFFSET_EXTRA_BITS[offset_code((pos - i) as u32) as usize],
                                )
                                - BASE_COST
                                - i64::from(
                                    LENGTH_EXTRA_BITS
                                        [LENGTH_CODES[((n - 3) & 255) as usize] as usize],
                                );
                            if new_gain <= c_gain {
                                break 'next;
                            }
                            c_gain = new_gain;
                        }
                        length = n;
                        offset = pos - i;
                        ok = true;
                        if n >= nice {
                            // The match is good enough that we don't try to find a better one.
                            break 'chain;
                        }
                        w_end = win[(pos + n) as usize];
                    }
                }
            }
            // next:
            if i <= min_index {
                // hashPrev[i & windowMask] has already been overwritten, so stop now.
                break;
            }
            i = s.hash_prev[(i & WINDOW_MASK) as usize] - s.hash_offset;
            if i < min_index {
                break;
            }
            tries -= 1;
        }
        (length, offset, ok)
    }

    /// `writeStoredBlock` writes an uncompressed block of window[..end].
    fn write_stored_block(&mut self, end: usize) -> io::Result<()> {
        self.w.write_stored_header(end, false);
        self.writer_err()?;
        self.w.write_bytes(&self.window[..end]);
        self.writer_err()
    }

    /// `initDeflate` initializes d for levels 7-9.
    fn init_deflate(&mut self) {
        self.window = vec![0; 2 * WINDOW_SIZE as usize];
        self.byte_available = false;
        self.err = ErrState::None;
        let Some(s) = self.state.as_mut() else {
            return;
        };
        s.index = 0;
        s.hash_offset = 1;
        s.length = MIN_MATCH_LENGTH - 1;
        s.offset = 0;
        s.chain_head = -1;
    }

    /// Insert window[index..] into the hash chains (Go's repeated inline
    /// sequence): chainHead = hashHead[h]; hashPrev[index] = chainHead;
    /// hashHead[h] = index + hashOffset.
    #[inline]
    fn insert_index(&mut self) {
        let s = self.state.as_mut().expect("advanced state");
        let h = hash4(&self.window[s.index as usize..]) as usize;
        let ch = s.hash_head[h];
        s.chain_head = ch;
        s.hash_prev[(s.index & WINDOW_MASK) as usize] = ch;
        s.hash_head[h] = s.index + s.hash_offset;
    }

    /// Append a literal and write the block once it is full.
    #[inline]
    fn add_literal_checked(&mut self, lit: u8) -> bool {
        self.tokens.add_literal(lit);
        if self.tokens.n == MAX_FLATE_BLOCK_TOKENS {
            let index = self.state.as_ref().expect("advanced state").index;
            let r = self.write_block(index, false);
            let failed = r.is_err();
            self.set_err(r);
            if failed {
                return false;
            }
            self.tokens.reset();
        }
        true
    }

    /// `tryBetterMatchAtEnd` checks whether a better match exists at the end
    /// of the previous match and, if so, emits the skipped literals and
    /// adjusts the match. Returns the (possibly updated) prevLength and
    /// prevOffset.
    fn try_better_match_at_end(
        &mut self,
        mut prev_length: i32,
        mut prev_offset: i32,
        lookahead: i32,
    ) -> (i32, i32) {
        // We start checking at checkOff from the current match position.
        const CHECK_OFF: i32 = 2;
        let s = self.state.as_ref().expect("advanced state");

        if prev_length >= MAX_MATCH_LENGTH as i32 - CHECK_OFF {
            return (prev_length, prev_offset);
        }
        let prev_index = s.index - 1;
        if prev_index + prev_length >= s.max_insert_index {
            return (prev_length, prev_offset);
        }

        let end = (lookahead.min(MAX_MATCH_LENGTH as i32 + CHECK_OFF) + prev_index) as usize;
        let min_index = (s.index - WINDOW_SIZE).max(0);

        let h = hash4(&self.window[(prev_index + prev_length) as usize..]) as usize;
        let ch2 = s.hash_head[h] - s.hash_offset - prev_length;
        if prev_index - ch2 == prev_offset || ch2 <= min_index + CHECK_OFF {
            return (prev_length, prev_offset);
        }

        let length = match_len(
            &self.window[(prev_index + CHECK_OFF) as usize..end],
            &self.window[(ch2 + CHECK_OFF) as usize..],
        ) as i32;
        if length <= prev_length {
            return (prev_length, prev_offset);
        }

        prev_length = length;
        prev_offset = prev_index - ch2;

        let mut i = CHECK_OFF - 1;
        while i >= 0 {
            if prev_length >= MAX_MATCH_LENGTH as i32
                || self.window[(prev_index + i) as usize] != self.window[(ch2 + i) as usize]
            {
                for j in 0..=i {
                    let lit = self.window[(prev_index + j) as usize];
                    if !self.add_literal_checked(lit) {
                        return (prev_length, prev_offset);
                    }
                    let s = self.state.as_mut().expect("advanced state");
                    s.index += 1;
                    if s.index < s.max_insert_index {
                        self.insert_index();
                    }
                }
                break;
            }
            prev_length += 1;
            i -= 1;
        }
        (prev_length, prev_offset)
    }

    /// `skipLiterals` emits extra literal bytes during long runs of
    /// incompressible data. Returns false on write error.
    fn skip_literals(&mut self) -> bool {
        let s = self.state.as_ref().expect("advanced state");
        let mut n = i32::from(s.literal_counter) - self.cl.chain;
        if n <= 0 {
            return true;
        }
        n = 1 + (n >> 6);
        for _ in 0..n {
            let s = self.state.as_ref().expect("advanced state");
            if s.index >= self.window_end - 1 {
                break;
            }
            let lit = self.window[(s.index - 1) as usize];
            if !self.add_literal_checked(lit) {
                return false;
            }
            let s = self.state.as_ref().expect("advanced state");
            if s.index < s.max_insert_index {
                self.insert_index();
            }
            self.state.as_mut().expect("advanced state").index += 1;
        }
        let index = self.state.as_ref().expect("advanced state").index;
        self.tokens.add_literal(self.window[(index - 1) as usize]);
        self.byte_available = false;
        if self.tokens.n == MAX_FLATE_BLOCK_TOKENS {
            let r = self.write_block(index, false);
            let failed = r.is_err();
            self.set_err(r);
            if failed {
                return false;
            }
            self.tokens.reset();
        }
        true
    }

    /// `deflateLazy` encodes the current window using lazy matching
    /// (levels 7-9).
    fn deflate_lazy(&mut self) {
        {
            let s = self.state.as_ref().expect("advanced state");
            if self.window_end - s.index < MIN_MATCH_LENGTH + MAX_MATCH_LENGTH as i32 && !self.sync
            {
                return;
            }
            if self.window_end != s.index && self.cl.chain > 100 {
                // Get literal huffman coder.
                // This is used to estimate the cost of emitting a literal.
                if self.h.is_none() {
                    self.h = Some(HuffmanEncoder::new(MAX_FLATE_BLOCK_TOKENS as usize));
                }
                let mut tmp = [0u16; 256];
                let to_index = &self.window[s.index as usize..self.window_end as usize];
                let to_index = &to_index[..to_index.len().min(MAX_FLATE_BLOCK_TOKENS as usize)];
                for &v in to_index {
                    tmp[v as usize] = tmp[v as usize].wrapping_add(1);
                }
                self.h.as_mut().unwrap().generate(&tmp, 15);
            }
        }

        {
            let we = self.window_end;
            let s = self.state.as_mut().expect("advanced state");
            s.max_insert_index = we - (MIN_MATCH_LENGTH - 1);
        }

        loop {
            let (index, max_insert_index) = {
                let s = self.state.as_ref().expect("advanced state");
                (s.index, s.max_insert_index)
            };
            let lookahead = self.window_end - index;
            if lookahead < MIN_MATCH_LENGTH + MAX_MATCH_LENGTH as i32 {
                if !self.sync {
                    return;
                }
                if lookahead == 0 {
                    // Flush current output block if any.
                    if self.byte_available {
                        // There is still one pending token that needs to be flushed
                        self.tokens.add_literal(self.window[(index - 1) as usize]);
                        self.byte_available = false;
                    }
                    if self.tokens.n > 0 {
                        let r = self.write_block(index, false);
                        let failed = r.is_err();
                        self.set_err(r);
                        if failed {
                            return;
                        }
                        self.tokens.reset();
                    }
                    return;
                }
            }
            if index < max_insert_index {
                self.insert_index();
            }
            let (prev_length, prev_offset, chain_head, hash_offset) = {
                let s = self.state.as_mut().expect("advanced state");
                let r = (s.length, s.offset, s.chain_head, s.hash_offset);
                s.length = MIN_MATCH_LENGTH - 1;
                s.offset = 0;
                r
            };
            let min_index = (index - WINDOW_SIZE).max(0);

            if chain_head - hash_offset >= min_index
                && lookahead > prev_length
                && prev_length < self.cl.lazy
            {
                let (new_length, new_offset, ok) =
                    self.find_match(index, chain_head - hash_offset, lookahead);
                if ok {
                    let s = self.state.as_mut().expect("advanced state");
                    s.length = new_length;
                    s.offset = new_offset;
                }
            }

            let cur_length = self.state.as_ref().expect("advanced state").length;
            if prev_length >= MIN_MATCH_LENGTH && cur_length <= prev_length {
                let (prev_length, prev_offset) =
                    self.try_better_match_at_end(prev_length, prev_offset, lookahead);
                if !matches!(self.err, ErrState::None) {
                    return;
                }

                // There was a match at the previous step, and the current match is
                // not better. Output the previous match.
                self.tokens.add_match(
                    (prev_length - 3) as u32,
                    (prev_offset - MIN_OFFSET_SIZE) as u32,
                );

                // Insert in the hash table all strings up to the end of the match.
                // index and index-1 are already inserted. If there is not enough
                // lookahead, the last two strings are not inserted into the hash
                // table.
                let s = self.state.as_mut().expect("advanced state");
                let new_index = s.index + prev_length - 1;
                let mut end = new_index.min(s.max_insert_index);
                end += MIN_MATCH_LENGTH - 1;
                let startindex = (s.index + 1).min(s.max_insert_index);
                let tocheck = &self.window[startindex as usize..end as usize];
                let dst_size = tocheck.len() as i32 - MIN_MATCH_LENGTH + 1;
                if dst_size > 0 {
                    let dst = &mut s.hash_match[..dst_size as usize];
                    bulk_hash4(tocheck, dst);
                    for (i, &val) in dst.iter().enumerate() {
                        let di = i as i32 + startindex;
                        let new_h = (val & HASH_MASK) as usize;
                        s.hash_prev[(di & WINDOW_MASK) as usize] = s.hash_head[new_h];
                        s.hash_head[new_h] = di + s.hash_offset;
                    }
                }

                s.index = new_index;
                self.byte_available = false;
                s.length = MIN_MATCH_LENGTH - 1;
                if self.tokens.n == MAX_FLATE_BLOCK_TOKENS {
                    let r = self.write_block(new_index, false);
                    let failed = r.is_err();
                    self.set_err(r);
                    if failed {
                        return;
                    }
                    self.tokens.reset();
                }
                self.state.as_mut().unwrap().literal_counter = 0;
                continue;
            }
            if cur_length >= MIN_MATCH_LENGTH {
                self.state.as_mut().unwrap().literal_counter = 0;
            }
            if self.byte_available {
                {
                    let s = self.state.as_mut().unwrap();
                    s.literal_counter = s.literal_counter.wrapping_add(1);
                }
                let lit = self.window[(index - 1) as usize];
                if !self.add_literal_checked(lit) {
                    return;
                }
                self.state.as_mut().unwrap().index += 1;
                if !self.skip_literals() {
                    return;
                }
            } else {
                self.state.as_mut().unwrap().index += 1;
                self.byte_available = true;
            }
        }
    }

    /// `store` will store the current window if it has filled or if we are
    /// in sync.
    fn store(&mut self) {
        if self.window_end > 0 && (self.window_end as usize == MAX_STORE_BLOCK_SIZE || self.sync) {
            let r = self.write_stored_block(self.window_end as usize);
            self.set_err(r);
            self.window_end = 0;
        }
    }

    /// `fillBlock` appends b to d.window, returning the number of bytes copied.
    fn fill_block(&mut self, b: &[u8]) -> usize {
        let we = self.window_end as usize;
        let n = b.len().min(self.window.len() - we);
        self.window[we..we + n].copy_from_slice(&b[..n]);
        self.window_end += n as i32;
        n
    }

    /// `deflateHuff` compresses and stores the current window (if it has
    /// filled or if we are in sync or flush), using Huffman-only encoding.
    fn deflate_huff(&mut self) {
        if (self.window_end as usize) < self.window.len() && !self.sync || self.window_end == 0 {
            return;
        }
        self.w
            .write_block_huff(false, &self.window[..self.window_end as usize], self.sync);
        let r = self.writer_err();
        self.set_err(r);
        self.window_end = 0;
    }

    /// `deflateFast` encodes the current window if it has filled or if we are
    /// doing sync/flush, using the level 1-6 fast encoding.
    fn deflate_fast(&mut self) {
        let we = self.window_end as usize;
        // We only compress if we have maxStoreBlockSize.
        if we < self.window.len() {
            if !self.sync {
                return;
            }
            // Handle extremely small sizes.
            if we < 128 {
                if we == 0 {
                    return;
                }
                if we <= 32 {
                    let r = self.write_stored_block(we);
                    self.set_err(r);
                } else {
                    self.w.write_block_huff(false, &self.window[..we], true);
                    let r = self.writer_err();
                    self.set_err(r);
                }
                self.tokens.reset();
                self.window_end = 0;
                self.fast.as_mut().expect("fast encoder").reset();
                return;
            }
        }

        self.fast
            .as_mut()
            .expect("fast encoder")
            .encode(&mut self.tokens, &self.window[..we]);
        // If we made zero matches, store the block as is.
        if self.tokens.n == 0 {
            let r = self.write_stored_block(we);
            self.set_err(r);
        // If we removed less than 1/16th, huffman compress the block.
        } else if i32::from(self.tokens.n) > self.window_end - (self.window_end >> 4) {
            self.w
                .write_block_huff(false, &self.window[..we], self.sync);
            let r = self.writer_err();
            self.set_err(r);
        } else {
            self.w.write_block_dynamic(
                &mut self.tokens,
                false,
                Some(&self.window[..we]),
                self.sync,
            );
            let r = self.writer_err();
            self.set_err(r);
        }
        self.tokens.reset();
        self.window_end = 0;
    }

    /// `write` adds b to the compressor.
    fn write(&mut self, mut b: &[u8]) -> io::Result<usize> {
        self.err_result()?;
        let n = b.len();
        while !b.is_empty() {
            if self.window_end as usize == self.window.len() || self.sync {
                self.step();
            }
            let filled = self.fill(b);
            b = &b[filled..];
            self.err_result()?;
        }
        self.err_result()?;
        Ok(n)
    }

    /// `syncFlush` will flush the compressor by writing any remaining window
    /// and writing a stored block to byte-align the output.
    fn sync_flush(&mut self) -> io::Result<()> {
        self.err_result()?;
        self.sync = true;
        self.step();
        if matches!(self.err, ErrState::None) {
            self.w.write_stored_header(0, false);
            self.w.flush();
            let r = self.writer_err();
            self.set_err(r);
        }
        self.sync = false;
        self.err_result()
    }

    /// `init` a new encode with new writer and compression level.
    fn new(w: W, mut level: i32) -> io::Result<Self> {
        let mut d = Compressor {
            cl: CompressionLevel::default(),
            mode: Mode::Store,
            h: None,
            w: HuffmanBitWriter::new(w),
            window: Vec::new(),
            window_end: 0,
            block_start: 0,
            err: ErrState::None,
            tokens: Tokens::new(),
            fast: None,
            state: None,
            sync: false,
            byte_available: false,
        };
        match level {
            NO_COMPRESSION => {
                d.window = vec![0; MAX_STORE_BLOCK_SIZE];
                d.mode = Mode::Store;
            }
            HUFFMAN_ONLY => {
                d.w.log_new_table_penalty = 10;
                d.window = vec![0; 32 << 10];
                d.mode = Mode::Huff;
            }
            _ => {
                if level == DEFAULT_COMPRESSION {
                    level = 6;
                }
                if (1..=6).contains(&level) {
                    d.w.log_new_table_penalty = 7;
                    d.fast = Some(FastEnc::new(level));
                    d.window = vec![0; MAX_STORE_BLOCK_SIZE];
                    d.mode = Mode::Fast;
                } else if (7..=9).contains(&level) {
                    d.w.log_new_table_penalty = 8;
                    d.state = Some(Box::new(AdvancedState {
                        length: 0,
                        offset: 0,
                        max_insert_index: 0,
                        chain_head: 0,
                        hash_offset: 0,
                        literal_counter: 0,
                        index: 0,
                        hash_match: [0; MAX_MATCH_LENGTH + MIN_MATCH_LENGTH as usize],
                        hash_head: vec![0; HASH_SIZE],
                        hash_prev: vec![0; WINDOW_SIZE as usize],
                    }));
                    d.cl = LEVELS[level as usize];
                    d.init_deflate();
                    d.mode = Mode::Lazy;
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "flate: invalid compression level {level}: want value in range [-2, 9]"
                        ),
                    ));
                }
            }
        }
        d.cl.level = level;
        Ok(d)
    }

    /// `close` flushes any uncompressed data and writes an EOF block.
    fn close(&mut self) -> io::Result<()> {
        match self.err {
            ErrState::Closed => return Ok(()),
            ErrState::Io(_) => return self.err_result(),
            ErrState::None => {}
        }
        self.sync = true;
        self.step();
        self.err_result()?;
        self.w.write_stored_header(0, true);
        self.writer_err()?;
        self.w.flush();
        self.writer_err()?;
        self.err = ErrState::Closed;
        Ok(())
    }
}

/// Go `flate.Writer`: takes data written to it and writes the compressed
/// form of that data to an underlying writer.
pub struct Writer<W: Write> {
    d: Compressor<W>,
}

impl<W: Write> Writer<W> {
    /// `NewWriter` returns a new Writer compressing data at the given level
    /// (-2 = HuffmanOnly, -1 = DefaultCompression (6), 0 = NoCompression,
    /// 1 = BestSpeed .. 9 = BestCompression).
    pub fn new(w: W, level: i32) -> io::Result<Self> {
        Ok(Self {
            d: Compressor::new(w, level)?,
        })
    }

    /// `Write` writes data to w, which will eventually write the compressed
    /// form of data to its underlying writer.
    pub fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.d.write(data)
    }

    /// `Flush` flushes any pending data to the underlying writer, emitting a
    /// sync marker (Z_SYNC_FLUSH).
    pub fn flush(&mut self) -> io::Result<()> {
        self.d.sync_flush()
    }

    /// `Close` flushes and closes the writer.
    pub fn close(&mut self) -> io::Result<()> {
        self.d.close()
    }

    /// The underlying writer.
    pub fn get_ref(&self) -> &W {
        &self.d.w.writer
    }

    /// The underlying writer, mutably (used by the zlib trailer).
    pub(crate) fn get_mut(&mut self) -> &mut W {
        &mut self.d.w.writer
    }

    /// Consume the compressor, returning the underlying writer.
    pub fn into_inner(self) -> W {
        self.d.w.writer
    }
}
