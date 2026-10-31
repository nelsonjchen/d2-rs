// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Copyright 2009 The Go Authors. All rights reserved.
// Go BSD-3-Clause terms: root THIRD_PARTY_NOTICES.md.
//! Shared image/png default encoder and first-frame asset normalization.
//! Go 1.27 image/png/writer.go; internal/rasterimage/firstframe.go.
use std::io::{self, Write};

pub(crate) fn decode_png(data: &[u8]) -> Result<PngImage, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    if width == 0 || height == 0 || width as u64 > 16000000 / height as u64 {
        return Err("native SVG raster asset exceeds 16 million pixels".into());
    }
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("png: image too large")?];
    let frame = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let info = reader.info();
    let depth = frame.bit_depth as usize;
    let source_channels = frame.color_type.samples();
    let indexed = frame.color_type == png::ColorType::Indexed;
    let mut palette = info.palette.as_deref().unwrap_or_default().to_vec();
    let trns = info.trns.as_deref().unwrap_or_default();
    let sample = |row: &[u8], index: usize| -> u16 {
        if depth < 8 {
            ((row[index * depth / 8] >> (8 - depth - index * depth % 8)) & ((1 << depth) - 1))
                as u16
        } else if depth == 8 {
            row[index] as u16
        } else {
            u16::from_be_bytes([row[2 * index], row[2 * index + 1]])
        }
    };
    let mut transparency = Vec::new();
    let (pixels, channels, out_depth, color_type, row_bytes) = if indexed {
        if palette.is_empty() || palette.len() % 3 != 0 || palette.len() > 768 {
            return Err("png: invalid format: missing palette".into());
        }
        let count = palette.len() / 3;
        let bits = if count <= 2 {
            1
        } else if count <= 4 {
            2
        } else if count <= 16 {
            4
        } else {
            8
        };
        let row_bytes = (width as usize * bits).div_ceil(8);
        let mut pixels = vec![0; row_bytes * height as usize];
        for y in 0..height as usize {
            let row = &buffer[y * frame.line_size..(y + 1) * frame.line_size];
            for x in 0..width as usize {
                let index = sample(row, x) as usize;
                if index >= count {
                    return Err("png: invalid palette index".into());
                }
                pixels[y * row_bytes + x * bits / 8] |= (index as u8) << (8 - bits - x * bits % 8);
            }
        }
        if let Some(last) = trns.iter().rposition(|a| *a != 255) {
            transparency.extend_from_slice(&trns[..=last])
        }
        (pixels, 1, bits as u8, 3, row_bytes)
    } else {
        let high = if depth == 16 { 65535 } else { 255 };
        let stride = if depth == 16 { 2 } else { 1 };
        let gray = matches!(
            frame.color_type,
            png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha
        );
        let has_alpha = matches!(
            frame.color_type,
            png::ColorType::GrayscaleAlpha | png::ColorType::Rgba
        );
        let mut rgba = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height as usize {
            let row = &buffer[y * frame.line_size..(y + 1) * frame.line_size];
            for x in 0..width as usize {
                let mut values = [0u16; 4];
                for (i, v) in values.iter_mut().enumerate().take(source_channels) {
                    *v = sample(row, x * source_channels + i)
                }
                if depth < 8 {
                    values[0] = values[0] * 255 / ((1 << depth) - 1) as u16
                }
                let (r, g, b) = if gray {
                    (values[0], values[0], values[0])
                } else {
                    (values[0], values[1], values[2])
                };
                let transparent = if gray && trns.len() >= 2 {
                    values[0]
                        == u16::from_be_bytes([trns[0], trns[1]])
                            * (if depth < 8 {
                                255 / ((1 << depth) - 1) as u16
                            } else {
                                1
                            })
                } else if !gray && trns.len() >= 6 {
                    (r, g, b)
                        == (
                            u16::from_be_bytes([trns[0], trns[1]]),
                            u16::from_be_bytes([trns[2], trns[3]]),
                            u16::from_be_bytes([trns[4], trns[5]]),
                        )
                } else {
                    false
                };
                let alpha = if has_alpha {
                    values[source_channels - 1]
                } else if transparent {
                    0
                } else {
                    high
                };
                rgba.push([r, g, b, alpha]);
            }
        }
        let alpha = rgba.iter().any(|p| p[3] != high);
        let output_gray = gray && !has_alpha && trns.is_empty();
        let channels = if output_gray {
            1
        } else if alpha {
            4
        } else {
            3
        };
        let mut pixels = Vec::with_capacity(rgba.len() * channels * stride);
        for p in rgba {
            for v in p.iter().take(channels) {
                if stride == 2 {
                    pixels.extend_from_slice(&v.to_be_bytes())
                } else {
                    pixels.push(*v as u8)
                }
            }
        }
        palette.clear();
        (
            pixels,
            channels * stride,
            if stride == 2 { 16 } else { 8 },
            if output_gray {
                0
            } else if alpha {
                6
            } else {
                2
            },
            width as usize * channels * stride,
        )
    };
    Ok(PngImage {
        width,
        height,
        pixels,
        channels,
        out_depth,
        color_type,
        row_bytes,
        palette,
        transparency,
    })
}

/// The concrete Go image model controls bit depth, palette and filtering.
pub(crate) struct PngImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub channels: usize,
    pub out_depth: u8,
    pub color_type: u8,
    pub row_bytes: usize,
    pub palette: Vec<u8>,
    pub transparency: Vec<u8>,
}

impl PngImage {
    pub(crate) fn encode(self) -> Result<Vec<u8>, String> {
        let Self {
            width,
            height,
            pixels,
            channels,
            out_depth,
            color_type,
            row_bytes,
            palette,
            transparency,
        } = self;
        let indexed = color_type == 3;
        let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = width.to_be_bytes().to_vec();
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[out_depth, color_type, 0, 0, 0]);
        chunk(&mut output, b"IHDR", &header);
        if indexed {
            chunk(&mut output, b"PLTE", &palette);
            if !transparency.is_empty() {
                chunk(&mut output, b"tRNS", &transparency)
            }
        }
        let sink = Idats {
            output,
            buffer: Vec::with_capacity(32768),
        };
        let mut zlib = d2_rs_goflate::zlib::Writer::with_level(
            sink,
            d2_rs_goflate::flate::DEFAULT_COMPRESSION,
        )
        .map_err(|e| e.to_string())?;
        if indexed {
            for row in pixels.chunks_exact(row_bytes) {
                zlib.write(&[0]).map_err(|e| e.to_string())?;
                zlib.write(row).map_err(|e| e.to_string())?;
            }
        } else {
            filter_rows(&pixels, row_bytes, channels, |row| {
                zlib.write(row).expect("Vec writer");
            })
        }
        zlib.close().map_err(|e| e.to_string())?;
        let mut sink = zlib.into_inner();
        sink.flush().map_err(|e| e.to_string())?;
        chunk(&mut sink.output, b"IEND", &[]);
        Ok(sink.output)
    }

    pub(crate) fn from_rgba(
        width: u32,
        height: u32,
        rgba: Vec<[u16; 4]>,
        sixteen: bool,
        gray: bool,
    ) -> Self {
        let alpha = rgba.iter().any(|p| p[3] != 65535);
        let count = if gray {
            1
        } else if alpha {
            4
        } else {
            3
        };
        let stride = if sixteen { 2 } else { 1 };
        let mut pixels = Vec::with_capacity(rgba.len() * count * stride);
        for p in rgba {
            for c in p.iter().take(count) {
                if sixteen {
                    pixels.extend_from_slice(&c.to_be_bytes())
                } else {
                    pixels.push((c >> 8) as u8)
                }
            }
        }
        Self {
            width,
            height,
            pixels,
            channels: count * stride,
            out_depth: if sixteen { 16 } else { 8 },
            color_type: if gray {
                0
            } else if alpha {
                6
            } else {
                2
            },
            row_bytes: width as usize * count * stride,
            palette: Vec::new(),
            transparency: Vec::new(),
        }
    }
}

// Copyright 2009 The Go Authors. BSD-3-Clause; see root THIRD_PARTY_NOTICES.md.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
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

fn paeth(left: u8, above: u8, upper_left: u8) -> u8 {
    let p = i32::from(left) + i32::from(above) - i32::from(upper_left);
    let a = (p - i32::from(left)).abs();
    let b = (p - i32::from(above)).abs();
    let c = (p - i32::from(upper_left)).abs();
    if a <= b && a <= c {
        left
    } else if b <= c {
        above
    } else {
        upper_left
    }
}

/// Go's filter selection, including its tie order: Up, Paeth, None, Sub, Avg.
pub(crate) fn filter_rows(
    pixels: &[u8],
    row_bytes: usize,
    channels: usize,
    mut emit: impl FnMut(&[u8]),
) {
    let mut previous = vec![0; row_bytes];
    let mut rows = vec![vec![0; row_bytes + 1]; 5];
    for pixels in pixels.chunks_exact(row_bytes) {
        rows[0][1..].copy_from_slice(pixels);
        let mut best = usize::MAX;
        let mut selected = 0;
        for filter in [2, 4, 0, 1, 3] {
            rows[filter][0] = filter as u8;
            let mut sum = 0usize;
            for (i, &pixel) in pixels.iter().enumerate() {
                let left = if i >= channels {
                    pixels[i - channels]
                } else {
                    0
                };
                let above = previous[i];
                let upper_left = if i >= channels {
                    previous[i - channels]
                } else {
                    0
                };
                let predictor = match filter {
                    0 => 0,
                    1 => left,
                    2 => above,
                    3 => ((u16::from(left) + u16::from(above)) / 2) as u8,
                    4 => paeth(left, above, upper_left),
                    _ => unreachable!(),
                };
                let byte = pixel.wrapping_sub(predictor);
                rows[filter][i + 1] = byte;
                sum += usize::from(byte.min(byte.wrapping_neg()));
            }
            if sum < best {
                best = sum;
                selected = filter;
            }
        }
        emit(&rows[selected]);
        previous.copy_from_slice(pixels);
    }
}

/// Normalize an authored raster asset's first frame, using the same 16-million
/// pixel / 32-MiB output bounds as D2's isometric SVG writer.
/// `format` is png, gif, jpeg or webp (image/ MIME prefixes are accepted).
/// Decoding and encoding run entirely in Rust.
pub fn normalize_first_frame_png(data: &[u8], format: &str) -> Result<Vec<u8>, String> {
    let encoded = crate::first_frame::decode(data, format)?.encode()?;
    if encoded.len() > 32 << 20 {
        return Err("native SVG raster asset exceeds 32 MiB".into());
    }
    Ok(encoded)
}

/// Encode a straight-alpha frame using Go image/png's default compression.
/// Row padding and nonzero rectangle origins are preserved as image bounds.
pub fn encode_png_default(frame: &crate::NrgbaImage) -> Result<Vec<u8>, String> {
    let (w, h) = (frame.rect.dx(), frame.rect.dy());
    if w <= 0 || h <= 0 || w >= 1 << 32 || h >= 1 << 32 {
        return Err(format!("png: invalid format: invalid image size: {w}x{h}"));
    }
    let row = (w as usize).checked_mul(4).ok_or("png: image too large")?;
    let needed = (h as usize - 1)
        .checked_mul(frame.stride)
        .and_then(|n| n.checked_add(row))
        .ok_or("png: image too large")?;
    if frame.stride < row || frame.pix.len() < needed {
        return Err("png: invalid NRGBA storage".into());
    }
    let opaque = (0..h as usize).all(|y| {
        frame.pix[y * frame.stride..y * frame.stride + row]
            .chunks_exact(4)
            .all(|p| p[3] == 255)
    });
    let channels = if opaque { 3 } else { 4 };
    let mut pixels = Vec::with_capacity(w as usize * h as usize * channels);
    for y in 0..h as usize {
        for p in frame.pix[y * frame.stride..y * frame.stride + row].chunks_exact(4) {
            pixels.extend_from_slice(&p[..channels]);
        }
    }
    PngImage {
        width: w as u32,
        height: h as u32,
        pixels,
        channels,
        out_depth: 8,
        color_type: if opaque { 2 } else { 6 },
        row_bytes: w as usize * channels,
        palette: Vec::new(),
        transparency: Vec::new(),
    }
    .encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    #[cfg(feature = "oracle-fixtures")]
    fn contracts() -> Vec<serde_json::Value> {
        // tools/isometric-probe/png_contract_test.go, pinned Go DecodeFirst
        // followed by png.Encode (recipe mathtext-raster-static-png).
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/static-png.json")).unwrap()
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn shared_static_png_models_match_go() {
        for record in contracts() {
            assert_eq!(
                normalize_first_frame_png(
                    &STANDARD.decode(record["Input"].as_str().unwrap()).unwrap(),
                    "image/png"
                )
                .unwrap(),
                STANDARD
                    .decode(record["Expected"].as_str().unwrap())
                    .unwrap(),
                "{}",
                record["Name"]
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn default_frame_encoder_matches_go_with_stride_and_origin() {
        for alpha in [255u8, 80] {
            let mut frame = crate::NrgbaImage {
                rect: crate::Rect::new(-10, 20, -3, 23),
                stride: 32,
                pix: vec![0xa5; 92],
            };
            for y in 0..3 {
                for x in 0..7 {
                    frame.pix[y * 32 + x * 4..y * 32 + x * 4 + 4].copy_from_slice(&[
                        x as u8 * 20,
                        y as u8 * 30,
                        123,
                        alpha,
                    ]);
                }
            }
            let record = contracts()
                .into_iter()
                .find(|r| r["Name"] == char::from(alpha).to_string())
                .unwrap();
            assert_eq!(
                encode_png_default(&frame).unwrap(),
                STANDARD
                    .decode(record["Expected"].as_str().unwrap())
                    .unwrap()
            );
        }
    }

    #[test]
    fn malformed_frame_storage_returns_an_error() {
        assert!(encode_png_default(&crate::NrgbaImage::default()).is_err());
        assert!(
            encode_png_default(&crate::NrgbaImage {
                rect: crate::Rect::new(0, 0, 3, 2),
                stride: 12,
                pix: vec![0; 23]
            })
            .is_err()
        );
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn partial_apng_source_and_over_preserve_go_alpha_rounding() {
        let records: Vec<serde_json::Value> =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/apng-alpha.json")).unwrap();
        for record in records {
            assert_eq!(
                normalize_first_frame_png(
                    &STANDARD.decode(record["input"].as_str().unwrap()).unwrap(),
                    "png"
                )
                .unwrap(),
                STANDARD
                    .decode(record["expected"].as_str().unwrap())
                    .unwrap(),
                "{}",
                record["name"]
            );
        }
    }
}
