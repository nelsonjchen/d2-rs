// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 image/png/writer.go, hash/crc32 and bufio/bufio.go,
// with row filtering from d2cli/raster_png_encoder.go.
// Copyright 2009 The Go Authors. All rights reserved. BSD-3-Clause;
// see THIRD_PARTY_NOTICES.md.

//! The RGB/NRGBA8 encoder reached by d2raster.EncodePNG (png.BestSpeed).

use std::io::{self, Write};

// Go hash/crc32 IEEE, slicing by eight (crc32_generic.go slicingUpdate).
const CRC_TABLES: [[u32; 256]; 8] = {
    let mut tables = [[0_u32; 256]; 8];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0_u32.wrapping_sub(crc & 1));
            bit += 1;
        }
        tables[0][index] = crc;
        index += 1;
    }
    let mut index = 0;
    while index < 256 {
        let mut crc = tables[0][index];
        let mut table = 1;
        while table < 8 {
            crc = tables[0][(crc & 0xff) as usize] ^ (crc >> 8);
            tables[table][index] = crc;
            table += 1;
        }
        index += 1;
    }
    tables
};

pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let tables = &CRC_TABLES;
    let mut crc = !0_u32;
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        crc ^= u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        crc = tables[0][usize::from(chunk[7])]
            ^ tables[1][usize::from(chunk[6])]
            ^ tables[2][usize::from(chunk[5])]
            ^ tables[3][usize::from(chunk[4])]
            ^ tables[4][(crc >> 24) as usize]
            ^ tables[5][((crc >> 16) & 0xff) as usize]
            ^ tables[6][((crc >> 8) & 0xff) as usize]
            ^ tables[7][(crc & 0xff) as usize];
    }
    for &byte in chunks.remainder() {
        crc = tables[0][((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

fn chunk(output: &mut Vec<u8>, kind: &[u8; 4], bytes: &[u8]) {
    output.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    let start = output.len();
    output.extend_from_slice(kind);
    output.extend_from_slice(bytes);
    output.extend_from_slice(&crc32(&output[start..]).to_be_bytes());
}

// D2 lib/png/png.go:WriteExif/writeD2ExifChunk. Only the CLI's standalone
// PNG encoder writes this metadata; PDF/PPTX image encoders do not.
pub(crate) fn with_exif(mut png: Vec<u8>, version: &str) -> Result<Vec<u8>, String> {
    const IEND: &[u8] = b"\0\0\0\0IEND\xae\x42\x60\x82";
    // Match AddExifToEncoderOutputInPlace's trusted encoder-output boundary.
    if png.len() < 45 || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("not PNG encoder output".into());
    }
    if &png[8..16] != b"\0\0\0\rIHDR" {
        return Err("PNG encoder output has an invalid IHDR chunk".into());
    }
    if crc32(&png[12..29]).to_be_bytes() != png[29..33] {
        return Err("PNG encoder output has an invalid IHDR CRC".into());
    }
    if !png.ends_with(IEND) {
        return Err("PNG encoder output has an invalid IEND chunk".into());
    }
    if version.len() > i32::MAX as usize - 39 {
        return Err(format!(
            "PNG EXIF model is too large: {} bytes",
            version.len()
        ));
    }
    let data_size = if version.len() < 4 {
        38
    } else {
        39 + version.len()
    };
    let mut data = vec![0; data_size];
    data[..10].copy_from_slice(b"MM\0\x2a\0\0\0\x08\0\x02");
    data[10..22].copy_from_slice(b"\x01\x0f\0\x02\0\0\0\x03D2\0\0");
    data[22..26].copy_from_slice(b"\x01\x10\0\x02");
    data[26..30].copy_from_slice(&((version.len() + 1) as u32).to_be_bytes());
    if version.len() < 4 {
        data[30..30 + version.len()].copy_from_slice(version.as_bytes());
    } else {
        data[30..34].copy_from_slice(&38_u32.to_be_bytes());
        data[38..38 + version.len()].copy_from_slice(version.as_bytes());
    }
    let mut exif = Vec::with_capacity(data_size + 12);
    chunk(&mut exif, b"eXIf", &data);
    png.splice(33..33, exif);
    Ok(png)
}

// Go's 32 KiB bufio.Writer preserves individual large writes. Merely splitting
// the finished zlib stream into equal chunks changes the PNG chunk boundaries.
struct Idats {
    output: Vec<u8>,
    buffer: Vec<u8>,
}

impl Write for Idats {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len();
        while bytes.len() > 32768 - self.buffer.len() {
            if self.buffer.is_empty() {
                chunk(&mut self.output, b"IDAT", bytes);
                return Ok(count);
            }
            let n = 32768 - self.buffer.len();
            self.buffer.extend_from_slice(&bytes[..n]);
            bytes = &bytes[n..];
            self.flush()?;
        }
        self.buffer.extend_from_slice(bytes);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            chunk(&mut self.output, b"IDAT", &self.buffer);
            self.buffer.clear();
        }
        Ok(())
    }
}

fn abs8(value: u8) -> usize {
    usize::from((value as i8).unsigned_abs())
}

/// Go's filter selection, including its tie order: Up, Paeth, None, Sub, Avg.
///
/// d2cli/raster_png_encoder.go rasterPNGFilteredRow: each candidate stops
/// scoring once it reaches the best cost so far. Only a strictly lower cost
/// replaces the selection, so a stopped candidate could never have won, and
/// the selection equals image/png's exhaustive scoring.
struct RowFilter {
    channels: usize,
    // Tag byte plus the raw row (tag 0 doubles as the None candidate).
    raw: Vec<u8>,
    prior: Vec<u8>,
    candidates: [Vec<u8>; 2],
}

impl RowFilter {
    fn new(row_bytes: usize, channels: usize) -> Self {
        Self {
            channels,
            raw: vec![0; row_bytes + 1],
            prior: vec![0; row_bytes + 1],
            candidates: [vec![0; row_bytes + 1], vec![0; row_bytes + 1]],
        }
    }

    fn raw_mut(&mut self) -> &mut [u8] {
        &mut self.raw[1..]
    }

    /// Filter the row written through `raw_mut`, pass it to `emit`, and make
    /// it the prior row.
    fn emit(&mut self, emit: &mut impl FnMut(&[u8])) {
        let channels = self.channels;
        let raw = &self.raw[1..];
        let prior = &self.prior[1..];
        let [first, second] = &mut self.candidates;
        let (mut winner, mut scratch) = (first, second);
        let mut best = up_filter(winner, raw, prior);
        let mut selected = 2;
        let cost = paeth_filter(scratch, raw, prior, best, channels);
        if cost < best {
            (best, selected) = (cost, 4);
            std::mem::swap(&mut winner, &mut scratch);
        }
        let cost = none_cost(raw, best);
        if cost < best {
            (best, selected) = (cost, 0);
        }
        let cost = sub_filter(scratch, raw, best, channels);
        if cost < best {
            (best, selected) = (cost, 1);
            std::mem::swap(&mut winner, &mut scratch);
        }
        if average_filter(scratch, raw, prior, best, channels) < best {
            selected = 3;
            winner = scratch;
        }
        if selected == 0 {
            emit(&self.raw);
        } else {
            emit(winner);
        }
        std::mem::swap(&mut self.raw, &mut self.prior);
        self.raw[0] = 0;
    }
}

// Candidates are scored in fixed-size blocks and stop after the first block
// that reaches `stop_at`. A stopped candidate returns a cost of at least
// `stop_at`, which is rejected exactly as Go's byte-wise early exit is.
const FILTER_BLOCK: usize = 256;

/// Score `raw[channels..]` in blocks: `filter(index)` returns the filtered
/// byte for one index, stored in `destination`. Returns the accumulated cost.
#[inline(always)]
fn score_blocks(
    destination: &mut [u8],
    mut cost: usize,
    start: usize,
    end: usize,
    stop_at: usize,
    filter: impl Fn(usize) -> u8,
) -> usize {
    let mut block = start;
    while block < end {
        let block_end = (block + FILTER_BLOCK).min(end);
        let mut block_cost = 0;
        for (index, output) in (block..block_end).zip(&mut destination[block..block_end]) {
            *output = filter(index);
            block_cost += abs8(*output);
        }
        cost += block_cost;
        if cost >= stop_at {
            break;
        }
        block = block_end;
    }
    cost
}

fn none_cost(raw: &[u8], stop_at: usize) -> usize {
    let mut cost = 0;
    for block in raw.chunks(FILTER_BLOCK) {
        cost += block.iter().map(|&value| abs8(value)).sum::<usize>();
        if cost >= stop_at {
            break;
        }
    }
    cost
}

fn up_filter(destination: &mut [u8], raw: &[u8], prior: &[u8]) -> usize {
    destination[0] = 2;
    let destination = &mut destination[1..];
    if raw == prior {
        destination.fill(0);
        return 0;
    }
    let mut cost = 0;
    for ((output, &value), &above) in destination.iter_mut().zip(raw).zip(prior) {
        *output = value.wrapping_sub(above);
        cost += abs8(*output);
    }
    cost
}

fn sub_filter(destination: &mut [u8], raw: &[u8], stop_at: usize, channels: usize) -> usize {
    destination[0] = 1;
    let destination = &mut destination[1..];
    let mut cost = 0;
    for index in 0..channels.min(raw.len()) {
        destination[index] = raw[index];
        cost += abs8(destination[index]);
        if cost >= stop_at {
            return cost;
        }
    }
    score_blocks(destination, cost, channels, raw.len(), stop_at, |index| {
        raw[index].wrapping_sub(raw[index - channels])
    })
}

fn average_filter(
    destination: &mut [u8],
    raw: &[u8],
    prior: &[u8],
    stop_at: usize,
    channels: usize,
) -> usize {
    destination[0] = 3;
    let destination = &mut destination[1..];
    let mut cost = 0;
    for index in 0..channels.min(raw.len()) {
        destination[index] = raw[index].wrapping_sub(prior[index] / 2);
        cost += abs8(destination[index]);
        if cost >= stop_at {
            return cost;
        }
    }
    let prior = &prior[..raw.len()];
    score_blocks(destination, cost, channels, raw.len(), stop_at, |index| {
        let average = ((u16::from(raw[index - channels]) + u16::from(prior[index])) / 2) as u8;
        raw[index].wrapping_sub(average)
    })
}

fn paeth_filter(
    destination: &mut [u8],
    raw: &[u8],
    prior: &[u8],
    stop_at: usize,
    channels: usize,
) -> usize {
    destination[0] = 4;
    let destination = &mut destination[1..];
    let mut cost = 0;
    for index in 0..channels.min(raw.len()) {
        destination[index] = raw[index].wrapping_sub(prior[index]);
        cost += abs8(destination[index]);
        if cost >= stop_at {
            return cost;
        }
    }
    let prior = &prior[..raw.len()];
    score_blocks(destination, cost, channels, raw.len(), stop_at, |index| {
        let left = i16::from(raw[index - channels]);
        let upper_left = i16::from(prior[index - channels]);
        let upper = i16::from(prior[index]);
        let left_distance = (upper - upper_left).abs();
        let upper_distance = (left - upper_left).abs();
        let diagonal_distance = (left + upper - 2 * upper_left).abs();
        let predictor = if left_distance <= upper_distance && left_distance <= diagonal_distance {
            left
        } else if upper_distance <= diagonal_distance {
            upper
        } else {
            upper_left
        };
        raw[index].wrapping_sub(predictor as u8)
    })
}

/// Filter each `row_bytes` row of `pixels` (see `RowFilter`).
pub(crate) fn filter_rows(
    pixels: &[u8],
    row_bytes: usize,
    channels: usize,
    mut emit: impl FnMut(&[u8]),
) {
    let mut filter = RowFilter::new(row_bytes, channels);
    for row in pixels.chunks_exact(row_bytes) {
        filter.raw_mut().copy_from_slice(row);
        filter.emit(&mut emit);
    }
}

pub(crate) fn encode(
    width: u32,
    height: u32,
    pixels: &[u8],
    channels: usize,
) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(channels));
    if width == 0 || height == 0 || !matches!(channels, 3 | 4) || expected != Some(pixels.len()) {
        return Err("invalid PNG image dimensions or pixel buffer".into());
    }
    // image.NRGBA.Opaque selects truecolor even for an RGBA input buffer.
    // Opaque rows are converted to RGB one at a time as they are filtered.
    let opaque = channels == 4
        && pixels
            .chunks(1 << 14)
            .all(|block| block.chunks_exact(4).fold(255, |alpha, p| alpha & p[3]) == 255);
    let mut stream = RowStream::new(width, height, if opaque { 3 } else { channels });
    stream.push_rows(pixels, channels);
    Ok(stream.finish())
}

/// A PNG written row by row: the encoder state of `encode` without the
/// complete frame. Rows arrive with `input_channels` bytes per pixel and are
/// stored with `channels` (an RGBA input stored as RGB drops alpha, which the
/// caller has established to be opaque).
pub(crate) struct RowStream {
    width: usize,
    channels: usize,
    filter: RowFilter,
    zlib: d2_rs_goflate::zlib::Writer<Idats>,
}

impl RowStream {
    pub(crate) fn new(width: u32, height: u32, channels: usize) -> Self {
        let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = Vec::from(width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, if channels == 3 { 2 } else { 6 }, 0, 0, 0]);
        chunk(&mut output, b"IHDR", &header);
        let sink = Idats {
            output,
            buffer: Vec::with_capacity(32768),
        };
        Self {
            width: width as usize,
            channels,
            filter: RowFilter::new(width as usize * channels, channels),
            zlib: d2_rs_goflate::zlib::Writer::with_level(sink, d2_rs_goflate::flate::BEST_SPEED)
                .expect("valid compression level"),
        }
    }

    pub(crate) fn push_rows(&mut self, pixels: &[u8], input_channels: usize) {
        let zlib = &mut self.zlib;
        let mut write = |row: &[u8]| {
            zlib.write(row).expect("writing to a Vec cannot fail");
        };
        for row in pixels.chunks_exact(self.width * input_channels) {
            if input_channels == 4 && self.channels == 3 {
                for (output, pixel) in self
                    .filter
                    .raw_mut()
                    .chunks_exact_mut(3)
                    .zip(row.chunks_exact(4))
                {
                    output.copy_from_slice(&pixel[..3]);
                }
            } else {
                self.filter.raw_mut().copy_from_slice(row);
            }
            self.filter.emit(&mut write);
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        let mut zlib = self.zlib;
        zlib.close().expect("writing to a Vec cannot fail");
        let mut sink = zlib.into_inner();
        sink.flush().expect("writing to a Vec cannot fail");
        chunk(&mut sink.output, b"IEND", &[]);
        sink.output
    }
}

/// NRGBA rows with image.NRGBA.Opaque's whole-frame color-type choice.
/// Retain only the opaque prefix: transparency or the last row establishes
/// the final color type, so a complete frame never needs a second copy.
// TODO(legacy-raster-cleanup): only the SVG bridge streamed paged bands; the
// scene path encodes whole paged frames as paged.go does.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct RgbaRowStream {
    width: u32,
    height: u32,
    opaque_prefix: Vec<u8>,
    stream: Option<RowStream>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl RgbaRowStream {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            opaque_prefix: Vec::new(),
            stream: None,
        }
    }

    pub(crate) fn push_rows(&mut self, pixels: &[u8]) {
        if self.stream.is_none() {
            let opaque = pixels
                .chunks(1 << 14)
                .all(|block| block.chunks_exact(4).fold(255, |alpha, p| alpha & p[3]) == 255);
            if !opaque
                || self.opaque_prefix.len() + pixels.len()
                    == self.width as usize * self.height as usize * 4
            {
                let mut stream =
                    RowStream::new(self.width, self.height, if opaque { 3 } else { 4 });
                stream.push_rows(&self.opaque_prefix, 4);
                self.opaque_prefix = Vec::new();
                self.stream = Some(stream);
            }
        }
        if let Some(stream) = &mut self.stream {
            stream.push_rows(pixels, 4);
        } else {
            if self.opaque_prefix.is_empty() {
                self.opaque_prefix
                    .reserve_exact(self.width as usize * self.height as usize * 4);
            }
            self.opaque_prefix.extend_from_slice(pixels);
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        match self.stream {
            Some(stream) => stream.finish(),
            None => {
                let mut stream = RowStream::new(self.width, self.height, 3);
                stream.push_rows(&self.opaque_prefix, 4);
                stream.finish()
            }
        }
    }
}

/// FPDF png.go splits already-filtered NRGBA rows, retaining each filter tag.
/// xcompr.go then compresses each plane with zlib.BestSpeed in a single write.
pub(crate) fn pdf_streams(
    width: u32,
    rgb: &[u8],
    alpha: Option<&[u8]>,
) -> (Vec<u8>, Option<Vec<u8>>) {
    let mut color_rows = Vec::new();
    let mut alpha_rows = Vec::new();
    if let Some(alpha) = alpha {
        let rgba = rgb
            .chunks_exact(3)
            .zip(alpha)
            .flat_map(|(c, &a)| [c[0], c[1], c[2], a])
            .collect::<Vec<_>>();
        filter_rows(&rgba, width as usize * 4, 4, |row| {
            color_rows.push(row[0]);
            alpha_rows.push(row[0]);
            for pixel in row[1..].chunks_exact(4) {
                color_rows.extend_from_slice(&pixel[..3]);
                alpha_rows.push(pixel[3]);
            }
        });
    } else {
        filter_rows(rgb, width as usize * 3, 3, |row| {
            color_rows.extend_from_slice(row)
        });
    }
    let compress =
        |rows: &[u8]| d2_rs_goflate::zlib_compress(rows, d2_rs_goflate::flate::BEST_SPEED);
    (compress(&color_rows), alpha.map(|_| compress(&alpha_rows)))
}

#[cfg(test)]
pub(crate) fn decode_predictor(width: u32, height: u32, channels: usize, stream: &[u8]) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::from(width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, if channels == 1 { 0 } else { 2 }, 0, 0, 0]);
    chunk(&mut bytes, b"IHDR", &header);
    chunk(&mut bytes, b"IDAT", stream);
    chunk(&mut bytes, b"IEND", &[]);
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    pixels
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use base64::Engine;

    #[test]
    fn rgba_bands_preserve_whole_frame_color_type_and_bytes() {
        let (width, height) = (113, 17);
        for transparent_row in [None, Some(0), Some(8), Some(16)] {
            let mut pixels = Vec::new();
            for y in 0..height {
                for x in 0..width {
                    pixels.extend_from_slice(&[
                        (x * 13 + y * 7) as u8,
                        (x * 3 + y * 23) as u8,
                        (x * 29 + y * 5) as u8,
                        if transparent_row == Some(y) { 127 } else { 255 },
                    ]);
                }
            }
            let expected = super::encode(width, height, &pixels, 4).unwrap();
            for rows in [1, 3, height] {
                let mut stream = super::RgbaRowStream::new(width, height);
                for band in pixels.chunks(width as usize * rows as usize * 4) {
                    stream.push_rows(band);
                }
                assert_eq!(
                    stream.finish(),
                    expected,
                    "alpha {transparent_row:?}, rows {rows}"
                );
            }
        }
    }

    /// image/png writer.go filter: score every candidate in full, in the
    /// order Up, Paeth, None, Sub, Average, keeping the first minimum.
    fn exhaustive_filter(raw: &[u8], prior: &[u8], channels: usize) -> Vec<u8> {
        let paeth = |a: u8, b: u8, c: u8| {
            let p = i32::from(a) + i32::from(b) - i32::from(c);
            let (pa, pb, pc) = (
                (p - i32::from(a)).abs(),
                (p - i32::from(b)).abs(),
                (p - i32::from(c)).abs(),
            );
            if pa <= pb && pa <= pc {
                a
            } else if pb <= pc {
                b
            } else {
                c
            }
        };
        let mut best: Option<(usize, Vec<u8>)> = None;
        for filter in [2_u8, 4, 0, 1, 3] {
            let mut row = vec![filter];
            for i in 0..raw.len() {
                let left = if i >= channels { raw[i - channels] } else { 0 };
                let upper_left = if i >= channels {
                    prior[i - channels]
                } else {
                    0
                };
                let predictor = match filter {
                    0 => 0,
                    1 => left,
                    2 => prior[i],
                    3 => ((u16::from(left) + u16::from(prior[i])) / 2) as u8,
                    _ => paeth(left, prior[i], upper_left),
                };
                row.push(raw[i].wrapping_sub(predictor));
            }
            let cost = row[1..]
                .iter()
                .map(|&byte| usize::from(byte.min(byte.wrapping_neg())))
                .sum();
            if best.as_ref().is_none_or(|(best, _)| cost < *best) {
                best = Some((cost, row));
            }
        }
        best.unwrap().1
    }

    #[test]
    fn early_exit_filter_selection_matches_exhaustive_scoring() {
        let mut state = 0x2545_f491_u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for channels in [3, 4] {
            for width in [1, 2, 5, 90, 700] {
                let row_bytes = width * channels;
                let mut pixels = Vec::new();
                for row in 0..24 {
                    for x in 0..row_bytes {
                        // Mix flat, repeated, gradient and noisy rows so every
                        // filter wins somewhere, including exact ties.
                        let value = match row % 6 {
                            0 => 255,
                            1 => (x * 7 + row) as u8,
                            2 => (next() % 4) as u8 * 60,
                            3 => pixels[(row - 1) * row_bytes + x],
                            4 => (x / channels * 3) as u8,
                            _ => next() as u8,
                        };
                        pixels.push(value);
                    }
                }
                let mut expected = Vec::new();
                let mut prior = vec![0; row_bytes];
                for row in pixels.chunks_exact(row_bytes) {
                    expected.extend(exhaustive_filter(row, &prior, channels));
                    prior.copy_from_slice(row);
                }
                let mut actual = Vec::new();
                super::filter_rows(&pixels, row_bytes, channels, |row| {
                    actual.extend_from_slice(row)
                });
                assert_eq!(actual, expected, "channels {channels}, width {width}");
            }
        }
    }

    #[test]
    fn table_crc_matches_bitwise_crc() {
        let bitwise = |bytes: &[u8]| {
            let mut crc = !0_u32;
            for &byte in bytes {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xedb88320 & 0_u32.wrapping_sub(crc & 1));
                }
            }
            !crc
        };
        let bytes = (0..1000_u32)
            .map(|index| (index * 31 + index / 7) as u8)
            .collect::<Vec<_>>();
        for len in [0, 1, 7, 8, 9, 63, 1000] {
            assert_eq!(super::crc32(&bytes[..len]), bitwise(&bytes[..len]));
        }
        assert_eq!(super::crc32(b"IEND"), 0xae42_6082);
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn standalone_png_exif_matches_complete_go_files() {
        // D2 lib/png.AddExif, including TIFF inline/offset boundaries and
        // byte lengths for Unicode models. Both RGB and RGBA PNGs stay intact.
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/png-exif-source-v090.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let decode = |key: &str| {
                base64::engine::general_purpose::STANDARD
                    .decode(case[key].as_str().unwrap())
                    .unwrap()
            };
            let actual = with_exif(decode("input"), case["version"].as_str().unwrap()).unwrap();
            assert_eq!(actual, decode("expected"), "{case}");
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn png_bytes_match_go_best_speed() {
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/png-encoding-go127.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let decode = |key: &str| {
                base64::engine::general_purpose::STANDARD
                    .decode(case[key].as_str().unwrap())
                    .unwrap()
            };
            let actual = encode(
                case["width"].as_u64().unwrap() as u32,
                case["height"].as_u64().unwrap() as u32,
                &decode("rgba"),
                4,
            )
            .unwrap();
            let expected = decode("png");
            assert!(
                actual == expected,
                "{}: lengths {}/{}, first diff {:?}",
                case["name"],
                actual.len(),
                expected.len(),
                actual.iter().zip(&expected).position(|(a, b)| a != b)
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn pdf_plane_streams_match_go_png_and_fpdf() {
        let cases: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/png-encoding-go127.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let decode = |key: &str| {
                base64::engine::general_purpose::STANDARD
                    .decode(case[key].as_str().unwrap())
                    .unwrap()
            };
            let rgba = decode("rgba");
            let rgb = rgba
                .chunks_exact(4)
                .flat_map(|p| p[..3].iter().copied())
                .collect::<Vec<_>>();
            let alpha = rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
            let (color, mask) = pdf_streams(
                case["width"].as_u64().unwrap() as u32,
                &rgb,
                case.get("pdf_alpha").map(|_| alpha.as_slice()),
            );
            assert!(color == decode("pdf_color"), "{} RGB stream", case["name"]);
            assert!(
                mask == case.get("pdf_alpha").map(|_| decode("pdf_alpha")),
                "{} alpha stream",
                case["name"]
            );
        }
    }
}
