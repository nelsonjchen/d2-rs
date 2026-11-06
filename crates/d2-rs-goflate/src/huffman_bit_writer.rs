// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 src/compress/flate/huffman_bit_writer.go.
// Copyright 2009 The Go Authors. All rights reserved. BSD-3-Clause; see
// THIRD_PARTY_NOTICES.md.

//! DEFLATE block writer (`huffman_bit_writer.go`). Chooses between stored,
//! fixed, fresh dynamic and *reused* dynamic Huffman tables exactly as Go
//! does; a dynamic block may stay open (`prevHeader > 0`) so the next block
//! can continue with the same table.
//!
//! Go's `writeBlock` is reachable only from the unused `writeBlockSkip` in
//! deflate.go and is not translated.

use std::io::{self, Write};
use std::sync::OnceLock;

use crate::deflate::MAX_STORE_BLOCK_SIZE;
use crate::huffman_code::{
    Hcode, HuffmanEncoder, LITERAL_COUNT, fixed_literal_encoding, fixed_offset_encoding, histogram,
};
use crate::token::{
    MATCH_OFFSET_ONLY_MASK, Token, Tokens, length_code, token_length, token_offset,
};

/// The largest offset code.
pub(crate) const OFFSET_CODE_COUNT: usize = 30;
/// The special code used to mark the end of a block.
pub(crate) const END_BLOCK_MARKER: usize = 256;
/// The first length code.
const LENGTH_CODES_START: usize = 257;
/// The number of codegen codes.
const CODEGEN_CODE_COUNT: usize = 19;
const BAD_CODE: u8 = 255;
/// `maxPredefinedTokens` is the maximum number of tokens where we check if
/// fixed size is smaller.
const MAX_PREDEFINED_TOKENS: u16 = 250;
/// `bufferFlushSize` indicates the buffer size after which bytes are flushed
/// to the writer.
const BUFFER_FLUSH_SIZE: usize = 246;

/// `lengthExtraBitsMinCode` is the minimum length code that emits extra bits.
const LENGTH_EXTRA_BITS_MIN_CODE: u8 = 8;

/// `lengthExtraBits[i]` is the number of extra bits needed by length code
/// i + lengthCodesStart.
pub(crate) static LENGTH_EXTRA_BITS: [u8; 32] = [
    /* 257 */ 0, 0, 0, /* 260 */ 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, /* 270 */ 2, 2, 2, 3,
    3, 3, 3, 4, 4, 4, /* 280 */ 4, 5, 5, 5, 5, 0, 0, 0, 0,
];

/// `lengthBase[i]` is the length indicated by length code i + lengthCodesStart.
static LENGTH_BASE: [u8; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 20, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128,
    160, 192, 224, 255, 0, 0, 0,
];

/// `offsetExtraBitsMinCode` is the minimum offset code that emits extra bits.
const OFFSET_EXTRA_BITS_MIN_CODE: u32 = 4;

/// `offsetExtraBits[i]` is the number of extra bits for offset code i.
pub(crate) static OFFSET_EXTRA_BITS: [i8; 32] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13, /* extended window */ 14, 14,
];

/// `offsetCombined` combines offset lookup of extra bits and offset code in a
/// single table.
static OFFSET_COMBINED: [u32; 32] = [
    0x0, 0x0, 0x0, 0x0, 0x401, 0x601, 0x802, 0xc02, 0x1003, 0x1803, 0x2004, 0x3004, 0x4005, 0x6005,
    0x8006, 0xc006, 0x10007, 0x18007, 0x20008, 0x30008, 0x40009, 0x60009, 0x8000a, 0xc000a,
    0x10000b, 0x18000b, 0x20000c, 0x30000c, 0x40000d, 0x60000d, 0x0, 0x0,
];

/// `codegenOrder` is the order in which codegen code sizes are written.
static CODEGEN_ORDER: [u32; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// `huffOffset` is a static offset encoder used for Huffman-only encoding.
fn huff_offset() -> &'static HuffmanEncoder {
    static ENC: OnceLock<HuffmanEncoder> = OnceLock::new();
    ENC.get_or_init(|| {
        let mut offset_freq = [0u16; OFFSET_CODE_COUNT];
        offset_freq[0] = 1;
        let mut h = HuffmanEncoder::new(OFFSET_CODE_COUNT);
        h.generate(&offset_freq, 15);
        h
    })
}

/// Re-create a sticky error for returning to callers.
pub(crate) fn clone_err(e: &io::Error) -> io::Error {
    io::Error::new(e.kind(), e.to_string())
}

/// Which literal/offset tables to write tokens with.
#[derive(Clone, Copy)]
enum Codes {
    Fixed,
    Current,
}

/// `huffmanBitWriter` encodes tokens and values to a stream. It supports
/// reusing huffman tables and will combine blocks, if compression is less
/// than creating a new table.
pub(crate) struct HuffmanBitWriter<W: Write> {
    /// writer is the underlying writer; use [`Self::write`], which makes
    /// write errors sticky.
    pub(crate) writer: W,

    // Data waiting to be written is bytes[0:nbytes] and then the low nbits
    // of bits.
    bits: u64,
    nbits: u32,
    bytes: Vec<u8>,

    /// If wroteHuffman is set, a table for outputting only literals has been
    /// generated and offsets are invalid.
    wrote_huffman: bool,
    literal_encoding: HuffmanEncoder,
    tmp_lit_encoding: HuffmanEncoder,
    offset_encoding: HuffmanEncoder,
    codegen_encoding: HuffmanEncoder,
    pub(crate) err: Option<io::Error>,

    /// If prevHeader is non-zero the Huffman table can be reused. It also
    /// indicates that an EOB has not yet been emitted, so if a new table is
    /// generated, an EOB with the previous table must be written.
    prev_header: i64,

    /// logNewTablePenalty is a log2 penalty reduction for creating new
    /// tables. The initial penalty is 100%. Adding 1 will cut the penalty in
    /// half.
    pub(crate) log_new_table_penalty: u32,
    literal_freq: [u16; LENGTH_CODES_START + 32],
    offset_freq: [u16; 32],
    codegen_freq: [u16; CODEGEN_CODE_COUNT],

    /// codegen must have an extra space for the final symbol.
    codegen: [u8; LITERAL_COUNT + OFFSET_CODE_COUNT + 1],
}

impl<W: Write> HuffmanBitWriter<W> {
    /// `newHuffmanBitWriter` creates a new huffmanBitWriter that will write to w.
    pub(crate) fn new(w: W) -> Self {
        Self {
            writer: w,
            bits: 0,
            nbits: 0,
            bytes: Vec::with_capacity(256 + 8),
            wrote_huffman: false,
            literal_encoding: HuffmanEncoder::new(LITERAL_COUNT),
            tmp_lit_encoding: HuffmanEncoder::new(LITERAL_COUNT),
            codegen_encoding: HuffmanEncoder::new(CODEGEN_CODE_COUNT),
            offset_encoding: HuffmanEncoder::new(OFFSET_CODE_COUNT),
            err: None,
            prev_header: 0,
            log_new_table_penalty: 0,
            literal_freq: [0; LENGTH_CODES_START + 32],
            offset_freq: [0; 32],
            codegen_freq: [0; CODEGEN_CODE_COUNT],
            codegen: [0; LITERAL_COUNT + OFFSET_CODE_COUNT + 1],
        }
    }

    /// `canReuse` checks if the current generated tables can be reused for
    /// the provided tokens.
    fn can_reuse(&self, t: &Tokens) -> bool {
        let b = &self.offset_encoding.codes;
        for (i, &v) in t.off_hist[..OFFSET_CODE_COUNT].iter().enumerate() {
            if v != 0 && b[i].zero() {
                return false;
            }
        }

        let b = &self.literal_encoding.codes[256..LITERAL_COUNT];
        for (i, &v) in t.extra_hist[..LITERAL_COUNT - 256].iter().enumerate() {
            if v != 0 && b[i].zero() {
                return false;
            }
        }

        let b = &self.literal_encoding.codes[..256];
        for (i, &v) in t.lit_hist.iter().enumerate() {
            if v != 0 && b[i].zero() {
                return false;
            }
        }
        true
    }

    /// Write the pending EOB of an open block (Go's repeated
    /// "We owe an EOB" sequence).
    #[inline]
    fn write_owed_eob(&mut self) {
        if self.prev_header > 0 {
            let c = self.literal_encoding.codes[END_BLOCK_MARKER];
            self.write_code(c);
            self.prev_header = 0;
        }
    }

    /// `flush` flushes the currently encoded data. An EOB will be written if
    /// the current block hasn't been ended.
    pub(crate) fn flush(&mut self) {
        if self.err.is_some() {
            self.nbits = 0;
            return;
        }
        self.write_owed_eob();
        while self.nbits != 0 {
            self.bytes.push(self.bits as u8);
            self.bits >>= 8;
            if self.nbits > 8 {
                // Avoid underflow
                self.nbits -= 8;
            } else {
                self.nbits = 0;
            }
        }
        self.bits = 0;
        if !self.bytes.is_empty() {
            let buf = std::mem::take(&mut self.bytes);
            self.write(&buf);
            self.bytes = buf;
            self.bytes.clear();
        }
    }

    /// `write` writes the provided bytes directly to the output.
    fn write(&mut self, b: &[u8]) {
        if self.err.is_some() {
            return;
        }
        if let Err(e) = self.writer.write_all(b) {
            self.err = Some(e);
        }
    }

    /// Hand buffered bytes to the writer once `bufferFlushSize` is reached.
    #[inline]
    fn maybe_write_buffer(&mut self) {
        if self.bytes.len() >= BUFFER_FLUSH_SIZE {
            if self.err.is_some() {
                self.bytes.clear();
                return;
            }
            let buf = std::mem::take(&mut self.bytes);
            self.write(&buf);
            self.bytes = buf;
            self.bytes.clear();
        }
    }

    /// `writeBits` writes nb bits from b to the stream.
    #[inline]
    fn write_bits(&mut self, b: i32, nb: u32) {
        // Go: uint64(int32) sign-extends; callers only pass non-negative values.
        self.bits |= (i64::from(b) as u64) << (self.nbits & 63);
        self.nbits += nb;
        if self.nbits >= 48 {
            self.flush_bits();
        }
    }

    /// `writeBytes` writes the provided bytes to the stream.
    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        if self.err.is_some() {
            return;
        }
        if self.nbits & 7 != 0 {
            self.err = Some(io::Error::other(
                "flate: internal error: writeBytes with unfinished bits",
            ));
            return;
        }
        while self.nbits != 0 {
            self.bytes.push(self.bits as u8);
            self.bits >>= 8;
            self.nbits -= 8;
        }
        if !self.bytes.is_empty() {
            let buf = std::mem::take(&mut self.bytes);
            self.write(&buf);
            self.bytes = buf;
        }
        self.bytes.clear();
        self.write(bytes);
    }

    /// `generateCodegen`: RFC 1951 3.2.7 run-length encoding of the
    /// concatenated literal and offset code lengths, written into codegen,
    /// with symbol frequencies in codegenFreq.
    fn generate_codegen(
        &mut self,
        num_literals: usize,
        num_offsets: usize,
        lit_enc: Option<&HuffmanEncoder>,
        off_enc: Option<&HuffmanEncoder>,
    ) {
        self.codegen_freq = [0; CODEGEN_CODE_COUNT];
        let lit_enc = lit_enc.unwrap_or(&self.literal_encoding);
        let off_enc = off_enc.unwrap_or(&self.offset_encoding);
        let codegen = &mut self.codegen;
        // Copy the concatenated code sizes to codegen. Put a marker at the end.
        for (dst, c) in codegen[..num_literals].iter_mut().zip(&lit_enc.codes) {
            *dst = c.len();
        }
        for (dst, c) in codegen[num_literals..num_literals + num_offsets]
            .iter_mut()
            .zip(&off_enc.codes)
        {
            *dst = c.len();
        }
        codegen[num_literals + num_offsets] = BAD_CODE;

        let mut size = codegen[0];
        let mut count: i32 = 1;
        let mut out_index = 0usize;
        let mut in_index = 1usize;
        while size != BAD_CODE {
            // INVARIANT: We have seen "count" copies of size that have not yet
            // had output generated for them.
            let next_size = codegen[in_index];
            if next_size == size {
                count += 1;
                in_index += 1;
                continue;
            }
            // We need to generate codegen indicating "count" of size.
            if size != 0 {
                codegen[out_index] = size;
                out_index += 1;
                self.codegen_freq[size as usize] += 1;
                count -= 1;
                while count >= 3 {
                    let n = count.min(6);
                    codegen[out_index] = 16;
                    out_index += 1;
                    codegen[out_index] = (n - 3) as u8;
                    out_index += 1;
                    self.codegen_freq[16] += 1;
                    count -= n;
                }
            } else {
                while count >= 11 {
                    let n = count.min(138);
                    codegen[out_index] = 18;
                    out_index += 1;
                    codegen[out_index] = (n - 11) as u8;
                    out_index += 1;
                    self.codegen_freq[18] += 1;
                    count -= n;
                }
                if count >= 3 {
                    // count >= 3 && count <= 10
                    codegen[out_index] = 17;
                    out_index += 1;
                    codegen[out_index] = (count - 3) as u8;
                    out_index += 1;
                    self.codegen_freq[17] += 1;
                    count = 0;
                }
            }
            count -= 1;
            while count >= 0 {
                codegen[out_index] = size;
                out_index += 1;
                self.codegen_freq[size as usize] += 1;
                count -= 1;
            }
            // Set up invariant for next time through the loop.
            size = next_size;
            count = 1;
            in_index += 1;
        }
        // Marker indicating the end of the codegen.
        codegen[out_index] = BAD_CODE;
    }

    /// `codegens` returns current number of non-zero codegens.
    fn codegens(&self) -> usize {
        let mut num_codegens = self.codegen_freq.len();
        while num_codegens > 4 && self.codegen_freq[CODEGEN_ORDER[num_codegens - 1] as usize] == 0 {
            num_codegens -= 1;
        }
        num_codegens
    }

    /// `headerSize` returns the size of the header with the current encodings.
    fn header_size(&self) -> (i64, usize) {
        let num_codegens = self.codegens();
        (
            3 + 5
                + 5
                + 4
                + (3 * num_codegens as i64)
                + self.codegen_encoding.bit_length(&self.codegen_freq)
                + i64::from(self.codegen_freq[16]) * 2
                + i64::from(self.codegen_freq[17]) * 3
                + i64::from(self.codegen_freq[18]) * 7,
            num_codegens,
        )
    }

    /// `dynamicReuseSize`, with the current literal/offset encodings.
    fn dynamic_reuse_size(&self) -> i64 {
        self.literal_encoding.bit_length(&self.literal_freq)
            + self.offset_encoding.bit_length(&self.offset_freq)
    }

    /// `dynamicSize` returns the size of dynamically encoded data in bits,
    /// with the current literal/offset encodings.
    fn dynamic_size(&self, extra_bits: i64) -> (i64, usize) {
        let (header, num_codegens) = self.header_size();
        let size = header
            + self.literal_encoding.bit_length(&self.literal_freq)
            + self.offset_encoding.bit_length(&self.offset_freq)
            + extra_bits;
        (size, num_codegens)
    }

    /// `extraBitSize` returns the number of bits that will be written as
    /// "extra" bits on matches.
    fn extra_bit_size(&self) -> i64 {
        let mut total: i64 = 0;
        for (i, &n) in self.literal_freq[257..LITERAL_COUNT].iter().enumerate() {
            total += i64::from(n) * i64::from(LENGTH_EXTRA_BITS[i & 31]);
        }
        for (i, &n) in self.offset_freq[..OFFSET_CODE_COUNT].iter().enumerate() {
            total += i64::from(n) * i64::from(OFFSET_EXTRA_BITS[i & 31]);
        }
        total
    }

    /// `fixedSize` returns the size of fixed-Huffman encoded data in bits.
    fn fixed_size(&self, extra_bits: i64) -> i64 {
        3 + fixed_literal_encoding().bit_length(&self.literal_freq)
            + fixed_offset_encoding().bit_length(&self.offset_freq)
            + extra_bits
    }

    /// `storedSize` calculates the stored size, including header, in bits and
    /// whether it fits inside a single block. `None` is Go's nil input.
    fn stored_size(input: Option<&[u8]>) -> (i64, bool) {
        let Some(input) = input else {
            return (0, false);
        };
        if input.len() <= MAX_STORE_BLOCK_SIZE {
            return ((input.len() as i64 + 5) * 8, true);
        }
        (0, false)
    }

    /// `writeCode` writes 'c' to the stream.
    #[inline]
    fn write_code(&mut self, c: Hcode) {
        self.bits |= c.code64() << (self.nbits & 0xff);
        self.nbits += u32::from(c.len());
        if self.nbits >= 48 {
            self.flush_bits();
        }
    }

    /// `flushBits` writes accumulated bits to the byte buffer.
    #[inline]
    fn flush_bits(&mut self) {
        let bits = self.bits;
        self.bits >>= 48;
        self.nbits -= 48;
        self.bytes.extend_from_slice(&bits.to_le_bytes()[..6]);
        self.maybe_write_buffer();
    }

    /// `writeDynamicHeader` writes the header of a dynamic Huffman block.
    fn write_dynamic_header(
        &mut self,
        num_literals: usize,
        num_offsets: usize,
        num_codegens: usize,
        is_eof: bool,
    ) {
        if self.err.is_some() {
            return;
        }
        let first_bits: i32 = if is_eof { 5 } else { 4 };
        self.write_bits(first_bits, 3);
        self.write_bits(num_literals as i32 - 257, 5);
        self.write_bits(num_offsets as i32 - 1, 5);
        self.write_bits(num_codegens as i32 - 4, 4);

        for &order in CODEGEN_ORDER.iter().take(num_codegens) {
            let value = self.codegen_encoding.codes[order as usize].len();
            self.write_bits(i32::from(value), 3);
        }

        let mut i = 0usize;
        loop {
            let code_word = self.codegen[i];
            i += 1;
            if code_word == BAD_CODE {
                break;
            }
            let c = self.codegen_encoding.codes[code_word as usize];
            self.write_code(c);

            match code_word {
                16 => {
                    self.write_bits(i32::from(self.codegen[i]), 2);
                    i += 1;
                }
                17 => {
                    self.write_bits(i32::from(self.codegen[i]), 3);
                    i += 1;
                }
                18 => {
                    self.write_bits(i32::from(self.codegen[i]), 7);
                    i += 1;
                }
                _ => {}
            }
        }
    }

    /// `writeStoredHeader` writes a stored header. If the stored block is
    /// only used for EOF, it is replaced with a fixed huffman block.
    pub(crate) fn write_stored_header(&mut self, length: usize, is_eof: bool) {
        if self.err.is_some() {
            return;
        }
        self.write_owed_eob();

        // To write EOF, use a fixed encoding block. 10 bits instead of 5 bytes.
        if length == 0 && is_eof {
            self.write_fixed_header(is_eof);
            // EOB: 7 bits, value: 0
            self.write_bits(0, 7);
            self.flush();
            return;
        }

        let flag: i32 = if is_eof { 1 } else { 0 };
        self.write_bits(flag, 3);
        self.flush();
        self.write_bits(length as i32, 16);
        self.write_bits(i32::from(!(length as u16)), 16);
    }

    /// `writeFixedHeader` writes a fixed encoding header to the output stream.
    fn write_fixed_header(&mut self, is_eof: bool) {
        if self.err.is_some() {
            return;
        }
        self.write_owed_eob();

        // Indicate that we are a fixed Huffman block
        let value: i32 = if is_eof { 3 } else { 2 };
        self.write_bits(value, 3);
    }

    /// Stored-block shortcut shared by the block writers.
    fn write_stored(&mut self, input: Option<&[u8]>, eof: bool) {
        let input = input.unwrap_or(&[]);
        self.write_stored_header(input.len(), eof);
        self.write_bytes(input);
    }

    /// `writeBlockDynamic` encodes a block using a dynamic Huffman table.
    /// This should be used if the symbols used have a disproportionate
    /// histogram distribution.
    pub(crate) fn write_block_dynamic(
        &mut self,
        tokens: &mut Tokens,
        eof: bool,
        input: Option<&[u8]>,
        sync: bool,
    ) {
        if self.err.is_some() {
            return;
        }

        let sync = sync || eof;
        if sync {
            tokens.add_eob();
        } else {
            // Ensure we can always write EOB.
            tokens.extra_hist[0] = 1;
        }

        // We cannot reuse pure Huffman table, and must mark as EOF.
        if (self.wrote_huffman || eof) && self.prev_header > 0 {
            // We will not try to reuse.
            let c = self.literal_encoding.codes[END_BLOCK_MARKER];
            self.write_code(c);
            self.prev_header = 0;
            self.wrote_huffman = false;
        }

        if self.prev_header > 0 && !self.can_reuse(tokens) {
            let c = self.literal_encoding.codes[END_BLOCK_MARKER];
            self.write_code(c);
            self.prev_header = 0;
        }

        let (num_literals, num_offsets) = self.index_tokens(tokens);
        let mut extra_bits: i64 = 0;
        let (ssize, storable) = Self::stored_size(input);

        if storable || self.prev_header > 0 {
            extra_bits = self.extra_bit_size();
        }

        let mut size: i64;

        // Check whether we should reuse the previous Huffman table.
        if self.prev_header > 0 {
            // Estimate size for using a new table.
            // Use the previous header size as the best estimate.
            let mut new_size = self.prev_header + tokens.estimated_bits();

            // The estimated size is calculated as an optimal table.
            // We add a penalty to make it more realistic and re-use a bit more.
            new_size += i64::from(self.literal_encoding.codes[END_BLOCK_MARKER].len())
                + (new_size >> self.log_new_table_penalty);

            // Calculate the size for reusing the current table.
            let reuse_size = self.dynamic_reuse_size() + extra_bits;

            // Check if a new table is better.
            if new_size < reuse_size {
                // Write the EOB we owe.
                let c = self.literal_encoding.codes[END_BLOCK_MARKER];
                self.write_code(c);
                size = new_size;
                self.prev_header = 0;
            } else {
                size = reuse_size;
            }

            // Small blocks can be more efficient with fixed encoding.
            if tokens.n < MAX_PREDEFINED_TOKENS {
                let pre_size = self.fixed_size(extra_bits) + 7;
                if pre_size < size {
                    // Check if we get a reasonable size decrease.
                    if storable && ssize <= size {
                        self.write_stored(input, eof);
                        return;
                    }
                    self.write_fixed_header(eof);
                    if !sync {
                        tokens.add_eob();
                    }
                    self.write_tokens(tokens.slice(), Codes::Fixed);
                    return;
                }
            }

            // Check if we get a reasonable size decrease.
            if storable && ssize <= size {
                self.write_stored(input, eof);
                return;
            }
        }

        // We want a new block/table
        if self.prev_header == 0 {
            self.literal_freq[END_BLOCK_MARKER] = 1;

            self.generate();
            // Generate codegen and codegenFrequencies, which indicates how to
            // encode the literalEncoding and the offsetEncoding.
            self.generate_codegen(num_literals, num_offsets, None, None);
            let codegen_freq = self.codegen_freq;
            self.codegen_encoding.generate(&codegen_freq, 7);

            let num_codegens;
            (size, num_codegens) = self.dynamic_size(extra_bits);

            // Store predefined or raw, if we don't get a reasonable improvement.
            if tokens.n < MAX_PREDEFINED_TOKENS {
                let pre_size = self.fixed_size(extra_bits);
                if pre_size <= size {
                    // Store bytes, if we don't get an improvement.
                    if storable && ssize <= pre_size {
                        self.write_stored(input, eof);
                        return;
                    }
                    self.write_fixed_header(eof);
                    if !sync {
                        tokens.add_eob();
                    }
                    self.write_tokens(tokens.slice(), Codes::Fixed);
                    return;
                }
            }

            if storable && ssize <= size {
                // Store bytes, if we don't get an improvement.
                self.write_stored(input, eof);
                return;
            }

            // Write Huffman table.
            self.write_dynamic_header(num_literals, num_offsets, num_codegens, eof);
            if !sync {
                self.prev_header = self.header_size().0;
            }
            self.wrote_huffman = false;
        }

        if sync {
            self.prev_header = 0;
        }
        // Write the tokens.
        self.write_tokens(tokens.slice(), Codes::Current);
    }

    /// `indexTokens` copies the token histograms into literalFreq and
    /// offsetFreq and returns the number of literal and offset codes.
    fn index_tokens(&mut self, t: &Tokens) -> (usize, usize) {
        self.literal_freq[..256].copy_from_slice(&t.lit_hist);
        self.literal_freq[256..256 + 32].copy_from_slice(&t.extra_hist);
        self.offset_freq = t.off_hist;

        if t.n == 0 {
            return (0, 0);
        }
        // get the number of literals
        let mut num_literals = self.literal_freq.len();
        while self.literal_freq[num_literals - 1] == 0 {
            num_literals -= 1;
        }
        // get the number of offsets
        let mut num_offsets = self.offset_freq.len();
        while num_offsets > 0 && self.offset_freq[num_offsets - 1] == 0 {
            num_offsets -= 1;
        }
        if num_offsets == 0 {
            // We haven't found a single match. If we want to go with the
            // dynamic encoding, we should count at least one offset to be sure
            // that the offset huffman tree could be encoded.
            self.offset_freq[0] = 1;
            num_offsets = 1;
        }
        (num_literals, num_offsets)
    }

    /// `generate` literalEncoding and offsetEncoding based on respective
    /// histograms.
    fn generate(&mut self) {
        self.literal_encoding
            .generate(&self.literal_freq[..LITERAL_COUNT], 15);
        self.offset_encoding
            .generate(&self.offset_freq[..OFFSET_CODE_COUNT], 15);
    }

    /// `writeTokens` writes a slice of tokens to the output.
    fn write_tokens(&mut self, tokens: &[Token], which: Codes) {
        if self.err.is_some() {
            return;
        }
        if tokens.is_empty() {
            return;
        }
        let (len_codes, off_codes): (&[Hcode], &[Hcode]) = match which {
            Codes::Fixed => (
                &fixed_literal_encoding().codes,
                &fixed_offset_encoding().codes,
            ),
            // Temporarily detach the tables so the bit writer can borrow self.
            Codes::Current => {
                let lit = std::mem::take(&mut self.literal_encoding.codes);
                let off = std::mem::take(&mut self.offset_encoding.codes);
                self.write_tokens_with(tokens, &lit, &off);
                self.literal_encoding.codes = lit;
                self.offset_encoding.codes = off;
                return;
            }
        };
        self.write_tokens_with(tokens, len_codes, off_codes);
    }

    fn write_tokens_with(
        &mut self,
        mut tokens: &[Token],
        len_codes: &[Hcode],
        off_codes: &[Hcode],
    ) {
        // Only last token should be endBlockMarker.
        let mut defer_eob = false;
        if tokens[tokens.len() - 1] == END_BLOCK_MARKER as Token {
            tokens = &tokens[..tokens.len() - 1];
            defer_eob = true;
        }

        let lits = &len_codes[..256];
        let lengths = &len_codes[LENGTH_CODES_START..];

        for &t in tokens {
            if t < 256 {
                let c = lits[t as usize];
                self.write_code(c);
                continue;
            }

            // Write the length
            let length = token_length(t);
            let len_code = length_code(length) & 31;
            self.write_code(lengths[len_code as usize]);

            if len_code >= LENGTH_EXTRA_BITS_MIN_CODE {
                let extra_length_bits = LENGTH_EXTRA_BITS[len_code as usize];
                let extra_length = i32::from(length.wrapping_sub(LENGTH_BASE[len_code as usize]));
                self.write_bits(extra_length, u32::from(extra_length_bits));
            }
            // Write the offset
            let offset = token_offset(t);
            let off_code = (offset >> 16) & 31;
            self.write_code(off_codes[off_code as usize]);

            if off_code >= OFFSET_EXTRA_BITS_MIN_CODE {
                let offset_comb = OFFSET_COMBINED[off_code as usize];
                let extra = offset.wrapping_sub(offset_comb >> 8) & MATCH_OFFSET_ONLY_MASK;
                self.bits |= u64::from(extra) << (self.nbits & 63);
                self.nbits += offset_comb & 0xff;
                if self.nbits >= 48 {
                    self.flush_bits();
                }
            }
        }

        if defer_eob {
            self.write_code(len_codes[END_BLOCK_MARKER]);
        }
    }

    /// `writeBlockHuff` encodes a block of bytes as either Huffman-encoded
    /// literals or uncompressed bytes if the results gain very little from
    /// compression.
    pub(crate) fn write_block_huff(&mut self, eof: bool, input: &[u8], sync: bool) {
        if self.err.is_some() {
            return;
        }

        // Clear histogram
        self.literal_freq = [0; LENGTH_CODES_START + 32];
        if !self.wrote_huffman {
            self.offset_freq = [0; 32];
        }

        const NUM_LITERALS: usize = END_BLOCK_MARKER + 1;
        const NUM_OFFSETS: usize = 1;

        // Estimate size of literal encoding.
        const GUESS_HEADER_SIZE_BITS: i64 = 70 * 8; // 70 bytes
        histogram(input, &mut self.literal_freq[..NUM_LITERALS]);
        let (ssize, storable) = Self::stored_size(Some(input));
        if storable && input.len() > 1024 {
            // Quick check for incompressible content: all frequencies lie
            // close to the average frequency. (diff*diff is exact in f64, so
            // FMA fusion cannot change this sum.)
            let mut abs = 0f64;
            let avg = input.len() as f64 / 256.0;
            let max = (input.len() * 2) as f64;
            for &v in self.literal_freq[..256].iter() {
                let diff = f64::from(v) - avg;
                abs += diff * diff;
                if abs >= max {
                    break;
                }
            }
            if abs < max {
                // No chance we can compress this...
                self.write_stored_header(input.len(), eof);
                self.write_bytes(input);
                return;
            }
        }
        self.literal_freq[END_BLOCK_MARKER] = 1;
        self.tmp_lit_encoding
            .generate(&self.literal_freq[..NUM_LITERALS], 15);
        let mut est_bits = self
            .tmp_lit_encoding
            .can_encode_len(&self.literal_freq[..NUM_LITERALS]);
        if est_bits < i64::from(i32::MAX) {
            est_bits += self.prev_header;
            if self.prev_header == 0 {
                est_bits += GUESS_HEADER_SIZE_BITS;
            }
            est_bits += est_bits >> self.log_new_table_penalty;
        }

        // Store bytes, if we don't get a reasonable improvement.
        if storable && ssize <= est_bits {
            self.write_stored_header(input.len(), eof);
            self.write_bytes(input);
            return;
        }

        if self.prev_header > 0 {
            let reuse_size = self
                .literal_encoding
                .can_encode_len(&self.literal_freq[..256]);
            if est_bits < reuse_size {
                // We owe an EOB
                let c = self.literal_encoding.codes[END_BLOCK_MARKER];
                self.write_code(c);
                self.prev_header = 0;
            }
        }

        if self.prev_header == 0 {
            // Use the temp encoding, so swap.
            std::mem::swap(&mut self.literal_encoding, &mut self.tmp_lit_encoding);
            // Generate codegen and codegenFrequencies, which indicates how to
            // encode the literalEncoding and the offsetEncoding.
            self.generate_codegen(NUM_LITERALS, NUM_OFFSETS, None, Some(huff_offset()));
            let codegen_freq = self.codegen_freq;
            self.codegen_encoding.generate(&codegen_freq, 7);
            let num_codegens = self.codegens();

            // Huffman.
            self.write_dynamic_header(NUM_LITERALS, NUM_OFFSETS, num_codegens, eof);
            self.wrote_huffman = true;
            self.prev_header = self.header_size().0;
        }

        // Go unrolls this loop and spills a variable number of whole bytes;
        // the emitted bit stream is the same as writing each code in order.
        let encoding: [Hcode; 256] = self.literal_encoding.codes[..256]
            .try_into()
            .expect("256 literal codes");
        for &t in input {
            let c = encoding[t as usize];
            self.bits |= c.code64() << (self.nbits & 63);
            self.nbits += u32::from(c.len());
            while self.nbits >= 8 {
                self.bytes.push(self.bits as u8);
                self.bits >>= 8;
                self.nbits -= 8;
            }
            self.maybe_write_buffer();
        }

        if eof || sync {
            let c = self.literal_encoding.codes[END_BLOCK_MARKER];
            self.write_code(c);
            self.prev_header = 0;
            self.wrote_huffman = false;
        }
    }
}
