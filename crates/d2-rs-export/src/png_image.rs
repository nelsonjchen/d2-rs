// SPDX-License-Identifier: MPL-2.0
//! Source d2raster/image.go PNG decoding, coverage and premultiplied sampling.

fn crc32(bytes: impl Iterator<Item = u8>) -> u32 {
    let mut crc = 0xffffffffu32;
    for byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    !crc
}

/// Adapt Go's legal empty-dictionary zlib header for PNG backends that reject
/// FDICT. The DEFLATE payload and pixels are unchanged; encoded source bytes
/// and their asset identity remain untouched outside the decoder.
pub(crate) fn empty_dictionary_decoder_bytes(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    use std::borrow::Cow;
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Cow::Borrowed(data);
    }
    // Most assets have no dictionary. Probe at most six zlib header bytes
    // across IDAT chunks before allocating an adapted encoded image.
    let mut header = [0u8; 6];
    let mut filled = 0;
    let mut probe = 8usize;
    while probe + 12 <= data.len() && filled < 6 {
        let length = u32::from_be_bytes(data[probe..probe + 4].try_into().unwrap()) as usize;
        let Some(end) = probe
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .filter(|&v| v <= data.len())
        else {
            return Cow::Borrowed(data);
        };
        if &data[probe + 4..probe + 8] == b"IDAT" {
            for &byte in data[probe + 8..probe + 8 + length].iter().take(6 - filled) {
                header[filled] = byte;
                filled += 1;
            }
            if filled >= 2 && header[1] & 0x20 == 0 {
                return Cow::Borrowed(data);
            }
        }
        probe = end;
    }
    if filled < 6 || header[2..6] != [0, 0, 0, 1] {
        return Cow::Borrowed(data);
    }
    let mut chunks = Vec::new();
    let mut entropy = Vec::new();
    let mut offset = 8usize;
    while offset + 12 <= data.len() {
        let length = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        let Some(end) = offset
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .filter(|&v| v <= data.len())
        else {
            return Cow::Borrowed(data);
        };
        let kind = &data[offset + 4..offset + 8];
        // Never make a corrupt source chunk valid by regenerating its CRC.
        if crc32(data[offset + 4..end - 4].iter().copied())
            != u32::from_be_bytes(data[end - 4..end].try_into().unwrap())
        {
            return Cow::Borrowed(data);
        }
        if kind == b"IDAT" {
            entropy.extend_from_slice(&data[offset + 8..offset + 8 + length]);
        }
        chunks.push((offset, end, kind == b"IDAT"));
        offset = end;
    }
    if offset != data.len()
        || entropy.len() < 6
        || entropy[0] & 15 != 8
        || entropy[0] >> 4 > 7
        || (u16::from(entropy[0]) * 256 + u16::from(entropy[1])) % 31 != 0
        || entropy[1] & 0x20 == 0
        || entropy[2..6] != [0, 0, 0, 1]
    {
        return Cow::Borrowed(data);
    }
    let flags = entropy[1] & 0xc0;
    let check = (31 - (u16::from(entropy[0]) * 256 + u16::from(flags)) % 31) % 31;
    let mut payload = vec![entropy[0], flags | check as u8];
    payload.extend_from_slice(&entropy[6..]);
    let mut out = data[..8].to_vec();
    let mut inserted = false;
    for (start, end, idat) in chunks {
        if !idat {
            out.extend_from_slice(&data[start..end]);
            continue;
        }
        if inserted {
            continue;
        }
        inserted = true;
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(b"IDAT");
        out.extend_from_slice(&payload);
        out.extend_from_slice(&crc32(b"IDAT".iter().chain(payload.iter()).copied()).to_be_bytes());
    }
    Cow::Owned(out)
}
