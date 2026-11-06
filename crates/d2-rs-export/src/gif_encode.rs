// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
//! GIF assembly from D2 lib/xgif/encode.go and LSB compression from
//! Go compress/lzw/writer.go. Copyright 2011 The Go Authors.
//! See THIRD_PARTY_NOTICES.md for the Go BSD terms.

pub(crate) struct Encoder {
    output: Vec<u8>,
    global: Vec<u8>,
    opaque: bool,
    total: usize,
    written: usize,
    // The previous frame's indices and LZW stream: a repeated frame is
    // compressed once.
    compressed: Option<(u8, Vec<u8>, Vec<u8>)>,
}

impl Encoder {
    pub(crate) fn new(total: usize, opaque: bool) -> Self {
        Self {
            output: Vec::new(),
            global: Vec::new(),
            opaque,
            total,
            written: 0,
            compressed: None,
        }
    }

    pub(crate) fn write_frame(&mut self, frame: &gif::Frame<'_>) {
        let palette = frame.palette.as_ref().unwrap();
        let entries = (palette.len() / 3).max(2).next_power_of_two();
        let size_code = entries.trailing_zeros() as u8 - 1;
        let mut table = palette.clone();
        table.resize(entries * 3, 0);
        if self.written == 0 {
            self.output.extend_from_slice(b"GIF89a");
            self.output.extend_from_slice(&frame.width.to_le_bytes());
            self.output.extend_from_slice(&frame.height.to_le_bytes());
            self.output
                .extend_from_slice(&[if self.opaque { 0x80 | size_code } else { 0 }, 0, 0]);
            if self.opaque {
                self.output.extend_from_slice(&table);
                self.global = table.clone();
            }
            if self.total > 1 {
                self.output
                    .extend_from_slice(b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00");
            }
        }
        let transparent = if self.opaque { None } else { frame.transparent };
        if frame.delay > 0 || transparent.is_some() {
            self.output
                .extend_from_slice(&[0x21, 0xf9, 4, u8::from(transparent.is_some())]);
            self.output.extend_from_slice(&frame.delay.to_le_bytes());
            self.output
                .extend_from_slice(&[transparent.unwrap_or(0), 0]);
        }
        let local = !self.opaque || !self.global.starts_with(palette);
        self.output.extend_from_slice(&[0x2c, 0, 0, 0, 0]);
        self.output.extend_from_slice(&frame.width.to_le_bytes());
        self.output.extend_from_slice(&frame.height.to_le_bytes());
        self.output.push(if local { 0x80 | size_code } else { 0 });
        if local {
            self.output.extend_from_slice(&table);
        }
        let literal_width = (size_code + 1).max(2);
        self.output.push(literal_width);
        let reusable = self.compressed.as_ref().is_some_and(|(width, pixels, _)| {
            *width == literal_width && pixels[..] == frame.buffer[..]
        });
        if !reusable {
            self.compressed = Some((
                literal_width,
                frame.buffer.to_vec(),
                lzw(&frame.buffer, literal_width),
            ));
        }
        let compressed = &self.compressed.as_ref().expect("compressed frame").2;
        for block in compressed.chunks(255) {
            self.output.push(block.len() as u8);
            self.output.extend_from_slice(block);
        }
        self.output.push(0);
        self.written += 1;
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.output.push(0x3b);
        self.output
    }
}

fn lzw(pixels: &[u8], literal_width: u8) -> Vec<u8> {
    let clear = 1_u32 << literal_width;
    let mut width = literal_width + 1;
    let mut hi = clear + 1;
    let mut overflow = clear << 1;
    let mut table = vec![0_u32; 16384];
    let mut output = Vec::new();
    let (mut bits, mut nbits) = (0_u32, 0_u8);
    let emit = |code: u32, width: u8, output: &mut Vec<u8>, bits: &mut u32, nbits: &mut u8| {
        *bits |= code << *nbits;
        *nbits += width;
        while *nbits >= 8 {
            output.push(*bits as u8);
            *bits >>= 8;
            *nbits -= 8;
        }
    };
    emit(clear, width, &mut output, &mut bits, &mut nbits);
    if let Some((&first, rest)) = pixels.split_first() {
        let mut code = u32::from(first);
        for &literal in rest {
            let key = code << 8 | u32::from(literal);
            let mut hash = ((key >> 12 ^ key) & 16383) as usize;
            while table[hash] != 0 && table[hash] >> 12 != key {
                hash = (hash + 1) & 16383;
            }
            if table[hash] != 0 {
                code = table[hash] & 4095;
                continue;
            }
            emit(code, width, &mut output, &mut bits, &mut nbits);
            code = u32::from(literal);
            hi += 1;
            if hi == overflow {
                width += 1;
                overflow <<= 1;
            }
            if hi == 4095 {
                emit(clear, width, &mut output, &mut bits, &mut nbits);
                width = literal_width + 1;
                hi = clear + 1;
                overflow = clear << 1;
                table.fill(0);
            } else {
                table[hash] = (key << 12) | hi;
            }
        }
        emit(code, width, &mut output, &mut bits, &mut nbits);
        hi += 1;
        if hi == overflow {
            width += 1;
        }
        if hi == 4095 {
            emit(clear, width, &mut output, &mut bits, &mut nbits);
            width = literal_width + 1;
        }
    }
    emit(clear + 1, width, &mut output, &mut bits, &mut nbits);
    if nbits > 0 {
        output.push(bits as u8);
    }
    output
}
