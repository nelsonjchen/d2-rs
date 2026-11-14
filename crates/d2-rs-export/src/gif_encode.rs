// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
//! GIF assembly from D2 lib/xgif/encode.go, Go image/gif/writer.go
//! EncodeAll (Copyright 2013 The Go Authors) and LSB compression from
//! Go compress/lzw/writer.go (Copyright 2011 The Go Authors).
//! See THIRD_PARTY_NOTICES.md for the Go BSD terms.

use std::io::{self, Write};

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

/// Go image/gif `EncodeAll` (writer.go, Go 1.27) for a `gif.GIF` whose
/// `Config.ColorModel` is `palette` and whose every frame shares that palette
/// slice, so each image block uses the global color table. `palette` holds
/// `color.RGBA` entries. Frames stream through Go's `bufio.NewWriter` chunking
/// (4096 bytes, flushed after every image block), so a bounded or cancellable
/// writer sees the same writes as Go's.
pub struct ImageGifEncoder<W: Write> {
    output: GoBufWriter<W>,
    width: usize,
    height: usize,
    padded_size: u8,
    transparent: Option<u8>,
}

impl<W: Write> ImageGifEncoder<W> {
    /// writer.go EncodeAll admission and writeHeader. `frames` is
    /// `len(g.Image)`; `loop_count` is `g.LoopCount`.
    pub fn new(
        writer: W,
        width: usize,
        height: usize,
        palette: &[[u8; 4]],
        frames: usize,
        loop_count: i32,
    ) -> io::Result<Self> {
        if frames == 0 {
            return Err(io::Error::other("gif: must provide at least one image"));
        }
        // writeImageBlock rejects an empty palette before any image data.
        if palette.is_empty() {
            return Err(io::Error::other(
                "gif: cannot encode image block with empty palette",
            ));
        }
        // log2: Size of Global Color Table is 2^(1+n).
        let padded_size = (usize::BITS - (palette.len() - 1).leading_zeros()).max(1) as u8 - 1;
        if padded_size >= 8 {
            return Err(io::Error::other(
                "gif: cannot encode color table with more than 256 entries",
            ));
        }
        let mut output = GoBufWriter::new(writer);
        output.write(b"GIF89a")?;
        output.write(&(width as u16).to_le_bytes())?;
        output.write(&(height as u16).to_le_bytes())?;
        // fColorTable | size; background index 0; pixel aspect ratio 0.
        output.write(&[0x80 | padded_size, 0, 0])?;
        let mut table = [0_u8; 3 * 256];
        for (i, color) in palette.iter().enumerate() {
            table[3 * i..3 * i + 3].copy_from_slice(&color[..3]);
        }
        output.write(&table[..3 << (padded_size + 1)])?;
        if frames > 1 && loop_count >= 0 {
            output.write(b"\x21\xff\x0bNETSCAPE2.0")?;
            let count = (loop_count as u16).to_le_bytes();
            output.write(&[0x03, 0x01, count[0], count[1], 0x00])?;
        }
        Ok(Self {
            output,
            width,
            height,
            padded_size,
            // The first palette entry whose alpha is zero.
            transparent: palette.iter().position(|c| c[3] == 0).map(|i| i as u8),
        })
    }

    /// writer.go writeImageBlock for a frame at `left`,`top` whose row-major
    /// indices are `pixels` (`width * height`, stride equal to width).
    #[allow(clippy::too_many_arguments)]
    pub fn write_frame(
        &mut self,
        left: usize,
        top: usize,
        width: usize,
        height: usize,
        delay: u16,
        disposal: u8,
        pixels: &[u8],
    ) -> io::Result<()> {
        let (right, bottom) = (left + width, top + height);
        if right >= 1 << 16 || bottom >= 1 << 16 {
            return Err(io::Error::other("gif: image block is too large to encode"));
        }
        if width > 0 && height > 0 && (right > self.width || bottom > self.height) {
            return Err(io::Error::other("gif: image block is out of bounds"));
        }
        if delay > 0 || disposal != 0 || self.transparent.is_some() {
            let delay = delay.to_le_bytes();
            self.output.write(&[
                0x21,
                0xf9,
                0x04,
                u8::from(self.transparent.is_some()) | disposal << 2,
                delay[0],
                delay[1],
                self.transparent.unwrap_or(0),
                0x00,
            ])?;
        }
        let mut descriptor = [0x2c_u8; 9];
        descriptor[1..3].copy_from_slice(&(left as u16).to_le_bytes());
        descriptor[3..5].copy_from_slice(&(top as u16).to_le_bytes());
        descriptor[5..7].copy_from_slice(&(width as u16).to_le_bytes());
        descriptor[7..9].copy_from_slice(&(height as u16).to_le_bytes());
        self.output.write(&descriptor)?;
        // The frame's palette is the global palette slice: use the global table.
        self.output.write(&[0])?;
        let literal_width = (self.padded_size + 1).max(2);
        self.output.write(&[literal_width])?;
        let pixels = &pixels[..width * height];
        if literal_width < 8 {
            let max_literal = (1_u8 << literal_width) - 1;
            if pixels.iter().any(|&index| index > max_literal) {
                return Err(io::Error::other(
                    "lzw: input byte too large for the litWidth",
                ));
            }
        }
        write_sub_blocks(&mut self.output, &lzw(pixels, literal_width))?;
        // blockWriter.close flushes the encoder's buffered writer.
        self.output.flush()
    }

    /// writer.go EncodeAll: the trailer and a final flush.
    pub fn finish(mut self) -> io::Result<W> {
        self.output.write(&[0x3b])?;
        self.output.flush()?;
        Ok(self.output.inner)
    }
}

/// blockWriter: (n, n bytes) sub-blocks of at most 255 bytes, then the
/// block terminator, appended to a pending sub-block when there is one.
fn write_sub_blocks<W: Write>(output: &mut GoBufWriter<W>, data: &[u8]) -> io::Result<()> {
    let mut block = [0_u8; 257];
    let mut chunks = data.chunks(255).peekable();
    while let Some(chunk) = chunks.next() {
        block[0] = chunk.len() as u8;
        block[1..=chunk.len()].copy_from_slice(chunk);
        if chunk.len() == 255 {
            output.write(&block[..256])?;
            if chunks.peek().is_none() {
                output.write(&[0])?;
            }
        } else {
            block[chunk.len() + 1] = 0;
            output.write(&block[..chunk.len() + 2])?;
        }
    }
    if data.is_empty() {
        output.write(&[0])?;
    }
    Ok(())
}

/// Go bufio.Writer with its default 4096-byte buffer: the wrapped writer
/// receives full buffers and explicit flushes only.
struct GoBufWriter<W: Write> {
    inner: W,
    buffer: Vec<u8>,
}

impl<W: Write> GoBufWriter<W> {
    const SIZE: usize = 4096;

    fn new(inner: W) -> Self {
        Self {
            inner,
            buffer: Vec::with_capacity(Self::SIZE),
        }
    }

    fn write(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while bytes.len() > Self::SIZE - self.buffer.len() {
            if self.buffer.is_empty() {
                // Large writes bypass an empty buffer.
                self.inner.write_all(bytes)?;
                return Ok(());
            }
            let n = Self::SIZE - self.buffer.len();
            self.buffer.extend_from_slice(&bytes[..n]);
            bytes = &bytes[n..];
            self.flush()?;
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            self.inner.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_frames(bytes: &[u8]) -> Vec<(u16, u16, u16, u16, u16, Vec<u8>)> {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options.read_info(bytes).unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            frames.push((
                frame.left,
                frame.top,
                frame.width,
                frame.height,
                frame.delay,
                frame.buffer.to_vec(),
            ));
        }
        frames
    }

    #[test]
    fn image_gif_layout_follows_go_encode_all() {
        // writer.go writeHeader: fColorTable | log2(256) = 0x87, background
        // and aspect zero, the 768-byte global table, then NETSCAPE2.0 with
        // LoopCount 0 because there is more than one image.
        let palette: Vec<[u8; 4]> = (0..=255).map(|i| [i, 255 - i, i / 2, 255]).collect();
        let first: Vec<u8> = (0..=255).cycle().take(40 * 30).collect();
        let second = vec![7_u8; 2 * 3];
        let mut encoder = ImageGifEncoder::new(Vec::new(), 40, 30, &palette, 2, 0).unwrap();
        encoder.write_frame(0, 0, 40, 30, 12, 1, &first).unwrap();
        encoder.write_frame(5, 6, 2, 3, 0, 1, &second).unwrap();
        let bytes = encoder.finish().unwrap();
        assert_eq!(&bytes[..13], b"GIF89a\x28\x00\x1e\x00\x87\x00\x00");
        assert_eq!(bytes[13..16], [0, 255, 0]);
        assert_eq!(bytes[13 + 3 * 255..13 + 768], [255, 0, 127]);
        let netscape = 13 + 768;
        assert_eq!(
            &bytes[netscape..netscape + 19],
            b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00"
        );
        // DisposalNone (1) always writes the graphic control extension; the
        // image block uses the global table (packed 0) and litWidth 8.
        let block = netscape + 19;
        assert_eq!(
            &bytes[block..block + 19],
            b"\x21\xf9\x04\x04\x0c\x00\x00\x00\x2c\x00\x00\x00\x00\x28\x00\x1e\x00\x00\x08"
        );
        assert_eq!(*bytes.last().unwrap(), 0x3b);
        let frames = decode_frames(&bytes);
        assert_eq!(frames[0], (0, 0, 40, 30, 12, first));
        assert_eq!(frames[1], (5, 6, 2, 3, 0, second));
    }

    #[test]
    fn image_gif_small_palettes_single_frames_and_transparency() {
        // Three colors pad to four (packed 0x81) with litWidth 2; a single
        // image has no loop extension; an alpha-zero entry is the transparent
        // index, and no delay or disposal still writes the extension for it.
        let palette = [[1, 2, 3, 255], [4, 5, 6, 0], [7, 8, 9, 255]];
        let pixels = [0, 1, 2, 1];
        let mut encoder = ImageGifEncoder::new(Vec::new(), 2, 2, &palette, 1, 0).unwrap();
        encoder.write_frame(0, 0, 2, 2, 0, 0, &pixels).unwrap();
        let bytes = encoder.finish().unwrap();
        assert_eq!(
            &bytes[10..25],
            b"\x81\x00\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x00\x00\x00"
        );
        assert_eq!(&bytes[25..33], b"\x21\xf9\x04\x01\x00\x00\x01\x00");
        assert_eq!(bytes[33 + 10], 2);
        assert!(!bytes.windows(11).any(|w| w == b"NETSCAPE2.0"));
        assert_eq!(decode_frames(&bytes)[0].5, pixels);
        // Opaque, no delay and no disposal: no extension at all.
        let mut encoder =
            ImageGifEncoder::new(Vec::new(), 2, 2, &[[0, 0, 0, 255]; 2], 1, 0).unwrap();
        encoder
            .write_frame(0, 0, 2, 2, 0, 0, &[0, 1, 1, 0])
            .unwrap();
        assert_eq!(encoder.finish().unwrap()[19], 0x2c);
    }

    #[test]
    fn image_gif_rejects_what_encode_all_rejects() {
        let error = |r: io::Result<ImageGifEncoder<Vec<u8>>>| r.err().unwrap().to_string();
        assert_eq!(
            error(ImageGifEncoder::new(Vec::new(), 1, 1, &[[0; 4]], 0, 0)),
            "gif: must provide at least one image"
        );
        assert_eq!(
            error(ImageGifEncoder::new(Vec::new(), 1, 1, &[], 1, 0)),
            "gif: cannot encode image block with empty palette"
        );
        assert_eq!(
            error(ImageGifEncoder::new(Vec::new(), 1, 1, &[[0; 4]; 257], 1, 0)),
            "gif: cannot encode color table with more than 256 entries"
        );
        let mut encoder = ImageGifEncoder::new(Vec::new(), 2, 2, &[[0; 4]; 4], 1, 0).unwrap();
        assert_eq!(
            encoder
                .write_frame(1, 1, 2, 1, 0, 1, &[0, 0])
                .unwrap_err()
                .to_string(),
            "gif: image block is out of bounds"
        );
        assert_eq!(
            encoder
                .write_frame(0, 0, 1, 1, 0, 1, &[4])
                .unwrap_err()
                .to_string(),
            "lzw: input byte too large for the litWidth"
        );
    }

    #[test]
    fn image_gif_writes_through_go_bufio_chunks() {
        // bufio.NewWriter(4096): the destination sees full buffers and the
        // flush after each image block, never the encoder's small writes.
        struct Sizes(Vec<usize>);
        impl Write for Sizes {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.push(bytes.len());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let palette = vec![[9, 9, 9, 255]; 256];
        let noise: Vec<u8> = (0..100_u32 * 100)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let mut encoder =
            ImageGifEncoder::new(Sizes(Vec::new()), 100, 100, &palette, 2, 0).unwrap();
        encoder.write_frame(0, 0, 100, 100, 1, 1, &noise).unwrap();
        let after_first = encoder.output.inner.0.clone();
        assert!(after_first.len() > 1);
        assert!(
            after_first[..after_first.len() - 1]
                .iter()
                .all(|&n| n == 4096)
        );
        encoder.write_frame(0, 0, 1, 1, 1, 1, &[0]).unwrap();
        let sizes = encoder.finish().unwrap().0;
        assert_eq!(sizes.len(), after_first.len() + 2);
        assert_eq!(*sizes.last().unwrap(), 1);
    }
}
