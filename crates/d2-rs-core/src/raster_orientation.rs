// SPDX-License-Identifier: MPL-2.0
// Source: D2 internal/rasterimage/jpeg_orientation.go.
use crate::{CancellationToken, LayoutError};
fn context(c: Option<&CancellationToken>) -> Result<(), LayoutError> {
    match c.and_then(CancellationToken::error) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
/// Reads optional pre-scan EXIF orientation. Invalid metadata falls back to 1.
pub fn jpeg(d: &[u8], c: Option<&CancellationToken>) -> Result<u8, LayoutError> {
    jpeg_with_poll(d, &|| context(c))
}
/// Source optional metadata with the caller's functional cancellation provider.
pub fn jpeg_with_poll(
    d: &[u8],
    poll: &dyn Fn() -> Result<(), LayoutError>,
) -> Result<u8, LayoutError> {
    if d.len() < 2 || d[..2] != [255, 216] {
        return Ok(1);
    }
    let mut o = 2;
    while o < d.len() {
        poll()?;
        if d[o] != 255 {
            return Ok(1);
        }
        while o < d.len() && d[o] == 255 {
            o += 1;
        }
        if o >= d.len() {
            return Ok(1);
        }
        let m = d[o];
        o += 1;
        if m == 218 || m == 217 {
            return Ok(1);
        }
        if m == 216 || m == 1 || (208..=215).contains(&m) {
            continue;
        }
        if d.len() - o < 2 {
            return Ok(1);
        }
        let n = u16::from_be_bytes(d[o..o + 2].try_into().unwrap()) as usize;
        if n < 2 || n > d.len() - o {
            return Ok(1);
        }
        if m == 225 {
            if let Some(v) = exif(&d[o + 2..o + n], poll) {
                return Ok(v);
            }
            poll()?;
        }
        o += n;
    }
    Ok(1)
}
fn exif(p: &[u8], poll: &dyn Fn() -> Result<(), LayoutError>) -> Option<u8> {
    if p.len() < 14 || &p[..6] != b"Exif\0\0" {
        return None;
    }
    let t = &p[6..];
    let little = match &t[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |b: &[u8]| {
        if little {
            u16::from_le_bytes(b.try_into().unwrap())
        } else {
            u16::from_be_bytes(b.try_into().unwrap())
        }
    };
    let u32_at = |b: &[u8]| {
        if little {
            u32::from_le_bytes(b.try_into().unwrap())
        } else {
            u32::from_be_bytes(b.try_into().unwrap())
        }
    };
    if u16_at(&t[2..4]) != 42 {
        return None;
    }
    let o = u32_at(&t[4..8]) as u64;
    if o > t.len() as u64 || t.len() as u64 - o < 2 {
        return None;
    }
    let ifd = &t[o as usize..];
    let n = u16_at(&ifd[..2]) as usize;
    if n > (ifd.len() - 2) / 12 {
        return None;
    }
    for (i, e) in ifd[2..2 + n * 12].as_chunks::<12>().0.iter().enumerate() {
        if i & 255 == 0 && poll().is_err() {
            return None;
        }
        if u16_at(&e[..2]) != 0x112 {
            continue;
        }
        if u16_at(&e[2..4]) != 3 || u32_at(&e[4..8]) != 1 {
            return None;
        }
        let v = u16_at(&e[8..10]);
        return (1..=8).contains(&v).then_some(v as u8);
    }
    None
}
/// Orientation values 5 through 8 transpose the logical canvas.
pub fn dimensions(width: i64, height: i64, orientation: u8) -> [i64; 2] {
    if (5..=8).contains(&orientation) {
        [height, width]
    } else {
        [width, height]
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Row {
        data: Vec<u8>,
        cancel: String,
        orientation: u8,
        error: String,
        dimensions: [i64; 2],
    }
    #[test]
    fn original_jpeg_optional_metadata_and_context_contract() {
        let rows: Vec<Row> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/jpeg-orientation-v090.json"
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
            let result = jpeg(&row.data, c);
            let error = match result.as_ref().err() {
                Some(LayoutError::DeadlineExceeded) => "context deadline exceeded",
                Some(_) => "context canceled",
                None => "",
            };
            assert_eq!(error, row.error, "control {index}");
            if let Ok(value) = result {
                assert_eq!(value, row.orientation, "control {index}");
                assert_eq!(dimensions(2, 3, value), row.dimensions, "control {index}");
            }
        }
    }
}
