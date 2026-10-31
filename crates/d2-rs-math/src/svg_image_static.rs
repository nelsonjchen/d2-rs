// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/image.go static raster inspection.
use d2_rs_core::CancellationToken;
fn context(cancel: Option<&CancellationToken>) -> Result<(), String> {
    match cancel.and_then(CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub fn inspect(
    data: &[u8],
    format: &str,
    cancel: Option<&CancellationToken>,
) -> Result<(), String> {
    match format {
        "png" => png(data, cancel),
        "jpeg" => jpeg(data, cancel),
        "gif" => gif(data, cancel),
        "webp" => webp(data, cancel),
        _ => Err(format!("unsupported raster format {format:?}")),
    }
}
fn png(d: &[u8], c: Option<&CancellationToken>) -> Result<(), String> {
    let mut o = 8;
    loop {
        context(c)?;
        if d.len().saturating_sub(o) < 12 {
            return Err("truncated PNG chunk".into());
        }
        let n = u32::from_be_bytes(d[o..o + 4].try_into().unwrap()) as u64;
        if n + 12 > (d.len() - o) as u64 {
            return Err("truncated PNG chunk data".into());
        }
        let t = &d[o + 4..o + 8];
        if matches!(t, b"acTL" | b"fcTL" | b"fdAT") {
            return Err("animated PNG is unsupported".into());
        }
        o += (n + 12) as usize;
        if t == b"IEND" {
            return if n != 0 || o != d.len() {
                Err("PNG has malformed IEND or trailing data".into())
            } else {
                Ok(())
            };
        }
    }
}
fn jpeg(d: &[u8], c: Option<&CancellationToken>) -> Result<(), String> {
    if d.len() < 4 || d[..2] != [255, 216] {
        return Err("JPEG is missing its SOI marker".into());
    }
    let mut o = 2;
    let mut scan = false;
    loop {
        context(c)?;
        if scan {
            while o < d.len() {
                if o & 32767 == 0 {
                    context(c)?;
                }
                if d[o] != 255 {
                    o += 1;
                    continue;
                }
                let start = o;
                while o < d.len() && d[o] == 255 {
                    if o & 32767 == 0 {
                        context(c)?;
                    }
                    o += 1;
                }
                if o >= d.len() {
                    return Err("JPEG entropy data ends in a partial marker".into());
                }
                let marker = d[o];
                if marker == 0 || (208..=215).contains(&marker) {
                    o += 1;
                    continue;
                }
                o = start;
                scan = false;
                break;
            }
            if scan {
                return Err("JPEG is missing its EOI marker".into());
            }
        }
        if o >= d.len() || d[o] != 255 {
            return Err("JPEG has data outside an entropy-coded scan".into());
        }
        while o < d.len() && d[o] == 255 {
            if o & 32767 == 0 {
                context(c)?;
            }
            o += 1;
        }
        if o >= d.len() {
            return Err("JPEG ends in a partial marker".into());
        }
        let marker = d[o];
        o += 1;
        if marker == 217 {
            return if o != d.len() {
                Err("JPEG has trailing data after its EOI marker".into())
            } else {
                Ok(())
            };
        }
        if marker == 216 {
            return Err("JPEG contains an unexpected nested SOI marker".into());
        }
        if marker == 1 || (208..=215).contains(&marker) {
            continue;
        }
        if d.len() - o < 2 {
            return Err("JPEG has a truncated marker length".into());
        }
        let n = u16::from_be_bytes(d[o..o + 2].try_into().unwrap()) as usize;
        if n < 2 || n > d.len() - o {
            return Err("JPEG has a malformed or truncated marker segment".into());
        }
        o += n;
        if marker == 218 {
            scan = true;
        }
    }
}
fn blocks(d: &[u8], mut o: usize, c: Option<&CancellationToken>) -> Result<usize, String> {
    loop {
        context(c)?;
        if o >= d.len() {
            return Err("truncated GIF sub-blocks".into());
        }
        let n = d[o] as usize;
        o += 1;
        if n == 0 {
            return Ok(o);
        }
        if n > d.len() - o {
            return Err("truncated GIF sub-block data".into());
        }
        o += n;
    }
}
fn gif(d: &[u8], c: Option<&CancellationToken>) -> Result<(), String> {
    if d.len() < 13 {
        return Err("truncated GIF logical screen descriptor".into());
    }
    let mut o = 13;
    if d[10] & 128 != 0 {
        o += 3 * (1usize << ((d[10] & 7) + 1));
        if o > d.len() {
            return Err("truncated GIF global color table".into());
        }
    }
    let mut frames = 0;
    loop {
        context(c)?;
        if o >= d.len() {
            return Err("GIF is missing its trailer".into());
        }
        let marker = d[o];
        o += 1;
        match marker {
            59 => {
                if frames != 1 {
                    return Err(format!(
                        "animated or malformed GIF has {frames} frames; exactly one is required"
                    ));
                }
                return if o != d.len() {
                    Err("GIF has trailing data".into())
                } else {
                    Ok(())
                };
            }
            33 => {
                if o >= d.len() {
                    return Err("truncated GIF extension".into());
                }
                o += 1;
                o = blocks(d, o, c)?;
            }
            44 => {
                frames += 1;
                if frames > 1 {
                    return Err("animated GIF is unsupported".into());
                }
                if d.len() - o < 9 {
                    return Err("truncated GIF image descriptor".into());
                }
                let packed = d[o + 8];
                o += 9;
                if packed & 128 != 0 {
                    o += 3 * (1usize << ((packed & 7) + 1));
                    if o > d.len() {
                        return Err("truncated GIF local color table".into());
                    }
                }
                if o >= d.len() {
                    return Err("GIF is missing its LZW code size".into());
                }
                o += 1;
                o = blocks(d, o, c)?;
            }
            _ => return Err(format!("unexpected GIF block marker 0x{marker:02x}")),
        }
    }
}
fn webp(d: &[u8], c: Option<&CancellationToken>) -> Result<(), String> {
    if d.len() < 12 {
        return Err("truncated WebP RIFF header".into());
    }
    let end = u32::from_le_bytes(d[4..8].try_into().unwrap()) as u64 + 8;
    if end != d.len() as u64 || end < 12 {
        return Err("WebP has invalid RIFF length or trailing data".into());
    }
    let mut images = 0;
    let mut o = 12;
    while (o as u64) < end {
        context(c)?;
        if end - (o as u64) < 8 {
            return Err("truncated WebP chunk header".into());
        }
        let n = u32::from_le_bytes(d[o + 4..o + 8].try_into().unwrap()) as u64;
        let next = o as u64 + 8 + n + n % 2;
        if next > end {
            return Err("truncated WebP chunk data".into());
        }
        let t = &d[o..o + 4];
        if matches!(t, b"ANIM" | b"ANMF") || (t == b"VP8X" && n >= 1 && d[o + 8] & 2 != 0) {
            return Err("animated WebP is unsupported".into());
        }
        if matches!(t, b"VP8 " | b"VP8L") {
            images += 1;
            if images > 1 {
                return Err("WebP contains multiple static image bitstreams".into());
            }
        }
        o = next as usize;
    }
    if images != 1 {
        return Err("WebP must contain exactly one static image bitstream".into());
    }
    Ok(())
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Row {
        format: String,
        data: Vec<u8>,
        cancel: String,
        error: String,
    }
    #[test]
    fn original_static_raster_structure_errors_and_context_precedence() {
        let rows: Vec<Row> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-image-static-v090.json"
        ))
        .unwrap();
        for (index, row) in rows.iter().enumerate() {
            let token = CancellationToken::new();
            let c = match row.cancel.as_str() {
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
            assert_eq!(
                inspect(&row.data, &row.format, c).err().unwrap_or_default(),
                row.error,
                "control {index}: {} length {} mode {}",
                row.format,
                row.data.len(),
                row.cancel
            );
        }
    }
}
