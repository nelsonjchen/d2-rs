// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Copyright 2011 The Go Authors. All rights reserved.
// Go Authors BSD terms are preserved in THIRD_PARTY_NOTICES.md.
// Source: D2 internal/rasterimage/firstframe.go and pinned Go PNG reader.
use d2_rs_core::{CancellationToken, LayoutError};
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Config {
    pub width: i64,
    pub height: i64,
    pub decoded_bytes: i64,
}
fn context(c: Option<&CancellationToken>) -> Result<(), String> {
    match c.and_then(CancellationToken::error) {
        Some(LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
fn crc(bytes: &[u8], c: Option<&CancellationToken>) -> Result<u32, String> {
    let mut v = 0xffffffffu32;
    for part in bytes.chunks(32768) {
        context(c)?;
        for &b in part {
            v ^= b as u32;
            for _ in 0..8 {
                v = (v >> 1) ^ if v & 1 != 0 { 0xedb88320 } else { 0 };
            }
        }
    }
    context(c)?;
    Ok(!v)
}
fn bad(s: &str) -> String {
    format!("png: invalid format: {s}")
}
/// Complete static importer PNG structure and configuration admission.
/// This does not decode compressed pixel data.
pub fn png_static(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    crate::svg_image_static::inspect(d, "png", c)?;
    png_admitted(d, c)
}
/// image.DecodeConfig for a PNG whose container (including any APNG
/// animation) the caller has already validated.
pub fn png_decode_config(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    png_admitted(d, c)
}
fn png_admitted(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    context(c)?;
    if d.len() < 8 || &d[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("invalid PNG signature".into());
    }
    let mut o = 8;
    let mut ihdr = false;
    loop {
        context(c)?;
        if d.len() - o < 12 {
            return Err("truncated PNG chunk".into());
        }
        let n = u32::from_be_bytes(d[o..o + 4].try_into().unwrap()) as usize;
        if n as u64 + 12 > (d.len() - o) as u64 {
            return Err("truncated PNG chunk data".into());
        }
        let t = &d[o + 4..o + 8];
        let b = &d[o + 8..o + 8 + n];
        let expected = u32::from_be_bytes(d[o + 8 + n..o + 12 + n].try_into().unwrap());
        if crc(&d[o + 4..o + 8 + n], c)? != expected {
            return Err(format!(
                "invalid PNG {} checksum",
                String::from_utf8_lossy(t)
            ));
        }
        match t {
            b"IHDR" => {
                if ihdr || o != 8 || n != 13 {
                    return Err("invalid PNG IHDR".into());
                }
                ihdr = true;
                if b[..4] == [0; 4] || b[4..8] == [0; 4] {
                    return Err("invalid PNG canvas dimensions".into());
                }
            }
            b"IDAT" => {
                if !ihdr {
                    return Err("PNG IDAT precedes IHDR".into());
                }
            }
            b"IEND" => {
                if n != 0 {
                    return Err("invalid PNG IEND".into());
                }
                if !ihdr {
                    return Err("PNG is missing IHDR".into());
                }
                break;
            }
            _ => {}
        }
        o += n + 12;
    }
    // Original DecodeConfig returns after IHDR for nonpaletted images and after
    // tRNS/IDAT for paletted images. Prior container validation is independent.
    let b = &d[16..29];
    if b[10] != 0 {
        return Err("png: unsupported feature: compression method".into());
    }
    if b[11] != 0 {
        return Err("png: unsupported feature: filter method".into());
    }
    if b[12] > 1 {
        return Err(bad("invalid interlace method"));
    }
    let w = i32::from_be_bytes(b[..4].try_into().unwrap()) as i64;
    let h = i32::from_be_bytes(b[4..8].try_into().unwrap()) as i64;
    if w <= 0 || h <= 0 {
        return Err(bad("non-positive dimension"));
    }
    if w.checked_mul(h).and_then(|v| v.checked_mul(8)).is_none() {
        return Err("png: unsupported feature: dimension overflow".into());
    }
    let depth = b[8];
    let color = b[9];
    let valid = match depth {
        1 | 2 | 4 => matches!(color, 0 | 3),
        8 => matches!(color, 0 | 2 | 3 | 4 | 6),
        16 => matches!(color, 0 | 2 | 4 | 6),
        _ => false,
    };
    if !valid {
        return Err(format!(
            "png: unsupported feature: bit depth {depth}, color type {color}"
        ));
    }
    if color == 3 {
        let mut stage = 1;
        o = 33;
        loop {
            let n = u32::from_be_bytes(d[o..o + 4].try_into().unwrap()) as usize;
            let t = &d[o + 4..o + 8];
            match t {
                b"IHDR" => return Err(bad("chunk out of order")),
                b"PLTE" => {
                    if stage != 1 {
                        return Err(bad("chunk out of order"));
                    }
                    stage = 2;
                    let entries = n / 3;
                    if !n.is_multiple_of(3)
                        || entries == 0
                        || entries > 256
                        || entries > (1usize << depth)
                    {
                        return Err(bad("bad PLTE length"));
                    }
                }
                b"tRNS" => {
                    if stage != 2 {
                        return Err(bad("chunk out of order"));
                    }
                    if n > 256 {
                        return Err(bad("bad tRNS length"));
                    }
                    break;
                }
                b"IDAT" => {
                    if stage == 1 {
                        return Err(bad("chunk out of order"));
                    }
                    break;
                }
                b"IEND" => return Err(bad("chunk out of order")),
                _ => {
                    if n > 0x7fffffff {
                        return Err(bad(&format!("Bad chunk length: {n}")));
                    }
                }
            }
            o += n + 12;
        }
    }
    Ok(Config {
        width: w,
        height: h,
        decoded_bytes: w * h * if depth == 16 { 8 } else { 4 },
    })
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Row {
        data: Vec<u8>,
        cancel: String,
        error: String,
        output: Option<Config>,
    }
    #[test]
    fn original_static_png_config_and_decoded_charge() {
        let rows: Vec<Row> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/png-config-v090.json"
        ))
        .unwrap();
        for (i, r) in rows.iter().enumerate() {
            let token = CancellationToken::new();
            let c = match r.cancel.as_str() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let result = png_static(&r.data, c);
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                r.error,
                "control {i}"
            );
            if let Ok(cfg) = result {
                assert_eq!(Some(cfg), r.output, "control {i}");
            }
        }
    }
}

/// Generic raster GIF configuration: animations are admitted and fully scanned.
/// Source: internal/rasterimage/firstframe.go inspectGIF, then gif.DecodeConfig.
pub fn gif_config(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    if d.len() < 13 {
        return Err("truncated logical screen descriptor".into());
    }
    let mut o = 13;
    if d[10] & 128 != 0 {
        o += 3 * (1usize << ((d[10] & 7) + 1));
    }
    if o > d.len() {
        return Err("truncated global color table".into());
    }
    let mut frames = 0;
    loop {
        context(c)?;
        let marker = *d.get(o).ok_or("missing GIF trailer")?;
        o += 1;
        match marker {
            59 => {
                if frames == 0 {
                    return Err("GIF has no image frame".into());
                }
                break;
            }
            33 => {
                if o >= d.len() {
                    return Err("truncated GIF extension".into());
                }
                o += 1;
            }
            44 => {
                frames += 1;
                if d.len() - o < 9 {
                    return Err("truncated GIF image descriptor".into());
                }
                let packed = d[o + 8];
                o += 9;
                if packed & 128 != 0 {
                    o += 3 * (1usize << ((packed & 7) + 1));
                }
                if o > d.len() {
                    return Err("truncated local color table".into());
                }
                if o == d.len() {
                    return Err("missing GIF LZW code size".into());
                }
                o += 1;
            }
            _ => return Err(format!("unexpected GIF block marker 0x{marker:02x}")),
        }
        loop {
            context(c)?;
            let n = *d.get(o).ok_or("truncated GIF sub-blocks")? as usize;
            o += 1;
            if n == 0 {
                break;
            }
            if n > d.len() - o {
                return Err("truncated GIF sub-block data".into());
            }
            o += n;
        }
    }
    context(c)?;
    if &d[..6] != b"GIF87a" && &d[..6] != b"GIF89a" {
        return Err(format!(
            "gif: can't recognize format {}",
            d2_rs_core::quote_go_bytes(&d[..6])
        ));
    }
    let width = u16::from_le_bytes(d[6..8].try_into().unwrap()) as i64;
    let height = u16::from_le_bytes(d[8..10].try_into().unwrap()) as i64;
    if width == 0 || height == 0 {
        return Err(format!("invalid decoded dimensions {width}x{height}"));
    }
    Ok(Config {
        width,
        height,
        decoded_bytes: width * height * 4,
    })
}

/// Complete static importer GIF structure and logical-canvas admission.
/// Frame pixels and palette values are not decoded by this configuration API.
pub fn gif_static(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    crate::svg_image_static::inspect(d, "gif", c)?;
    gif_admitted(d, c)
}
fn gif_admitted(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    context(c)?;
    // Config performs its own complete structural scan before DecodeConfig.
    // On this statically admitted single-frame domain its traversal and
    // checkpoints are the same as the preceding static inspector.
    crate::svg_image_static::inspect(d, "gif", c)?;
    // DecodeConfig's context reader checks before reading the buffered header.
    context(c)?;
    if &d[..6] != b"GIF87a" && &d[..6] != b"GIF89a" {
        return Err(format!(
            "gif: can't recognize format {}",
            d2_rs_core::quote_go_bytes(&d[..6])
        ));
    }
    let w = u16::from_le_bytes(d[6..8].try_into().unwrap()) as i64;
    let h = u16::from_le_bytes(d[8..10].try_into().unwrap()) as i64;
    if w == 0 || h == 0 {
        return Err(format!("invalid decoded dimensions {w}x{h}"));
    }
    Ok(Config {
        width: w,
        height: h,
        decoded_bytes: w * h * 4,
    })
}

#[cfg(test)]
mod gif_tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Row {
        data: Vec<u8>,
        cancel: String,
        error: String,
        output: Option<Config>,
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_static_gif_config_logical_canvas_and_charge() {
        let rows: Vec<Row> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/gif-config-v090.json"
        ))
        .unwrap();
        for (i, r) in rows.iter().enumerate() {
            let token = CancellationToken::new();
            let c = match r.cancel.as_str() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let result = gif_static(&r.data, c);
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                r.error,
                "control {i}"
            );
            if let Ok(cfg) = result {
                assert_eq!(Some(cfg), r.output, "control {i}");
            }
        }
    }
}

fn jbad(s: &str) -> String {
    format!("invalid JPEG format: {s}")
}
fn junsupported(s: &str) -> String {
    format!("unsupported JPEG feature: {s}")
}
fn jpeg_sof(b: &[u8]) -> Result<(i64, i64), String> {
    let count = match b.len() {
        9 => 1,
        15 => 3,
        18 => 4,
        _ => return Err(junsupported("number of components")),
    };
    if b[0] != 8 {
        return Err(junsupported("precision"));
    }
    let h = u16::from_be_bytes(b[1..3].try_into().unwrap()) as i64;
    let w = u16::from_be_bytes(b[3..5].try_into().unwrap()) as i64;
    if b[5] as usize != count {
        return Err(jbad("SOF has wrong length"));
    }
    let mut ids = Vec::new();
    let mut factors = Vec::new();
    for (i, e) in b[6..].as_chunks::<3>().0.iter().enumerate() {
        if ids.contains(&e[0]) {
            return Err(jbad("repeated component identifier"));
        }
        ids.push(e[0]);
        if e[2] > 3 {
            return Err(jbad("bad Tq value"));
        }
        let mut x = e[1] >> 4;
        let mut y = e[1] & 15;
        if !(1..=4).contains(&x) || !(1..=4).contains(&y) {
            return Err(jbad("luma/chroma subsampling ratio"));
        }
        if x == 3 || y == 3 {
            return Err(junsupported("luma/chroma subsampling ratio"));
        }
        if count == 1 {
            x = 1;
            y = 1;
        }
        if count == 4 {
            let valid = match i {
                0 => e[1] == 17 || e[1] == 34,
                1 | 2 => e[1] == 17,
                _ => factors[0] == (x, y),
            };
            if !valid {
                return Err(junsupported("luma/chroma subsampling ratio"));
            }
        }
        factors.push((x, y));
    }
    if count == 3 {
        let mx = factors.iter().map(|v| v.0).max().unwrap();
        let my = factors.iter().map(|v| v.1).max().unwrap();
        if factors.iter().any(|v| mx % v.0 != 0 || my % v.1 != 0) {
            return Err(junsupported("luma/chroma subsampling ratio"));
        }
    }
    Ok((w, h))
}
/// Static JPEG container, pre-scan header and oriented logical dimensions.
pub fn jpeg_static(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    crate::svg_image_static::inspect(d, "jpeg", c)?;
    jpeg_admitted(d, c)
}
/// D2 rasterimage.Config JPEG branch: EXIF plus image.DecodeConfig, without
/// the distinct SVG importer static-container inspection phase.
pub fn jpeg_config(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    jpeg_admitted(d, c)
}
fn jpeg_admitted(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    context(c)?;
    let orientation = d2_rs_core::raster_orientation::jpeg(d, c).map_err(|e| -> String {
        match e {
            LayoutError::DeadlineExceeded => "context deadline exceeded".into(),
            _ => "context canceled".into(),
        }
    })?;
    let mut filled = 0usize;
    let mut read = |offset: usize, length: usize| -> Result<(), String> {
        let end = offset.checked_add(length).ok_or("unexpected EOF")?;
        while end > filled {
            context(c)?;
            if filled == d.len() {
                return Err("unexpected EOF".into());
            }
            filled = d.len().min(filled + if filled == 0 { 4096 } else { 4094 });
        }
        Ok(())
    };
    read(0, 2)?;
    let mut o = 2;
    let mut jfif = false;
    let mut dimensions = None;
    loop {
        read(o, 2)?;
        let mut first = d[o];
        let mut m = d[o + 1];
        o += 2;
        while first != 255 {
            first = m;
            read(o, 1)?;
            m = d[o];
            o += 1;
        }
        if m == 0 {
            continue;
        }
        while m == 255 {
            read(o, 1)?;
            m = d[o];
            o += 1;
        }
        if m == 217 {
            return Err(jbad("missing SOS marker"));
        }
        if (208..=215).contains(&m) {
            continue;
        }
        read(o, 2)?;
        let n = u16::from_be_bytes(d[o..o + 2].try_into().unwrap()) as usize;
        o += 2;
        if n < 2 {
            return Err(jbad("short segment length"));
        }
        let n = n - 2;
        // SOF rejects an unsupported segment length before reading its body.
        if matches!(m, 192..=194) {
            if dimensions.is_some() {
                return Err(jbad("multiple SOF markers"));
            }
            if !matches!(n, 9 | 15 | 18) {
                return Err(junsupported("number of components"));
            }
        }
        if m == 218 {
            break;
        }
        if !matches!(m, 192 | 193 | 194 | 196 | 219 | 221 | 224..=239 | 254) {
            return Err(if m < 192 {
                jbad("unknown marker")
            } else {
                junsupported("unknown marker")
            });
        }
        read(o, n)?;
        let body = &d[o..o + n];
        match m {
            192..=194 => {
                dimensions = Some(jpeg_sof(body)?);
                if jfif {
                    break;
                }
            }
            224 if n >= 5 => {
                jfif = &body[..5] == b"JFIF\0";
            }
            _ => {}
        }
        o += n;
    }
    let (w, h) = dimensions.ok_or_else(|| jbad("missing SOF marker"))?;
    let [w, h] = d2_rs_core::raster_orientation::dimensions(w, h, orientation);
    if w == 0 || h == 0 {
        return Err(format!("invalid decoded dimensions {w}x{h}"));
    }
    Ok(Config {
        width: w,
        height: h,
        decoded_bytes: w * h * 4,
    })
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod jpeg_tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Row {
        data: Vec<u8>,
        cancel: String,
        error: String,
        output: Option<Config>,
    }
    #[test]
    fn original_static_jpeg_config_markers_sampling_and_orientation() {
        let rows: Vec<Row> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/jpeg-config-v090.json"
        ))
        .unwrap();
        for (i, r) in rows.iter().enumerate() {
            let token = CancellationToken::new();
            let c = match r.cancel.as_str() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let result = jpeg_static(&r.data, c);
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                r.error,
                "control {i}"
            );
            if let Ok(cfg) = result {
                assert_eq!(Some(cfg), r.output, "control {i}");
            }
        }
    }
}

fn u24(b: &[u8]) -> i64 {
    b[0] as i64 | ((b[1] as i64) << 8) | ((b[2] as i64) << 16)
}
/// internal/rasterimage Config for "webp", imageasset's raster probe: an
/// animated container reports its logical canvas (as NRGBA) after its first
/// frame is validated; a static one passes image.DecodeConfig.
pub fn webp_config(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    context(c)?;
    let plan = crate::webp_first_frame::inspect(d, false, c)?;
    let Some((width, height)) = plan.canvas.filter(|_| plan.animated) else {
        return webp_admitted(d, c);
    };
    let (width, height) = (width as i64, height as i64);
    Ok(Config {
        width,
        height,
        decoded_bytes: width * height * 4,
    })
}

/// Static WebP container and header/canvas configuration. No pixel decoding.
pub fn webp_static(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    crate::svg_image_static::inspect(d, "webp", c)?;
    webp_admitted(d, c)
}
fn webp_admitted(d: &[u8], c: Option<&CancellationToken>) -> Result<Config, String> {
    context(c)?;
    if &d[..4] != b"RIFF" || &d[8..12] != b"WEBP" {
        return Err("invalid WebP RIFF header".into());
    }
    // Config's complete container pass rejects repeated or malformed VP8X.
    let mut o = 12;
    let mut extended = false;
    while o < d.len() {
        context(c)?;
        let n = u32::from_le_bytes(d[o + 4..o + 8].try_into().unwrap()) as usize;
        let t = &d[o..o + 4];
        if t == b"VP8X" {
            if extended || n != 10 {
                return Err("invalid WebP VP8X chunk".into());
            }
            extended = true;
        }
        o += 8 + n + n % 2;
    }
    // image.DecodeConfig recognizes the registered RIFF????WEBPVP8 magic.
    context(c)?;
    if &d[12..15] != b"VP8" {
        return Err("image: unknown format".into());
    }
    o = 12;
    let (w, h) = loop {
        if o == d.len() {
            return Err("webp: invalid format".into());
        }
        context(c)?;
        let n = u32::from_le_bytes(d[o + 4..o + 8].try_into().unwrap()) as usize;
        let t = &d[o..o + 4];
        let b = &d[o + 8..o + 8 + n];
        match t {
            b"VP8X" => {
                let w = u24(&b[4..7]) + 1;
                let h = u24(&b[7..10]) + 1;
                if w * h > 0x7fffffff {
                    return Err("webp: invalid format".into());
                }
                break (w, h);
            }
            b"ALPH" => return Err("webp: invalid format".into()),
            b"VP8 " => {
                if n < 3 {
                    return Err("unexpected EOF".into());
                }
                if b[0] & 1 != 0 {
                    break (0, 0);
                }
                if n < 10 {
                    return Err("unexpected EOF".into());
                }
                if b[3..6] != [157, 1, 42] {
                    return Err("vp8: invalid format".into());
                }
                break (
                    (b[6] as i64) | (((b[7] & 63) as i64) << 8),
                    (b[8] as i64) | (((b[9] & 63) as i64) << 8),
                );
            }
            b"VP8L" => {
                if n == 0 {
                    return Err("unexpected EOF".into());
                }
                if b[0] != 47 {
                    return Err("vp8l: invalid header".into());
                }
                if n < 5 {
                    return Err("unexpected EOF".into());
                }
                let bits = u32::from_le_bytes(b[1..5].try_into().unwrap());
                if bits >> 29 != 0 {
                    return Err("vp8l: invalid version".into());
                }
                break ((bits & 16383) as i64 + 1, ((bits >> 14) & 16383) as i64 + 1);
            }
            _ => {}
        }
        o += 8 + n + n % 2;
    };
    if w <= 0 || h <= 0 {
        return Err(format!("invalid decoded dimensions {w}x{h}"));
    }
    Ok(Config {
        width: w,
        height: h,
        decoded_bytes: w * h * 4,
    })
}

/// Full static configuration admission for the four embedded raster formats.
pub fn static_config(
    d: &[u8],
    format: &str,
    c: Option<&CancellationToken>,
) -> Result<Config, String> {
    match format {
        "png" => png_static(d, c),
        "jpeg" => jpeg_static(d, c),
        "gif" => gif_static(d, c),
        "webp" => webp_static(d, c),
        _ => Err(format!(
            "unsupported raster format {}",
            d2_rs_core::quote_go_bytes(format.as_bytes())
        )),
    }
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod webp_tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Row {
        data: Vec<u8>,
        cancel: String,
        error: String,
        output: Option<Config>,
    }
    #[test]
    fn original_static_webp_config_bitstreams_and_canvas() {
        let rows: Vec<Row> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/webp-config-v090.json"
        ))
        .unwrap();
        for (i, r) in rows.iter().enumerate() {
            let token = CancellationToken::new();
            let c = match r.cancel.as_str() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let result = static_config(&r.data, "webp", c);
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                r.error,
                "control {i}"
            );
            if let Ok(cfg) = result {
                assert_eq!(Some(cfg), r.output, "control {i}");
            }
        }
    }
}

/// Config phase after complete static-format admission; internal resolver only.
pub(crate) fn admitted(
    d: &[u8],
    format: &str,
    c: Option<&CancellationToken>,
) -> Result<Config, String> {
    match format {
        "png" => png_admitted(d, c),
        "jpeg" => jpeg_admitted(d, c),
        "gif" => gif_admitted(d, c),
        "webp" => webp_admitted(d, c),
        _ => unreachable!("format admitted by embedded URI/signature"),
    }
}
