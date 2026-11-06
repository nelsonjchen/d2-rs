// SPDX-License-Identifier: MPL-2.0
//! D2 internal/rasterimage/firstframe.go inspectPNG: the complete PNG/APNG
//! container pass rasterimage.Config runs for imageasset's raster probe.
//! An animated PNG is admitted by its first frame; nothing is decoded.

fn be(data: &[u8]) -> u32 {
    u32::from_be_bytes(data[..4].try_into().unwrap())
}

/// IEEE CRC-32, as hash/crc32.ChecksumIEEE.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn chunk<'a>(data: &'a [u8], at: &mut usize) -> Result<(&'a [u8], &'a [u8], usize), String> {
    if data.len().saturating_sub(*at) < 12 {
        return Err("truncated PNG chunk".into());
    }
    let size = be(&data[*at..]) as usize;
    if size > data.len() - *at - 12 {
        return Err("truncated PNG chunk data".into());
    }
    let raw = &data[*at..*at + size + 12];
    *at += size + 12;
    if crc32(&raw[4..size + 8]) != be(&raw[size + 8..]) {
        return Err(format!(
            "invalid PNG {} checksum",
            String::from_utf8_lossy(&raw[4..8])
        ));
    }
    Ok((&raw[4..8], &raw[8..size + 8], raw.len()))
}

fn sequence(payload: &[u8], next: &mut u64) -> Result<(), String> {
    let got = u64::from(be(payload));
    if got != *next {
        return Err(format!("APNG sequence number is {got}, expected {}", *next));
    }
    *next += 1;
    Ok(())
}

/// inspectPNG's validation; returns whether the PNG is animated.
pub fn inspect_png_container(data: &[u8]) -> Result<bool, String> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("invalid PNG signature".into());
    }
    let mut canvas = (0_u32, 0_u32);
    let (mut animated, mut uses_idat, mut frame_bytes) = (false, false, 0_usize);
    let (mut ihdr, mut idat, mut actl, mut current, mut current_idat, mut has_data) =
        (false, false, false, false, false, false);
    let (mut declared, mut frames, mut next, mut at) = (0_u32, 0_u64, 0_u64, 8_usize);
    loop {
        let start = at;
        let (kind, payload, _) = chunk(data, &mut at)?;
        match kind {
            b"IHDR" => {
                if ihdr || start != 8 || payload.len() != 13 {
                    return Err("invalid PNG IHDR".into());
                }
                ihdr = true;
                canvas = (be(payload), be(&payload[4..]));
                if canvas.0 == 0 || canvas.1 == 0 {
                    return Err("invalid PNG canvas dimensions".into());
                }
            }
            b"acTL" => {
                if !ihdr || actl || idat || payload.len() != 8 || be(payload) == 0 {
                    return Err("invalid APNG animation control chunk".into());
                }
                actl = true;
                animated = true;
                declared = be(payload);
                if frames > u64::from(declared) {
                    return Err("APNG frame count exceeds acTL declaration".into());
                }
            }
            b"fcTL" => {
                if !ihdr || payload.len() != 26 || (!actl && (idat || frames != 0)) {
                    return Err("invalid APNG frame control chunk".into());
                }
                if current && !has_data {
                    return Err("APNG frame has no data".into());
                }
                sequence(payload, &mut next)?;
                frames += 1;
                if actl && frames > u64::from(declared) {
                    return Err("APNG frame count exceeds acTL declaration".into());
                }
                let (w, h, x, y) = (
                    be(&payload[4..]),
                    be(&payload[8..]),
                    be(&payload[12..]),
                    be(&payload[16..]),
                );
                if w == 0
                    || h == 0
                    || u64::from(x) + u64::from(w) > u64::from(canvas.0)
                    || u64::from(y) + u64::from(h) > u64::from(canvas.1)
                {
                    return Err("APNG frame falls outside its logical canvas".into());
                }
                if payload[24] > 2 || payload[25] > 1 {
                    return Err("APNG frame has invalid dispose or blend operation".into());
                }
                current = true;
                has_data = false;
                current_idat = frames == 1 && !idat;
                if frames == 1 {
                    uses_idat = current_idat;
                    if current_idat && (x != 0 || y != 0 || (w, h) != canvas) {
                        return Err(
                            "APNG IDAT first frame does not cover its logical canvas".into()
                        );
                    }
                }
            }
            b"IDAT" => {
                if !ihdr {
                    return Err("PNG IDAT precedes IHDR".into());
                }
                if current && !actl {
                    return Err("APNG IDAT data precedes animation control".into());
                }
                if current && !current_idat {
                    return Err("APNG IDAT data follows a non-IDAT frame control".into());
                }
                idat = true;
                if current_idat && !payload.is_empty() {
                    has_data = true;
                }
            }
            b"fdAT" => {
                if !actl || !current {
                    return Err("APNG frame data precedes frame control".into());
                }
                if payload.len() < 4 || current_idat {
                    return Err("invalid APNG frame data chunk".into());
                }
                sequence(payload, &mut next)?;
                if payload.len() > 4 {
                    has_data = true;
                }
                if frames == 1 {
                    frame_bytes = frame_bytes
                        .checked_add(payload.len() - 4)
                        .ok_or("APNG first-frame data bytes overflow")?;
                }
            }
            b"IEND" => {
                if !payload.is_empty() {
                    return Err("invalid PNG IEND".into());
                }
                if !ihdr {
                    return Err("PNG is missing IHDR".into());
                }
                if frames != 0 && !actl {
                    return Err("APNG is missing an animation control chunk".into());
                }
                if !animated {
                    return Ok(false);
                }
                if frames == 0 {
                    return Err("APNG is missing a first frame control chunk".into());
                }
                if !has_data {
                    return Err("APNG frame has no data".into());
                }
                if frames != u64::from(declared) {
                    return Err(format!(
                        "APNG contains {frames} frames, acTL declares {declared}"
                    ));
                }
                if !uses_idat && frame_bytes == 0 {
                    return Err("APNG first frame has no data".into());
                }
                return Ok(true);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
        output.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        let start = output.len();
        output.extend_from_slice(kind);
        output.extend_from_slice(payload);
        let crc = crc32(&output[start..]);
        output.extend_from_slice(&crc.to_be_bytes());
    }

    fn fctl(sequence: u32, w: u32, h: u32, x: u32, y: u32) -> Vec<u8> {
        let mut payload = sequence.to_be_bytes().to_vec();
        for value in [w, h, x, y] {
            payload.extend_from_slice(&value.to_be_bytes());
        }
        payload.extend_from_slice(&[0, 1, 0, 1, 0, 0]);
        payload
    }

    #[test]
    fn apng_containers_are_admitted_with_firstframe_go_errors() {
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&4_u32.to_be_bytes());
        ihdr.extend_from_slice(&3_u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        let build = |frame_x: u32| {
            let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
            png_chunk(&mut data, b"IHDR", &ihdr);
            png_chunk(&mut data, b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]);
            png_chunk(&mut data, b"fcTL", &fctl(0, 4, 3, frame_x, 0));
            png_chunk(&mut data, b"IDAT", &[1]);
            png_chunk(&mut data, b"IEND", &[]);
            data
        };
        assert_eq!(inspect_png_container(&build(0)), Ok(true));
        assert_eq!(
            inspect_png_container(&build(1)).unwrap_err(),
            "APNG frame falls outside its logical canvas"
        );
        let mut still = b"\x89PNG\r\n\x1a\n".to_vec();
        png_chunk(&mut still, b"IHDR", &ihdr);
        png_chunk(&mut still, b"IDAT", &[1]);
        png_chunk(&mut still, b"IEND", &[]);
        assert_eq!(inspect_png_container(&still), Ok(false));
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }
}
