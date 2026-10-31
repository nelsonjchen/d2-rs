// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2017 The Go Authors. All rights reserved.
// Rust translation of golang.org/x/image v0.45.0 font/sfnt/cmap.go and the
// x/text charmap.Macintosh encoder it uses.

use super::*;

const PID_UNICODE: u16 = 0;
const PID_MACINTOSH: u16 = 1;
const PID_WINDOWS: u16 = 3;

fn platform_encoding_width(pid: u16, psid: u16) -> usize {
    match (pid, psid) {
        (PID_UNICODE, 3) => 2,
        (PID_UNICODE, 4) => 4,
        (PID_MACINTOSH, 0) => 1,
        (PID_WINDOWS, 0 | 1) => 2,
        (PID_WINDOWS, 10) => 4,
        _ => 0,
    }
}

fn supported_cmap_format(format: u16, pid: u16, psid: u16) -> bool {
    match format {
        0 => pid == PID_MACINTOSH && psid == 0,
        4 | 6 | 12 => true,
        _ => false,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Entry16 {
    end: u16,
    start: u16,
    delta: u16,
    offset: u16,
}

#[derive(Clone, Copy)]
pub(crate) struct Entry32 {
    start: u32,
    end: u32,
    delta: u32,
}

pub(crate) enum GlyphIndexFunc {
    Format0([u8; 256]),
    Format4 {
        entries: Vec<Entry16>,
        indexes_base: u32,
        indexes_length: u32,
    },
    Format6 {
        first_code: u16,
        entries: Vec<u16>,
    },
    Format12(Vec<Entry32>),
}

pub(crate) fn parse_cmap(src: &Source, cmap: Table) -> Result<GlyphIndexFunc, Error> {
    const HEADER_SIZE: u32 = 4;
    const ENTRY_SIZE: u32 = 8;
    if cmap.length < HEADER_SIZE {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    let num_subtables = u32::from(src.u16(cmap, 2)?);
    if cmap.length < HEADER_SIZE + ENTRY_SIZE * num_subtables {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    let (mut best_width, mut best_offset, mut best_length, mut best_format) = (0, 0u32, 0u32, 0u16);
    for i in 0..num_subtables {
        let buf = src.view(
            i64::from(cmap.offset + HEADER_SIZE + ENTRY_SIZE * i),
            i64::from(ENTRY_SIZE),
        )?;
        let pid = u16_at(buf, 0);
        let psid = u16_at(buf, 2);
        let width = platform_encoding_width(pid, psid);
        if width <= best_width {
            continue;
        }
        let offset = u32_at(buf, 4);
        if offset > cmap.length.wrapping_sub(4) {
            return Err(ERR_INVALID_CMAP_TABLE);
        }
        let buf = src.view(i64::from(cmap.offset + offset), 4)?;
        let format = u16_at(buf, 0);
        if !supported_cmap_format(format, pid, psid) {
            continue;
        }
        best_width = width;
        best_offset = offset;
        best_length = u32::from(u16_at(buf, 2));
        best_format = format;
    }
    if best_width == 0 {
        return Err(ERR_UNSUPPORTED_CMAP_ENCODINGS);
    }
    match best_format {
        0 => format0(src, cmap, best_offset, best_length),
        4 => format4(src, cmap, best_offset),
        6 => format6(src, cmap, best_offset),
        12 => format12(src, cmap, best_offset),
        _ => unreachable!(),
    }
}

fn format0(src: &Source, cmap: Table, offset: u32, length: u32) -> Result<GlyphIndexFunc, Error> {
    if length != 6 + 256 || offset + length > cmap.length {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    let buf = src.view(i64::from(cmap.offset + offset), i64::from(length))?;
    let mut table = [0u8; 256];
    table.copy_from_slice(&buf[6..]);
    Ok(GlyphIndexFunc::Format0(table))
}

fn format4(src: &Source, cmap: Table, mut offset: u32) -> Result<GlyphIndexFunc, Error> {
    const HEADER_SIZE: u32 = 14;
    if offset + HEADER_SIZE > cmap.length {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    let buf = src.view(i64::from(cmap.offset + offset), i64::from(HEADER_SIZE))?;
    offset += HEADER_SIZE;
    let mut seg_count = u16_at(buf, 6);
    if seg_count & 1 != 0 {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    seg_count /= 2;
    if u32::from(seg_count) > MAX_CMAP_SEGMENTS {
        return Err(ERR_UNSUPPORTED_NUMBER_OF_CMAP_SEGMENTS);
    }
    let e_length = 8 * u32::from(seg_count) + 2;
    if offset + e_length > cmap.length {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    let buf = src.view(i64::from(cmap.offset + offset), i64::from(e_length))?;
    offset += e_length;
    let n = usize::from(seg_count);
    let entries = (0..n)
        .map(|i| Entry16 {
            end: u16_at(buf, 2 * i),
            start: u16_at(buf, 2 * n + 2 + 2 * i),
            delta: u16_at(buf, 4 * n + 2 + 2 * i),
            offset: u16_at(buf, 6 * n + 2 + 2 * i),
        })
        .collect();
    Ok(GlyphIndexFunc::Format4 {
        entries,
        indexes_base: cmap.offset + offset,
        indexes_length: cmap.length.wrapping_sub(offset),
    })
}

fn format6(src: &Source, cmap: Table, mut offset: u32) -> Result<GlyphIndexFunc, Error> {
    const HEADER_SIZE: u32 = 10;
    if offset + HEADER_SIZE > cmap.length {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    let buf = src.view(i64::from(cmap.offset + offset), i64::from(HEADER_SIZE))?;
    offset += HEADER_SIZE;
    let first_code = u16_at(buf, 6);
    let entry_count = u16_at(buf, 8);
    let e_length = 2 * u32::from(entry_count);
    if offset + e_length > cmap.length {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    let mut entries = Vec::with_capacity(usize::from(entry_count));
    if entry_count != 0 {
        let buf = src.view(i64::from(cmap.offset + offset), i64::from(e_length))?;
        for i in 0..usize::from(entry_count) {
            entries.push(u16_at(buf, 2 * i));
        }
    }
    Ok(GlyphIndexFunc::Format6 {
        first_code,
        entries,
    })
}

fn format12(src: &Source, cmap: Table, mut offset: u32) -> Result<GlyphIndexFunc, Error> {
    const HEADER_SIZE: u32 = 16;
    if offset + HEADER_SIZE > cmap.length {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    let buf = src.view(i64::from(cmap.offset + offset), i64::from(HEADER_SIZE))?;
    let length = u32_at(buf, 4);
    if cmap.length < offset || length > cmap.length - offset {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    offset += HEADER_SIZE;
    let num_groups = u32_at(buf, 12);
    if num_groups > MAX_CMAP_SEGMENTS {
        return Err(ERR_UNSUPPORTED_NUMBER_OF_CMAP_SEGMENTS);
    }
    let e_length = 12 * num_groups;
    if HEADER_SIZE + e_length != length {
        return Err(ERR_INVALID_CMAP_TABLE);
    }
    let buf = src.view(i64::from(cmap.offset + offset), i64::from(e_length))?;
    let entries = (0..num_groups as usize)
        .map(|i| Entry32 {
            start: u32_at(buf, 12 * i),
            end: u32_at(buf, 4 + 12 * i),
            delta: u32_at(buf, 8 + 12 * i),
        })
        .collect();
    Ok(GlyphIndexFunc::Format12(entries))
}

impl GlyphIndexFunc {
    pub(crate) fn lookup(&self, src: &Source, r: u32) -> Result<GlyphIndex, Error> {
        match self {
            GlyphIndexFunc::Format0(table) => Ok(macintosh_encode(r)
                .map(|x| GlyphIndex::from(table[usize::from(x)]))
                .unwrap_or(0)),
            GlyphIndexFunc::Format4 {
                entries,
                indexes_base,
                indexes_length,
            } => {
                if r > 0xffff {
                    return Ok(0);
                }
                let c = r as u16;
                let (mut i, mut j) = (0usize, entries.len());
                while i < j {
                    let h = i + (j - i) / 2;
                    let entry = &entries[h];
                    if c < entry.start {
                        j = h;
                    } else if entry.end < c {
                        i = h + 1;
                    } else if entry.offset == 0 {
                        return Ok(c.wrapping_add(entry.delta));
                    } else {
                        // Go: uint32(entry.offset) + 2*uint32(h-len(entries)+int(c-entry.start)).
                        let index = h as i64 - entries.len() as i64
                            + i64::from(c.wrapping_sub(entry.start));
                        let offset =
                            u32::from(entry.offset).wrapping_add(2u32.wrapping_mul(index as u32));
                        if offset > *indexes_length || offset.wrapping_add(2) > *indexes_length {
                            return Err(ERR_INVALID_CMAP_TABLE);
                        }
                        let x = src.view(i64::from(indexes_base.wrapping_add(offset)), 2)?;
                        return Ok(u16_at(x, 0));
                    }
                }
                Ok(0)
            }
            GlyphIndexFunc::Format6 {
                first_code,
                entries,
            } => {
                if (r as u16) < *first_code {
                    return Ok(0);
                }
                let c = usize::from((r as u16) - first_code);
                Ok(entries.get(c).copied().unwrap_or(0))
            }
            GlyphIndexFunc::Format12(entries) => {
                let c = r;
                let (mut i, mut j) = (0usize, entries.len());
                while i < j {
                    let h = i + (j - i) / 2;
                    let entry = &entries[h];
                    if c < entry.start {
                        j = h;
                    } else if entry.end < c {
                        i = h + 1;
                    } else {
                        return Ok(
                            c.wrapping_sub(entry.start).wrapping_add(entry.delta) as GlyphIndex
                        );
                    }
                }
                Ok(0)
            }
        }
    }
}

/// x/text `charmap.Macintosh.EncodeRune`.
fn macintosh_encode(r: u32) -> Option<u8> {
    if r < 0x80 {
        return Some(r as u8);
    }
    MACINTOSH[0x80..]
        .iter()
        .position(|&point| point == r)
        .map(|index| (index + 0x80) as u8)
}

const MACINTOSH: [u32; 256] = [
    0x0000, 0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007, 0x0008, 0x0009, 0x000a, 0x000b,
    0x000c, 0x000d, 0x000e, 0x000f, 0x0010, 0x0011, 0x0012, 0x0013, 0x0014, 0x0015, 0x0016, 0x0017,
    0x0018, 0x0019, 0x001a, 0x001b, 0x001c, 0x001d, 0x001e, 0x001f, 0x0020, 0x0021, 0x0022, 0x0023,
    0x0024, 0x0025, 0x0026, 0x0027, 0x0028, 0x0029, 0x002a, 0x002b, 0x002c, 0x002d, 0x002e, 0x002f,
    0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037, 0x0038, 0x0039, 0x003a, 0x003b,
    0x003c, 0x003d, 0x003e, 0x003f, 0x0040, 0x0041, 0x0042, 0x0043, 0x0044, 0x0045, 0x0046, 0x0047,
    0x0048, 0x0049, 0x004a, 0x004b, 0x004c, 0x004d, 0x004e, 0x004f, 0x0050, 0x0051, 0x0052, 0x0053,
    0x0054, 0x0055, 0x0056, 0x0057, 0x0058, 0x0059, 0x005a, 0x005b, 0x005c, 0x005d, 0x005e, 0x005f,
    0x0060, 0x0061, 0x0062, 0x0063, 0x0064, 0x0065, 0x0066, 0x0067, 0x0068, 0x0069, 0x006a, 0x006b,
    0x006c, 0x006d, 0x006e, 0x006f, 0x0070, 0x0071, 0x0072, 0x0073, 0x0074, 0x0075, 0x0076, 0x0077,
    0x0078, 0x0079, 0x007a, 0x007b, 0x007c, 0x007d, 0x007e, 0x007f, 0x00c4, 0x00c5, 0x00c7, 0x00c9,
    0x00d1, 0x00d6, 0x00dc, 0x00e1, 0x00e0, 0x00e2, 0x00e4, 0x00e3, 0x00e5, 0x00e7, 0x00e9, 0x00e8,
    0x00ea, 0x00eb, 0x00ed, 0x00ec, 0x00ee, 0x00ef, 0x00f1, 0x00f3, 0x00f2, 0x00f4, 0x00f6, 0x00f5,
    0x00fa, 0x00f9, 0x00fb, 0x00fc, 0x2020, 0x00b0, 0x00a2, 0x00a3, 0x00a7, 0x2022, 0x00b6, 0x00df,
    0x00ae, 0x00a9, 0x2122, 0x00b4, 0x00a8, 0x2260, 0x00c6, 0x00d8, 0x221e, 0x00b1, 0x2264, 0x2265,
    0x00a5, 0x00b5, 0x2202, 0x2211, 0x220f, 0x03c0, 0x222b, 0x00aa, 0x00ba, 0x03a9, 0x00e6, 0x00f8,
    0x00bf, 0x00a1, 0x00ac, 0x221a, 0x0192, 0x2248, 0x2206, 0x00ab, 0x00bb, 0x2026, 0x00a0, 0x00c0,
    0x00c3, 0x00d5, 0x0152, 0x0153, 0x2013, 0x2014, 0x201c, 0x201d, 0x2018, 0x2019, 0x00f7, 0x25ca,
    0x00ff, 0x0178, 0x2044, 0x20ac, 0x2039, 0x203a, 0xfb01, 0xfb02, 0x2021, 0x00b7, 0x201a, 0x201e,
    0x2030, 0x00c2, 0x00ca, 0x00c1, 0x00cb, 0x00c8, 0x00cd, 0x00ce, 0x00cf, 0x00cc, 0x00d3, 0x00d4,
    0xf8ff, 0x00d2, 0x00da, 0x00db, 0x00d9, 0x0131, 0x02c6, 0x02dc, 0x00af, 0x02d8, 0x02d9, 0x02da,
    0x00b8, 0x02dd, 0x02db, 0x02c7,
];
