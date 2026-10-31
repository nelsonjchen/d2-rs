// SPDX-License-Identifier: MPL-2.0
// Translated from Go 1.27.0 image/png/writer.go and bufio/bufio.go.
// Copyright 2009 The Go Authors. All rights reserved. BSD-3-Clause;
// see THIRD_PARTY_NOTICES.md.

//! The RGB/NRGBA8 encoder reached by d2raster.EncodePNG (png.BestSpeed).

use std::io::{self, Write};

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
    let rgb;
    let (pixels, channels) = if channels == 4 && pixels.chunks_exact(4).all(|p| p[3] == 255) {
        rgb = pixels
            .chunks_exact(4)
            .flat_map(|p| p[..3].iter().copied())
            .collect::<Vec<_>>();
        (rgb.as_slice(), 3)
    } else {
        (pixels, channels)
    };
    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::from(width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, if channels == 3 { 2 } else { 6 }, 0, 0, 0]);
    chunk(&mut output, b"IHDR", &header);
    let sink = Idats {
        output,
        buffer: Vec::with_capacity(32768),
    };
    let mut zlib = d2_rs_goflate::zlib::Writer::with_level(sink, d2_rs_goflate::flate::BEST_SPEED)
        .expect("valid compression level");
    filter_rows(pixels, width as usize * channels, channels, |row| {
        zlib.write(row).expect("writing to a Vec cannot fail");
    });
    zlib.close().expect("writing to a Vec cannot fail");
    let mut sink = zlib.into_inner();
    sink.flush().expect("writing to a Vec cannot fail");
    chunk(&mut sink.output, b"IEND", &[]);
    Ok(sink.output)
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
    use super::*;
    use base64::Engine;

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn standalone_png_exif_matches_complete_go_files() {
        // D2 lib/png.AddExif, including TIFF inline/offset boundaries and
        // byte lengths for Unicode models. Both RGB and RGBA PNGs stay intact.
        let cases: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/png-exif-source-v090.json"))
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
        let cases: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/png-encoding-go127.json"))
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
        let cases: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/png-encoding-go127.json"))
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
