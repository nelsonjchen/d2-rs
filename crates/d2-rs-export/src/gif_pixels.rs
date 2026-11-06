// SPDX-License-Identifier: MPL-2.0
//! GIF first-frame pixel admission. Source: Go image/gif reader.go and
//! compress/lzw reader.go (BSD-3-Clause); see repository third-party notices.
//! The generic raster structural scan precedes this decoder. Later animation
//! frames are deliberately not pixel-decoded, matching gif.Decode.
fn word(d: &[u8], o: usize) -> usize {
    u16::from_le_bytes([d[o], d[o + 1]]) as usize
}
pub(crate) fn validate(d: &[u8]) -> Result<(), String> {
    let mut o = 13;
    let global = if d[10] & 128 != 0 {
        1usize << ((d[10] & 7) + 1)
    } else {
        0
    };
    o += global * 3;
    let mut transparent = None;
    loop {
        let marker = d[o];
        o += 1;
        if marker == 33 {
            let label = d[o];
            o += 1;
            if label == 249 {
                if d.len() - o < 6 {
                    return Err("gif: can't read graphic control: unexpected EOF".into());
                }
                if d[o] != 4 {
                    return Err(format!(
                        "gif: invalid graphic control extension block size: {}",
                        d[o]
                    ));
                }
                if d[o + 1] & 1 != 0 {
                    transparent = Some(d[o + 4] as usize);
                }
                if d[o + 5] != 0 {
                    return Err(format!(
                        "gif: invalid graphic control extension block terminator: {}",
                        d[o + 5]
                    ));
                }
            } else if !matches!(label, 1 | 254 | 255) {
                return Err(format!("gif: unknown extension 0x{label:02x}"));
            }
            loop {
                let n = d[o] as usize;
                o += 1;
                if n == 0 {
                    break;
                }
                o += n;
            }
            continue;
        }
        if marker != 44 {
            return Err("gif: missing image data".into());
        }
        let left = word(d, o);
        let top = word(d, o + 2);
        let w = word(d, o + 4);
        let h = word(d, o + 6);
        let packed = d[o + 8];
        o += 9;
        if left + w > word(d, 6) || top + h > word(d, 8) {
            return Err("gif: frame bounds larger than image bounds".into());
        }
        let mut palette = global;
        if packed & 128 != 0 {
            palette = 1usize << ((packed & 7) + 1);
            o += palette * 3;
        }
        if palette == 0 {
            return Err("gif: no color table".into());
        }
        if let Some(t) = transparent {
            palette = palette.max(t + 1);
        }
        let bits = d[o];
        o += 1;
        if !(2..=8).contains(&bits) {
            return Err(format!("gif: pixel size in decode out of range: {bits}"));
        }
        let mut compressed = Vec::new();
        let mut ends = Vec::new();
        loop {
            let n = d[o] as usize;
            o += 1;
            if n == 0 {
                break;
            }
            compressed.extend_from_slice(&d[o..o + n]);
            ends.push(compressed.len());
            o += n;
        }
        let (pixels, consumed) = lzw(&compressed, bits, w * h)?;
        // blockReader.close permits an unconsumed tail in the current block,
        // or a single extra byte when the code ended at a block boundary.
        if let Some(i) = ends.iter().position(|&end| end >= consumed) {
            if consumed == ends[i] {
                if ends.len() > i + 2 || (ends.len() == i + 2 && ends[i + 1] - ends[i] > 1) {
                    return Err("gif: too much image data".into());
                }
            } else if ends.len() > i + 1 {
                return Err("gif: too much image data".into());
            }
        }
        if pixels.iter().any(|&p| p as usize >= palette) {
            return Err("gif: invalid pixel value".into());
        }
        return Ok(());
    }
}
fn lzw(data: &[u8], bits: u8, expected: usize) -> Result<(Vec<u8>, usize), String> {
    let clear = 1usize << bits;
    let eof = clear + 1;
    let mut width = bits as usize + 1;
    let mut hi = eof;
    let mut last = None;
    let mut prefix = [0usize; 4096];
    let mut suffix = [0u8; 4096];
    let mut bit = 0usize;
    let mut output = Vec::with_capacity(expected.min(4096));
    loop {
        if bit + width > data.len() * 8 {
            break;
        }
        let mut code = 0usize;
        for i in 0..width {
            code |= (((data[(bit + i) / 8] >> ((bit + i) % 8)) & 1) as usize) << i;
        }
        bit += width;
        if code == clear {
            width = bits as usize + 1;
            hi = eof;
            last = None;
            continue;
        }
        if code == eof {
            break;
        }
        let mut expansion = Vec::new();
        if code < clear {
            expansion.push(code as u8);
        } else if code <= hi {
            let mut c = code;
            if code == hi {
                if let Some(previous) = last {
                    c = previous;
                    while c >= clear {
                        c = prefix[c];
                    }
                    expansion.push(c as u8);
                    c = previous;
                }
            }
            while c >= clear {
                expansion.push(suffix[c]);
                c = prefix[c];
            }
            expansion.push(c as u8);
            expansion.reverse();
        } else {
            return Err("gif: reading image data: lzw: invalid code".into());
        }
        if let Some(previous) = last {
            prefix[hi] = previous;
            suffix[hi] = expansion[0];
        }
        output.extend_from_slice(&expansion);
        if output.len() > expected {
            return Err("gif: too much image data".into());
        }
        last = Some(code);
        hi += 1;
        if hi == 1usize << width {
            if width == 12 {
                last = None;
                hi -= 1;
            } else {
                width += 1;
            }
        }
    }
    if output.len() < expected {
        return Err("gif: not enough image data".into());
    }
    Ok((output, bit.div_ceil(8)))
}
#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_pixel_controls() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/gif-pixels-v090.json"
        ))
        .unwrap();
        for row in rows {
            let bytes: Vec<u8> = row["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect();
            let got = super::validate(&bytes).err();
            let want = row["error"].as_str().map(str::to_owned);
            assert_eq!(got, want, "{}", row["name"]);
        }
    }
}

#[cfg(test)]
mod upstream_valid_tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_animations_cross_lzw_width_and_table_reset_boundaries() {
        for bytes in [
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/source-xgif-v090.gif"),
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/source-animation-v090.gif"),
        ] {
            d2_rs_math::svg_image_config::gif_config(bytes, None).unwrap();
            super::validate(bytes).unwrap();
        }
    }
}
