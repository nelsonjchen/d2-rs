// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2016 The Go Authors. All rights reserved.
// Rust translation of golang.org/x/image v0.45.0 font/sfnt (sfnt.go,
// cmap.go, gpos.go, truetype.go and postscript.go) for D2's fontface package.

mod cmap;
mod gpos;
mod probe_cff;
mod truetype;

use crate::fixed::{Int26_6, Point26_6, Rectangle26_6};
use std::fmt;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

pub(crate) const MAX_CMAP_SEGMENTS: u32 = 20000;
pub(crate) const MAX_COMPOUND_RECURSION_DEPTH: u32 = 8;
pub(crate) const MAX_COMPOUND_STACK_SIZE: u32 = 64;
pub(crate) const MAX_GLYPH_DATA_LENGTH: u32 = 64 * 1024;
const MAX_NUM_FONTS: u32 = 256;
const MAX_NUM_TABLES: usize = 256;
const MAX_TABLE_LENGTH: u32 = 1 << 29;
const MAX_TABLE_OFFSET: u32 = 1 << 29;
const DFONT_RESOURCE_DATA_OFFSET: u32 = 0x0000_0100;

/// Bounded random-access source for the system-font coverage probe.
///
/// Mirrors x/image's ReaderAt source and d2fonts' boundedFontReaderAt: every
/// byte actually read is charged, including the large-range existence check.
pub struct BoundedReader<R> {
    reader: R,
    remaining: u64,
    read: u64,
    min_size: u64,
    limit_hit: bool,
}

/// Table directory selected by the ReaderAt-backed font probe.
#[derive(Debug)]
pub struct ProbeTables {
    /// Table tag, absolute offset, and length, in directory order.
    pub entries: Vec<(u32, u32, u32)>,
    pub is_postscript: bool,
    pub final_offset: u32,
}

impl ProbeTables {
    fn table(&self, tag: u32) -> (u32, u32) {
        self.entries
            .iter()
            .find(|entry| entry.0 == tag)
            .map(|entry| (entry.1, entry.2))
            .unwrap_or((0, 0))
    }
}

/// Cached glyph map for a ReaderAt-backed system-font probe.
#[derive(Debug)]
pub enum ProbeCmap {
    Format0(Box<[u8; 256]>),
    Format4 {
        entries: Vec<(u16, u16, u16, u16)>,
        base: u32,
        length: u32,
    },
    Format6 {
        first: u16,
        entries: Vec<u16>,
    },
    Format12(Vec<(u32, u32, u32)>),
}

fn probe_buffer_capacity(length: usize) -> usize {
    (length + 1023) & !1023
}

/// ReaderAt scratch slices alias until source.view grows their backing array.
/// GPOS retains an outer slice across nested pair-position parsing, so both
/// the alias and x/image's KiB allocation rounding affect subsequent reads.
#[derive(Clone)]
struct ProbeBuffer {
    data: std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
    length: usize,
}

impl ProbeBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            data: std::rc::Rc::new(std::cell::RefCell::new(vec![0; capacity])),
            length: 0,
        }
    }

    fn replace(&mut self, data: &[u8]) {
        if data.len() > self.data.borrow().len() {
            *self = Self::new(probe_buffer_capacity(data.len()));
        }
        self.data.borrow_mut()[..data.len()].copy_from_slice(data);
        self.length = data.len();
    }

    fn bytes<const N: usize>(&self, offset: usize) -> std::io::Result<[u8; N]> {
        let data = self.data.borrow();
        data[..self.length]
            .get(offset..offset + N)
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "sfnt: invalid GPOS table")
            })
    }

    fn u16(&self, offset: usize) -> std::io::Result<u16> {
        Ok(u16::from_be_bytes(self.bytes(offset)?))
    }

    fn u32(&self, offset: usize) -> std::io::Result<u32> {
        Ok(u32::from_be_bytes(self.bytes(offset)?))
    }
}

impl<R: Read + Seek> BoundedReader<R> {
    // x/image source.varLenView reads the fixed header first, then the entire
    // table, charging both reads even when the count is zero.
    fn probe_var_len_view(
        &mut self,
        offset: i64,
        header: i64,
        count_at: usize,
        item: i64,
    ) -> std::io::Result<(Vec<u8>, usize)> {
        let first = self.view(offset, header)?;
        let count = usize::from(u16::from_be_bytes([first[count_at], first[count_at + 1]]));
        let bytes = self.view(offset, header + count as i64 * item)?;
        Ok((bytes, count))
    }

    /// Read the script and feature lists in parseGPOSKern order. The returned
    /// offsets and lookup indices feed the remaining lookup-table admission.
    pub fn font_gpos_lookups(
        &mut self,
        tables: &ProbeTables,
    ) -> std::io::Result<Option<(i64, Vec<usize>)>> {
        Ok(self
            .font_gpos_lookups_with_capacity(tables, 1024)?
            .map(|(base, indices, _)| (base, indices)))
    }

    fn font_gpos_lookups_with_capacity(
        &mut self,
        tables: &ProbeTables,
        mut capacity: usize,
    ) -> std::io::Result<Option<(i64, Vec<usize>, usize)>> {
        let (base, length) = tables.table(u32::from_be_bytes(*b"GPOS"));
        if length == 0 {
            return Ok(None);
        }
        let invalid =
            || std::io::Error::new(std::io::ErrorKind::InvalidData, "sfnt: invalid GPOS table");
        if length < 10 {
            return Err(invalid());
        }
        let base = i64::from(base);
        let header = self.view(base, 10)?;
        let u16_at = |b: &[u8], i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
        if u16_at(&header, 0) != 1 || u16_at(&header, 2) > 1 {
            return Err(invalid());
        }
        let scripts = base + i64::from(u16_at(&header, 4));
        let features = base + i64::from(u16_at(&header, 6));
        let lookups = base + i64::from(u16_at(&header, 8));
        let mut indices = Vec::new();
        for tag in [*b"latn", *b"DFLT"] {
            let (list, count) = self.probe_var_len_view(scripts, 2, 0, 6)?;
            capacity = capacity.max(probe_buffer_capacity(list.len()));
            let script_offset = (0..count)
                .find_map(|i| {
                    let at = 2 + i * 6;
                    (list[at..at + 4] == tag).then(|| u16_at(&list, at + 4))
                })
                .unwrap_or(0);
            if script_offset == 0 {
                continue;
            }
            let script = scripts + i64::from(script_offset);
            let lang = self.view(script, 2)?;
            let lang_offset = u16_at(&lang, 0);
            if lang_offset == 0 {
                continue;
            }
            let (lang, count) =
                self.probe_var_len_view(script + i64::from(lang_offset), 6, 4, 2)?;
            capacity = capacity.max(probe_buffer_capacity(lang.len()));
            indices.extend((0..count).map(|i| usize::from(u16_at(&lang, 6 + i * 2))));
            if !indices.is_empty() {
                break;
            }
        }
        if indices.is_empty() {
            return Ok(None);
        }
        let (list, count) = self.probe_var_len_view(features, 2, 0, 6)?;
        capacity = capacity.max(probe_buffer_capacity(list.len()));
        let mut result = Vec::new();
        for index in indices {
            if index >= count {
                return Err(invalid());
            }
            let at = 2 + index * 6;
            if &list[at..at + 4] != b"kern" {
                continue;
            }
            // parseGPOSFeaturesLookup passes nil for these nested reads, so
            // they neither overwrite nor grow the outer scratch buffer.
            let (feature, count) =
                self.probe_var_len_view(features + i64::from(u16_at(&list, at + 4)), 4, 2, 2)?;
            result.extend((0..count).map(|i| usize::from(u16_at(&feature, 4 + i * 2))));
        }
        Ok(Some((lookups, result, capacity)))
    }

    /// Admit GPOS kerning tables, preserving the ReaderAt read order used by
    /// x/image's parseGPOSKern. The coverage probe does not use kerning values.
    pub fn font_gpos(&mut self, tables: &ProbeTables) -> std::io::Result<()> {
        self.font_gpos_with_capacity(tables, 1024)
    }

    fn font_gpos_with_capacity(
        &mut self,
        tables: &ProbeTables,
        capacity: usize,
    ) -> std::io::Result<()> {
        let Some((base, indices, capacity)) =
            self.font_gpos_lookups_with_capacity(tables, capacity)?
        else {
            return Ok(());
        };
        let invalid =
            || std::io::Error::new(std::io::ErrorKind::InvalidData, "sfnt: invalid GPOS table");
        let mut buf = ProbeBuffer::new(capacity);
        let count = self.probe_gpos_var_len_view(&mut buf, base, 2, 0, 2)?;
        for index in indices {
            if index >= count {
                return Err(invalid());
            }
            // Go reads the next lookup offset from its current view, whose
            // backing bytes may have been overwritten by the previous lookup.
            let offset = base + i64::from(buf.u16(2 + index * 2)?);
            let count = self.probe_gpos_var_len_view(&mut buf, offset, 8, 4, 2)?;
            let kind = buf.u16(0)?;
            let flags = buf.u16(2)?;
            let mut subtables = Vec::with_capacity(count);
            for i in 0..count {
                subtables.push(offset + i64::from(buf.u16(6 + i * 2)?));
            }
            if kind == 9 {
                let mut supported = true;
                for subtable in &mut subtables {
                    self.probe_gpos_view(&mut buf, *subtable, 8)?;
                    if buf.u16(0)? != 1 {
                        return Err(invalid());
                    }
                    if buf.u16(2)? != 2 {
                        supported = false;
                        break;
                    }
                    *subtable += i64::from(buf.u32(4)?);
                }
                if !supported {
                    continue;
                }
            } else if kind != 2 {
                continue;
            }
            if flags & 0x10 != 0 {
                continue;
            }
            for subtable in subtables {
                self.probe_gpos_view(&mut buf, subtable, 4)?;
                let format = buf.u16(0)?;
                let coverage = subtable + i64::from(buf.u16(2)?);
                self.probe_gpos_definition(&mut buf, coverage, false)?;
                // The short declaration in each Go switch case shadows buf.
                // Nested reads overwrite the shared backing array until a
                // larger view allocates a new array; the outer view survives.
                let mut pair = buf.clone();
                match format {
                    1 => {
                        let count = self.probe_gpos_var_len_view(&mut pair, subtable, 10, 8, 2)?;
                        if pair.u16(4)? != 4 || pair.u16(6)? != 0 {
                            continue;
                        }
                        let mut last = 0;
                        for i in 0..count {
                            last = last.max(pair.u16(10 + i * 2)?);
                        }
                        self.probe_gpos_view(&mut pair, subtable + i64::from(last), 2)?;
                        let length = i64::from(last) + 2 + i64::from(pair.u16(0)?) * 4;
                        self.probe_gpos_view(&mut pair, subtable, length)?;
                    }
                    2 => {
                        self.probe_gpos_view(&mut pair, subtable, 16)?;
                        if pair.u16(4)? != 4 || pair.u16(6)? != 0 {
                            continue;
                        }
                        let class1 = subtable + i64::from(pair.u16(8)?);
                        let class2 = subtable + i64::from(pair.u16(10)?);
                        let size = i64::from(pair.u16(12)?) * i64::from(pair.u16(14)?) * 2;
                        self.probe_gpos_definition(&mut pair, class1, true)?;
                        self.probe_gpos_definition(&mut pair, class2, true)?;
                        self.probe_gpos_view(&mut pair, subtable + 16, size)?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn probe_gpos_view(
        &mut self,
        buf: &mut ProbeBuffer,
        offset: i64,
        length: i64,
    ) -> std::io::Result<()> {
        let data = self.view(offset, length)?;
        buf.replace(&data);
        Ok(())
    }

    fn probe_gpos_var_len_view(
        &mut self,
        buf: &mut ProbeBuffer,
        offset: i64,
        header: i64,
        count_at: usize,
        item: i64,
    ) -> std::io::Result<usize> {
        self.probe_gpos_view(buf, offset, header)?;
        let count = usize::from(buf.u16(count_at)?);
        self.probe_gpos_view(buf, offset, header + count as i64 * item)?;
        Ok(count)
    }

    fn probe_gpos_definition(
        &mut self,
        buf: &mut ProbeBuffer,
        offset: i64,
        class: bool,
    ) -> std::io::Result<()> {
        self.probe_gpos_view(buf, offset, 2)?;
        match (buf.u16(0)?, class) {
            (1, false) => {
                self.probe_gpos_var_len_view(buf, offset, 4, 2, 2)?;
            }
            (2, false) | (2, true) => {
                self.probe_gpos_var_len_view(buf, offset, 4, 2, 6)?;
            }
            (1, true) => {
                self.probe_gpos_var_len_view(buf, offset, 6, 4, 2)?;
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "sfnt: unsupported GPOS definition format",
                ));
            }
        }
        Ok(())
    }

    pub fn new(reader: R, limit: u64) -> Self {
        Self {
            reader,
            remaining: limit,
            read: 0,
            min_size: 0,
            limit_hit: false,
        }
    }

    pub fn bytes_read(&self) -> u64 {
        self.read
    }

    pub fn limit_hit(&self) -> bool {
        self.limit_hit
    }

    /// Select the widest supported cmap subtable in sfnt.Font.parseCmap order.
    /// The returned tuple is (absolute table offset, subtable offset, length,
    /// format); subsequent parsing must use this reader to charge its reads.
    pub fn font_cmap_selection(
        &mut self,
        tables: &ProbeTables,
    ) -> std::io::Result<(u32, u32, u32, u16)> {
        let invalid =
            || std::io::Error::new(std::io::ErrorKind::InvalidData, "sfnt: invalid cmap table");
        let (base, length) = tables.table(u32::from_be_bytes(*b"cmap"));
        if length < 4 {
            return Err(invalid());
        }
        let count = self.view(i64::from(base) + 2, 2)?;
        let count = u32::from(u16::from_be_bytes([count[0], count[1]]));
        if length < 4 + 8 * count {
            return Err(invalid());
        }
        let mut best = (0, 0, 0, 0);
        for i in 0..count {
            let entry = self.view(i64::from(base) + 4 + i64::from(i) * 8, 8)?;
            let pid = u16::from_be_bytes([entry[0], entry[1]]);
            let psid = u16::from_be_bytes([entry[2], entry[3]]);
            let width = match (pid, psid) {
                (0, 3) | (3, 0 | 1) => 2,
                (0, 4) | (3, 10) => 4,
                (1, 0) => 1,
                _ => 0,
            };
            if width <= best.0 {
                continue;
            }
            let offset = u32::from_be_bytes(entry[4..8].try_into().unwrap());
            if offset > length - 4 {
                return Err(invalid());
            }
            let header = self.view(i64::from(base) + i64::from(offset), 4)?;
            let format = u16::from_be_bytes([header[0], header[1]]);
            if !matches!(format, 4 | 6 | 12) && !(format == 0 && (pid, psid) == (1, 0)) {
                continue;
            }
            best = (
                width,
                offset,
                u32::from(u16::from_be_bytes([header[2], header[3]])),
                format,
            );
        }
        if best.0 == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sfnt: unsupported cmap encodings",
            ));
        }
        Ok((base, best.1, best.2, best.3))
    }

    /// Cache the selected subtable in x/image's makeCachedGlyphIndex read order.
    pub fn font_cmap(&mut self, tables: &ProbeTables) -> std::io::Result<ProbeCmap> {
        let (base, mut offset, selected_length, format) = self.font_cmap_selection(tables)?;
        let (_, length) = tables.table(u32::from_be_bytes(*b"cmap"));
        let invalid =
            || std::io::Error::new(std::io::ErrorKind::InvalidData, "sfnt: invalid cmap table");
        let fits = |offset: u32, size: u32| offset <= length && size <= length - offset;
        let u16at = |b: &[u8], i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
        let u32at = |b: &[u8], i: usize| u32::from_be_bytes(b[i..i + 4].try_into().unwrap());
        match format {
            0 => {
                if selected_length != 262 || !fits(offset, selected_length) {
                    return Err(invalid());
                }
                let data = self.view(i64::from(base) + i64::from(offset), 262)?;
                Ok(ProbeCmap::Format0(Box::new(data[6..].try_into().unwrap())))
            }
            4 => {
                if !fits(offset, 14) {
                    return Err(invalid());
                }
                let header = self.view(i64::from(base) + i64::from(offset), 14)?;
                offset += 14;
                let count = u16at(&header, 6);
                if count & 1 != 0 {
                    return Err(invalid());
                }
                let count = usize::from(count / 2);
                if count as u32 > MAX_CMAP_SEGMENTS {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "sfnt: unsupported number of cmap segments",
                    ));
                }
                let size = 8 * count as u32 + 2;
                if !fits(offset, size) {
                    return Err(invalid());
                }
                let data = self.view(i64::from(base) + i64::from(offset), i64::from(size))?;
                offset += size;
                let entries = (0..count)
                    .map(|i| {
                        (
                            u16at(&data, 2 * i),
                            u16at(&data, 2 * count + 2 + 2 * i),
                            u16at(&data, 4 * count + 2 + 2 * i),
                            u16at(&data, 6 * count + 2 + 2 * i),
                        )
                    })
                    .collect();
                Ok(ProbeCmap::Format4 {
                    entries,
                    base: base + offset,
                    length: length - offset,
                })
            }
            6 => {
                if !fits(offset, 10) {
                    return Err(invalid());
                }
                let header = self.view(i64::from(base) + i64::from(offset), 10)?;
                offset += 10;
                let first = u16at(&header, 6);
                let count = u32::from(u16at(&header, 8));
                let size = 2 * count;
                if !fits(offset, size) {
                    return Err(invalid());
                }
                let data = if count == 0 {
                    Vec::new()
                } else {
                    self.view(i64::from(base) + i64::from(offset), i64::from(size))?
                };
                Ok(ProbeCmap::Format6 {
                    first,
                    entries: data.chunks_exact(2).map(|b| u16at(b, 0)).collect(),
                })
            }
            12 => {
                if !fits(offset, 16) {
                    return Err(invalid());
                }
                let header = self.view(i64::from(base) + i64::from(offset), 16)?;
                let sub_length = u32at(&header, 4);
                if !fits(offset, sub_length) {
                    return Err(invalid());
                }
                offset += 16;
                let count = u32at(&header, 12);
                if count > MAX_CMAP_SEGMENTS {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "sfnt: unsupported number of cmap segments",
                    ));
                }
                let size = 12 * count;
                if 16 + size != sub_length {
                    return Err(invalid());
                }
                let data = self.view(i64::from(base) + i64::from(offset), i64::from(size))?;
                Ok(ProbeCmap::Format12(
                    data.chunks_exact(12)
                        .map(|b| (u32at(b, 0), u32at(b, 4), u32at(b, 8)))
                        .collect(),
                ))
            }
            _ => unreachable!(),
        }
    }

    /// Resolve one rune, charging format 4's deferred glyph-array read.
    pub fn cmap_glyph_index(&mut self, cmap: &ProbeCmap, value: char) -> std::io::Result<u16> {
        let c = value as u32;
        match cmap {
            ProbeCmap::Format0(table) => {
                Ok(cmap::macintosh_encode(c).map_or(0, |i| u16::from(table[i as usize])))
            }
            ProbeCmap::Format6 { first, entries } => Ok((c as u16)
                .checked_sub(*first)
                .and_then(|i| entries.get(i as usize))
                .copied()
                .unwrap_or(0)),
            ProbeCmap::Format12(entries) => {
                let mut lo = 0;
                let mut hi = entries.len();
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    let (start, end, delta) = entries[mid];
                    if c < start {
                        hi = mid;
                    } else if c > end {
                        lo = mid + 1;
                    } else {
                        return Ok(c.wrapping_sub(start).wrapping_add(delta) as u16);
                    }
                }
                Ok(0)
            }
            ProbeCmap::Format4 {
                entries,
                base,
                length,
            } => {
                if c > 0xffff {
                    return Ok(0);
                }
                let c = c as u16;
                let mut lo = 0;
                let mut hi = entries.len();
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    let (end, start, delta, range) = entries[mid];
                    if c < start {
                        hi = mid;
                    } else if c > end {
                        lo = mid + 1;
                    } else if range == 0 {
                        return Ok(c.wrapping_add(delta));
                    } else {
                        let index = mid as i64 - entries.len() as i64 + i64::from(c - start);
                        let offset = u32::from(range).wrapping_add(2u32.wrapping_mul(index as u32));
                        if offset > *length || offset.wrapping_add(2) > *length {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "sfnt: invalid cmap table",
                            ));
                        }
                        let b = self.view(i64::from(base.wrapping_add(offset)), 2)?;
                        return Ok(u16::from_be_bytes([b[0], b[1]]));
                    }
                }
                Ok(0)
            }
        }
    }

    /// Read the head and maxp fields in Font.initialize order.
    /// Returns the glyph count and the loca offset format for the next step.
    pub fn font_head_maxp(&mut self, tables: &ProbeTables) -> std::io::Result<(u16, bool)> {
        let invalid = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sfnt: invalid font metrics table",
            )
        };
        let (head, head_len) = tables.table(u32::from_be_bytes(*b"head"));
        if head_len != 54 {
            return Err(invalid());
        }
        let units = self.view(i64::from(head) + 18, 2)?;
        if u16::from_be_bytes([units[0], units[1]]) == 0 {
            return Err(invalid());
        }
        for offset in [36, 38, 40, 42] {
            self.view(i64::from(head) + offset, 2)?;
        }
        let format = self.view(i64::from(head) + 50, 2)?;
        let (maxp, maxp_len) = tables.table(u32::from_be_bytes(*b"maxp"));
        if maxp_len != if tables.is_postscript { 6 } else { 32 } {
            return Err(invalid());
        }
        let count = self.view(i64::from(maxp) + 4, 2)?;
        Ok((
            u16::from_be_bytes([count[0], count[1]]),
            u16::from_be_bytes([format[0], format[1]]) != 0,
        ))
    }

    /// Read TrueType glyph locations in the same single view as parseLoca.
    pub fn font_truetype_locations(
        &mut self,
        tables: &ProbeTables,
        glyphs: u16,
        long_offsets: bool,
    ) -> std::io::Result<Vec<u32>> {
        let (loca, length) = tables.table(u32::from_be_bytes(*b"loca"));
        let (glyf, _) = tables.table(u32::from_be_bytes(*b"glyf"));
        // parseGlyphData admits bitmap-only faces without reading a loca
        // table, giving them one zero location per glyph plus the sentinel.
        if length == 0 {
            let (_, cblc_length) = tables.table(u32::from_be_bytes(*b"CBLC"));
            if cblc_length != 0 {
                return Ok(vec![0; usize::from(glyphs) + 1]);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sfnt: invalid location data",
            ));
        }
        let expected = (u32::from(glyphs) + 1) * if long_offsets { 4 } else { 2 };
        if length != expected {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sfnt: invalid loca table",
            ));
        }
        let bytes = self.view(i64::from(loca), i64::from(length))?;
        Ok(if long_offsets {
            bytes
                .chunks_exact(4)
                .map(|b| u32::from_be_bytes(b.try_into().unwrap()).wrapping_add(glyf))
                .collect()
        } else {
            bytes
                .chunks_exact(2)
                .map(|b| {
                    (u32::from(u16::from_be_bytes(b.try_into().unwrap())) * 2).wrapping_add(glyf)
                })
                .collect()
        })
    }

    /// Validate kerning metadata in sfnt.Font.parseKern read order.
    pub fn font_kern(&mut self, tables: &ProbeTables) -> std::io::Result<()> {
        let invalid =
            || std::io::Error::new(std::io::ErrorKind::InvalidData, "sfnt: invalid kern table");
        let unsupported = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sfnt: unsupported kern table",
            )
        };
        let (offset, length) = tables.table(u32::from_be_bytes(*b"kern"));
        if length == 0 {
            return Ok(());
        }
        if length < 4 {
            return Err(invalid());
        }
        let header = self.view(i64::from(offset), 4)?;
        match u16::from_be_bytes([header[0], header[1]]) {
            1 if header[2] == 0 && header[3] == 0 => return Ok(()),
            0 if header[2] == 0 && header[3] == 0 => return Ok(()),
            0 => {}
            _ => return Err(unsupported()),
        }
        let remaining = length - 4;
        if remaining < 6 {
            return Err(invalid());
        }
        let sub = self.view(i64::from(offset) + 4, 6)?;
        if u16::from_be_bytes([sub[0], sub[1]]) != 0 {
            return Err(unsupported());
        }
        let sub_length = u16::from_be_bytes([sub[2], sub[3]]);
        if sub_length < 6 || u32::from(sub_length) > remaining {
            return Err(invalid());
        }
        if sub[5] != 1 || sub[4] != 0 {
            return Err(unsupported());
        }
        let remaining = remaining - 6;
        if remaining < 8 {
            return Err(invalid());
        }
        let data = self.view(i64::from(offset) + 10, 8)?;
        let count = u32::from(u16::from_be_bytes([data[0], data[1]]));
        let size = 8 + 6 * count;
        if remaining < size || sub_length - 6 != size as u16 {
            return Err(invalid());
        }
        Ok(())
    }

    /// Validate post metadata and charge the same field reads as parsePost.
    pub fn font_post(&mut self, tables: &ProbeTables, glyphs: u16) -> std::io::Result<()> {
        let invalid =
            || std::io::Error::new(std::io::ErrorKind::InvalidData, "sfnt: invalid post table");
        let (offset, length) = tables.table(u32::from_be_bytes(*b"post"));
        if length < 32 {
            return Err(invalid());
        }
        let version = self.view(i64::from(offset), 4)?;
        match u32::from_be_bytes(version.try_into().unwrap()) {
            0x10000 | 0x30000 => {}
            0x20000 if length >= 34 + 2 * u32::from(glyphs) => {}
            0x20000 => return Err(invalid()),
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "sfnt: unsupported post table",
                ));
            }
        }
        for (field, size) in [(4, 4), (8, 2), (10, 2), (12, 4)] {
            self.view(i64::from(offset) + field, size)?;
        }
        Ok(())
    }

    /// Read horizontal metrics and OS/2 fields in Font.initialize order.
    /// Call this after the glyph, cmap, kern and GPOS parsers have succeeded.
    pub fn font_horizontal_metrics(
        &mut self,
        tables: &ProbeTables,
        glyphs: u16,
    ) -> std::io::Result<bool> {
        let invalid = |name| std::io::Error::new(std::io::ErrorKind::InvalidData, name);
        let (hhea, length) = tables.table(u32::from_be_bytes(*b"hhea"));
        if length != 36 {
            return Err(invalid("sfnt: invalid hhea table"));
        }
        let count = self.view(i64::from(hhea) + 34, 2)?;
        let metrics = u32::from(u16::from_be_bytes([count[0], count[1]]));
        if metrics == 0 || metrics > u32::from(glyphs) {
            return Err(invalid("sfnt: invalid hhea table"));
        }
        for offset in [4, 6, 8, 20, 18] {
            self.view(i64::from(hhea) + offset, 2)?;
        }
        let (_, length) = tables.table(u32::from_be_bytes(*b"hmtx"));
        if length != 4 * metrics && length != 4 * metrics + 2 * (u32::from(glyphs) - metrics) {
            return Err(invalid("sfnt: invalid hmtx table"));
        }
        let (os2, length) = tables.table(u32::from_be_bytes(*b"OS/2"));
        if length == 0 {
            return Ok(false);
        }
        if length < 2 {
            return Err(invalid("sfnt: invalid OS/2 table"));
        }
        let version = self.view(i64::from(os2), 2)?;
        let modern = u16::from_be_bytes([version[0], version[1]]) >= 2;
        if !modern {
            if length < 68 {
                return Err(invalid("sfnt: invalid OS/2 table"));
            }
        } else {
            if length < 96 {
                return Err(invalid("sfnt: invalid OS/2 table"));
            }
            self.view(i64::from(os2) + 86, 2)?;
            self.view(i64::from(os2) + 88, 2)?;
        }
        Ok(modern)
    }

    /// Initialize a face in x/image's Font.initialize read order.
    /// Unsupported paths are distinguished from invalid fonts so callers do
    /// not silently omit a face whose coverage has not actually been probed.
    pub fn font_probe_cmap(&mut self, offset: u32, dfont: bool) -> std::io::Result<ProbeCmap> {
        let tables = self.font_tables(offset, dfont)?;
        let (glyphs, long_offsets) = self.font_head_maxp(&tables)?;
        let mut cff = None;
        let locations = if tables.is_postscript {
            cff = Some(probe_cff::probe_prefix(self, &tables, glyphs)?);
            None
        } else {
            Some(self.font_truetype_locations(&tables, glyphs, long_offsets)?)
        };
        let cmap = self.font_cmap(&tables)?;
        self.font_kern(&tables)?;
        // initializeTables, parseCmap and parseKern share buf. Glyph-location
        // and CFF parsing use independent buffers and do not grow that slice.
        let cmap_bytes = match &cmap {
            ProbeCmap::Format0(_) => 262,
            ProbeCmap::Format4 { entries, .. } => 14.max(8 * entries.len() + 2),
            ProbeCmap::Format6 { entries, .. } => 10.max(2 * entries.len()),
            ProbeCmap::Format12(entries) => 16.max(12 * entries.len()),
        };
        let capacity = probe_buffer_capacity((tables.entries.len() * 16).max(cmap_bytes).max(12));
        self.font_gpos_with_capacity(&tables, capacity)?;
        let modern_metrics = self.font_horizontal_metrics(&tables, glyphs)?;
        self.font_post(&tables, glyphs)?;
        if !modern_metrics {
            if let Some(cff) = cff.as_ref() {
                for value in ['x', 'H'] {
                    let glyph = self.cmap_glyph_index(&cmap, value)?;
                    if glyph != 0 {
                        probe_cff::probe_type2(self, cff, glyph)?;
                    }
                }
            } else if let Some(locations) = locations.as_ref() {
                self.font_legacy_metrics(&tables, &cmap, locations)?;
            }
        }
        Ok(cmap)
    }

    /// Follow glyphTopOS2's two lookups for legacy or absent OS/2 tables.
    /// Outline validation follows the same recursive reads as loadGlyf.
    fn font_legacy_metrics(
        &mut self,
        tables: &ProbeTables,
        cmap: &ProbeCmap,
        locations: &[u32],
    ) -> std::io::Result<()> {
        for value in ['x', 'H'] {
            let glyph = usize::from(self.cmap_glyph_index(cmap, value)?);
            if glyph == 0 {
                continue;
            }
            if tables.table(u32::from_be_bytes(*b"loca")).1 == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "sfnt: colored glyph",
                ));
            }
            self.probe_glyf(locations, glyph, 0, 0)?;
        }
        Ok(())
    }

    // The compound header is parsed before any child is read, just as in
    // x/image's loadCompoundGlyf; recursive views therefore charge in order.
    fn probe_glyf(
        &mut self,
        locations: &[u32],
        glyph: usize,
        stack_bottom: u32,
        depth: u32,
    ) -> std::io::Result<()> {
        let invalid =
            || std::io::Error::new(std::io::ErrorKind::InvalidData, "sfnt: invalid glyph data");
        let unsupported = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sfnt: unsupported compound glyph",
            )
        };
        let (&start, &end) = locations
            .get(glyph)
            .zip(locations.get(glyph + 1))
            .ok_or_else(invalid)?;
        let length = end
            .checked_sub(start)
            .filter(|n| *n <= MAX_GLYPH_DATA_LENGTH)
            .ok_or_else(invalid)?;
        let data = self.view(i64::from(start), i64::from(length))?;
        if data.is_empty() {
            return Ok(());
        }
        if data.len() < 10 {
            return Err(invalid());
        }
        let contours = i16::from_be_bytes([data[0], data[1]]);
        if contours == 0 {
            return Ok(());
        }
        if contours > 0 {
            let index = 10 + 2 * contours as usize;
            if index > data.len() {
                return Err(invalid());
            }
            let points = 1 + usize::from(u16::from_be_bytes([data[index - 2], data[index - 1]]));
            return truetype::load_simple_glyf(&data, index, points, contours, &mut Vec::new())
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err));
        }
        if contours != -1 {
            return Err(invalid());
        }
        let depth = depth + 1;
        if depth == MAX_COMPOUND_RECURSION_DEPTH {
            return Err(unsupported());
        }
        let mut index = 10;
        let mut children = Vec::new();
        loop {
            if stack_bottom + children.len() as u32 >= MAX_COMPOUND_STACK_SIZE {
                return Err(unsupported());
            }
            if index + 4 > data.len() {
                return Err(invalid());
            }
            let flags = u16::from_be_bytes([data[index], data[index + 1]]);
            let child = usize::from(u16::from_be_bytes([data[index + 2], data[index + 3]]));
            index += 4;
            let args = if flags & 1 != 0 { 4 } else { 2 };
            if index + args > data.len() {
                return Err(invalid());
            }
            index += args;
            if flags & 2 == 0 {
                return Err(unsupported());
            }
            let transform = if flags & 8 != 0 {
                2
            } else if flags & 64 != 0 {
                4
            } else if flags & 128 != 0 {
                8
            } else {
                0
            };
            if index + transform > data.len() {
                return Err(invalid());
            }
            index += transform;
            children.push(child);
            if flags & 32 == 0 {
                break;
            }
        }
        let stack_top = stack_bottom + children.len() as u32;
        for child in children {
            self.probe_glyf(locations, child, stack_top, depth)?;
        }
        Ok(())
    }

    /// Read and validate a face directory as sfnt.Font.initializeTables does.
    pub fn font_tables(&mut self, offset: u32, dfont: bool) -> std::io::Result<ProbeTables> {
        let invalid = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sfnt: invalid font table directory",
            )
        };
        let header = self.view(i64::from(offset), 12)?;
        let signature = u32::from_be_bytes(header[..4].try_into().unwrap());
        if !matches!(signature, 0x0001_0000 | 0x4f54_544f | 0x7472_7565) {
            return Err(invalid());
        }
        let count = u16::from_be_bytes(header[4..6].try_into().unwrap()) as usize;
        if count > MAX_NUM_TABLES {
            return Err(invalid());
        }
        let data = self.view(i64::from(offset) + 12, (count * 16) as i64)?;
        let mut entries = Vec::with_capacity(count);
        let mut previous = None;
        let mut final_offset = 0;
        for entry in data.chunks_exact(16) {
            let tag = u32::from_be_bytes(entry[..4].try_into().unwrap());
            if previous.is_some_and(|prev| tag <= prev) {
                return Err(invalid());
            }
            previous = Some(tag);
            let mut start = u32::from_be_bytes(entry[8..12].try_into().unwrap());
            let length = u32::from_be_bytes(entry[12..16].try_into().unwrap());
            if dfont {
                start = start.checked_add(offset).ok_or_else(invalid)?;
            }
            if start > MAX_TABLE_OFFSET || length > MAX_TABLE_LENGTH || start & 3 != 0 {
                return Err(invalid());
            }
            final_offset = final_offset.max(start + length);
            entries.push((tag, start, length));
        }
        Ok(ProbeTables {
            entries,
            is_postscript: signature == 0x4f54_544f,
            final_offset,
        })
    }

    /// Read the face offsets in an SFNT collection, including Mac dfont
    /// resources. The reads follow sfnt.Collection.initialize and parseDfont.
    pub fn collection_offsets(&mut self) -> std::io::Result<(Vec<u32>, bool)> {
        fn invalid() -> std::io::Error {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sfnt: invalid font collection",
            )
        }
        fn u16be(b: &[u8]) -> u16 {
            u16::from_be_bytes([b[0], b[1]])
        }
        fn u32be(b: &[u8]) -> u32 {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        }
        let header = self.view(0, 16)?;
        match u32be(&header) {
            0x0001_0000 | 0x4f54_544f | 0x7472_7565 => Ok((vec![0], false)),
            0x7474_6366 => {
                let count = u32be(&header[8..]);
                if count == 0 || count > MAX_NUM_FONTS {
                    return Err(invalid());
                }
                let data = self.view(12, i64::from(count) * 4)?;
                let offsets: Vec<_> = data.chunks_exact(4).map(u32be).collect();
                if offsets.iter().any(|&offset| offset > MAX_TABLE_OFFSET) {
                    return Err(invalid());
                }
                Ok((offsets, false))
            }
            DFONT_RESOURCE_DATA_OFFSET => {
                let map_offset = u32be(&header[4..]);
                let map_length = u32be(&header[12..]);
                if map_offset > MAX_TABLE_OFFSET || !(28..=MAX_TABLE_LENGTH).contains(&map_length) {
                    return Err(invalid());
                }
                let data = self.view(i64::from(map_offset) + 24, 2)?;
                let type_offset = i16::from_be_bytes([data[0], data[1]]) as i32;
                if type_offset < 28 || type_offset as u32 + 2 > map_length {
                    return Err(invalid());
                }
                let data = self.view(i64::from(map_offset) + i64::from(type_offset), 2)?;
                let count = i16::from_be_bytes([data[0], data[1]]) as i32;
                if count < 0 || (count as u32) * 8 > map_length - type_offset as u32 - 2 {
                    return Err(invalid());
                }
                let data = self.view(
                    i64::from(map_offset) + i64::from(type_offset) + 2,
                    i64::from(count) * 8,
                )?;
                let mut resources = None;
                for entry in data.chunks_exact(8) {
                    if u32be(entry) == 0x7366_6e74 {
                        let n = u16be(&entry[4..]) as i16 as i32;
                        let offset = u16be(&entry[6..]) as i16 as i32;
                        if n < 0 || offset < 0 {
                            return Err(invalid());
                        }
                        resources = Some((n as u32 + 1, offset as u32));
                        break;
                    }
                }
                let (count, offset) = resources.ok_or_else(invalid)?;
                if count > MAX_NUM_FONTS {
                    return Err(invalid());
                }
                let offset = type_offset as u32 + offset;
                if offset > map_length || count * 12 > map_length - offset {
                    return Err(invalid());
                }
                let data = self.view(
                    i64::from(map_offset) + i64::from(offset),
                    i64::from(count) * 12,
                )?;
                let offsets: Vec<_> = data
                    .chunks_exact(12)
                    .map(|entry| {
                        (u32be(&entry[4..]) & 0x00ff_ffff) + DFONT_RESOURCE_DATA_OFFSET + 4
                    })
                    .collect();
                if offsets.iter().any(|&offset| offset > MAX_TABLE_OFFSET) {
                    return Err(invalid());
                }
                Ok((offsets, true))
            }
            _ => Err(invalid()),
        }
    }

    fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> std::io::Result<()> {
        if bytes.len() as u64 > self.remaining - self.read {
            self.limit_hit = true;
            return Err(std::io::Error::other(
                "font cmap probe bytes exceed remaining scan limit",
            ));
        }
        self.reader.seek(SeekFrom::Start(offset))?;
        let mut filled = 0;
        while filled < bytes.len() {
            match self.reader.read(&mut bytes[filled..]) {
                Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => {
                    filled += n;
                    self.read += n as u64;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Read a source view, rejecting negative or overflowing ranges.
    pub fn view(&mut self, offset: i64, length: i64) -> std::io::Result<Vec<u8>> {
        let end = offset
            .checked_add(length)
            .filter(|&end| offset >= 0 && end >= offset)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "sfnt: invalid bounds")
            })? as u64;
        let length = usize::try_from(length).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "sfnt: invalid bounds")
        })?;
        if end > self.min_size && length > 1 << 20 {
            let mut byte = [0];
            self.read_at(end - 1, &mut byte).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "sfnt: invalid bounds")
            })?;
            self.min_size = end;
        }
        if length as u64 > self.remaining - self.read {
            self.limit_hit = true;
            return Err(std::io::Error::other(
                "font cmap probe bytes exceed remaining scan limit",
            ));
        }
        let mut bytes = vec![0; length];
        self.read_at(offset as u64, &mut bytes)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod bounded_reader_tests {
    use super::{BoundedReader, ProbeCmap, ProbeTables};
    use std::io::Cursor;

    #[test]
    fn legacy_metrics_probe_missing_and_empty_glyphs_in_order() {
        let tables = ProbeTables {
            entries: vec![(u32::from_be_bytes(*b"loca"), 0, 6)],
            is_postscript: false,
            final_offset: 0,
        };
        let cmap = ProbeCmap::Format6 {
            first: 'H' as u16,
            entries: {
                let mut entries = vec![0; ('x' as usize) - ('H' as usize) + 1];
                entries[0] = 1;
                entries
            },
        };
        let mut reader = BoundedReader::new(Cursor::new(Vec::<u8>::new()), 0);
        reader
            .font_legacy_metrics(&tables, &cmap, &[0, 0, 0])
            .unwrap();
        assert_eq!(reader.bytes_read(), 0);
        let mut reader = BoundedReader::new(Cursor::new(vec![0; 10]), 10);
        reader
            .font_legacy_metrics(&tables, &cmap, &[0, 0, 10])
            .unwrap();
        assert_eq!(reader.bytes_read(), 10);
        let mut data = vec![0; 10];
        data[1] = 1;
        let mut reader = BoundedReader::new(Cursor::new(data), 10);
        let err = reader
            .font_legacy_metrics(&tables, &cmap, &[0, 0, 10])
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn bounded_compound_glyph_reads_children_in_order() {
        // One single-point contour, then a compound glyph referencing it.
        let simple = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x31];
        let compound = [0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 0];
        let data = [simple.as_slice(), compound.as_slice()].concat();
        let mut reader = BoundedReader::new(Cursor::new(data), 31);
        reader.probe_glyf(&[0, 0, 15, 31], 2, 0, 0).unwrap();
        assert_eq!(reader.bytes_read(), 31);
        let mut reader = BoundedReader::new(
            Cursor::new([simple.as_slice(), compound.as_slice()].concat()),
            30,
        );
        assert!(reader.probe_glyf(&[0, 0, 15, 31], 2, 0, 0).is_err());
        assert_eq!(reader.bytes_read(), 16);
    }

    #[test]
    fn ordered_probe_matches_regular_face_cmap_and_enforces_budget() {
        let bytes = d2_rs_text::assets::SOURCE_SANS_PRO_REGULAR_TTF;
        let mut reader = BoundedReader::new(Cursor::new(bytes), u64::MAX);
        let (offsets, dfont) = reader.collection_offsets().unwrap();
        let cmap = reader.font_probe_cmap(offsets[0], dfont).unwrap();
        let font = super::parse(std::sync::Arc::from(bytes)).unwrap();
        for c in ['A', 'x', '∈', '😀'] {
            assert_eq!(
                reader.cmap_glyph_index(&cmap, c).unwrap(),
                font.glyph_index(c).unwrap()
            );
        }
        let mut limited = BoundedReader::new(Cursor::new(bytes), 16);
        limited.collection_offsets().unwrap();
        assert!(limited.font_probe_cmap(0, false).is_err());
        assert!(limited.limit_hit());
    }

    #[test]
    fn bounded_bitmap_face_admits_missing_loca_without_reading() {
        let mut reader = BoundedReader::new(Cursor::new(Vec::<u8>::new()), 0);
        let mut tables = ProbeTables {
            entries: vec![(u32::from_be_bytes(*b"CBLC"), 0, 8)],
            is_postscript: false,
            final_offset: 8,
        };
        assert_eq!(
            reader.font_truetype_locations(&tables, 2, false).unwrap(),
            vec![0; 3]
        );
        tables.entries.clear();
        assert!(reader.font_truetype_locations(&tables, 2, false).is_err());
    }

    #[test]
    fn bounded_gpos_search_charges_both_script_lookups() {
        let mut data = vec![0u8; 12];
        data[0..2].copy_from_slice(&1u16.to_be_bytes());
        data[4..6].copy_from_slice(&10u16.to_be_bytes());
        let tables = ProbeTables {
            entries: vec![(u32::from_be_bytes(*b"GPOS"), 0, 12)],
            is_postscript: false,
            final_offset: 0,
        };
        let mut reader = BoundedReader::new(Cursor::new(data.clone()), 18);
        assert!(reader.font_gpos_lookups(&tables).unwrap().is_none());
        assert_eq!(reader.bytes_read(), 18);
        let mut reader = BoundedReader::new(Cursor::new(data), 17);
        assert!(reader.font_gpos_lookups(&tables).is_err());
        assert!(reader.limit_hit());
    }

    #[test]
    fn bounded_gpos_finds_kern_lookup_in_latin_default_language() {
        let mut data = vec![0u8; 80];
        for (at, value) in [
            (0, 1u16),
            (4, 10),
            (6, 40),
            (8, 64),
            (10, 1),
            (16, 8),
            (18, 4),
            (26, 1),
            (28, 0),
            (40, 1),
            (46, 8),
            (50, 1),
            (52, 3),
        ] {
            data[at..at + 2].copy_from_slice(&value.to_be_bytes());
        }
        data[12..16].copy_from_slice(b"latn");
        data[42..46].copy_from_slice(b"kern");
        let tables = ProbeTables {
            entries: vec![(u32::from_be_bytes(*b"GPOS"), 0, 80)],
            is_postscript: false,
            final_offset: 0,
        };
        let mut reader = BoundedReader::new(Cursor::new(data), 56);
        assert_eq!(
            reader.font_gpos_lookups(&tables).unwrap(),
            Some((64, vec![3]))
        );
        assert_eq!(reader.bytes_read(), 56);
    }

    #[test]
    fn bounded_gpos_reads_pair_and_coverage_in_order() {
        let mut data = vec![0u8; 128];
        for (at, value) in [
            (0, 1u16),
            (4, 10),
            (6, 40),
            (8, 64),
            (10, 1),
            (16, 8),
            (18, 4),
            (26, 1),
            (40, 1),
            (46, 8),
            (50, 1),
            (52, 0),
            (64, 1),
            (66, 4),
            (68, 2),
            (72, 1),
            (74, 12),
            (80, 1),
            (82, 20),
            (84, 4),
            (100, 1),
        ] {
            data[at..at + 2].copy_from_slice(&value.to_be_bytes());
        }
        data[12..16].copy_from_slice(b"latn");
        data[42..46].copy_from_slice(b"kern");
        let tables = ProbeTables {
            entries: vec![(u32::from_be_bytes(*b"GPOS"), 0, 128)],
            is_postscript: false,
            final_offset: 0,
        };
        let mut reader = BoundedReader::new(Cursor::new(data.clone()), 122);
        reader.font_gpos(&tables).unwrap();
        assert_eq!(reader.bytes_read(), 122);
        let mut reader = BoundedReader::new(Cursor::new(data), 121);
        assert!(reader.font_gpos(&tables).is_err());
        assert!(reader.limit_hit());
    }

    #[test]
    fn bounded_gpos_reuses_reader_scratch_between_lookups() {
        let mut data = vec![0u8; 160];
        for (at, value) in [
            (0, 1u16),
            (4, 10),
            (6, 40),
            (8, 64),
            (10, 1),
            (16, 8),
            (18, 4),
            (26, 1),
            (40, 1),
            (46, 8),
            (50, 2),
            (54, 1),
            (64, 2),
            (66, 6),
            (68, 64),
            (70, 2),
            (74, 1),
            (76, 12),
            (82, 1),
            (84, 20),
            (86, 4),
            (102, 1),
            (104, 2),
            (132, 12),
        ] {
            data[at..at + 2].copy_from_slice(&value.to_be_bytes());
        }
        data[12..16].copy_from_slice(b"latn");
        data[42..46].copy_from_slice(b"kern");
        let tables = ProbeTables {
            entries: vec![(u32::from_be_bytes(*b"GPOS"), 0, 160)],
            is_postscript: false,
            final_offset: 160,
        };
        // x/image v0.45.0 parseGPOSKern reads the second offset from the
        // overwritten coverage buffer. ReaderAt capture of this GPOS table
        // embedded in Source Sans charges 146 bytes; retaining LookupList
        // separately instead charges 170 and rejects this permitted budget.
        let mut reader = BoundedReader::new(Cursor::new(data.clone()), 146);
        reader.font_gpos(&tables).unwrap();
        assert_eq!(reader.bytes_read(), 146);
        let mut reader = BoundedReader::new(Cursor::new(data), 145);
        assert!(reader.font_gpos(&tables).is_err());
        assert!(reader.limit_hit());
    }

    #[test]
    fn bounded_cmap_format4_charges_deferred_glyph_reads() {
        let mut data = vec![0u8; 46];
        data[2..4].copy_from_slice(&1u16.to_be_bytes());
        data[4..6].copy_from_slice(&3u16.to_be_bytes());
        data[6..8].copy_from_slice(&1u16.to_be_bytes());
        data[8..12].copy_from_slice(&12u32.to_be_bytes());
        data[12..14].copy_from_slice(&4u16.to_be_bytes());
        data[14..16].copy_from_slice(&34u16.to_be_bytes());
        data[18..20].copy_from_slice(&2u16.to_be_bytes());
        data[26..28].copy_from_slice(&0x2208u16.to_be_bytes());
        data[30..32].copy_from_slice(&0x2208u16.to_be_bytes());
        data[34..36].copy_from_slice(&4u16.to_be_bytes());
        data[38..40].copy_from_slice(&73u16.to_be_bytes());
        let tables = ProbeTables {
            entries: vec![(u32::from_be_bytes(*b"cmap"), 0, 46)],
            is_postscript: false,
            final_offset: 0,
        };
        let mut reader = BoundedReader::new(Cursor::new(data.clone()), 38);
        let cmap = reader.font_cmap(&tables).unwrap();
        assert_eq!(reader.bytes_read(), 38);
        assert!(reader.cmap_glyph_index(&cmap, '∈').is_err());
        assert!(reader.limit_hit());
        let mut reader = BoundedReader::new(Cursor::new(data), 40);
        let cmap = reader.font_cmap(&tables).unwrap();
        assert_eq!(reader.cmap_glyph_index(&cmap, '∈').unwrap(), 73);
        assert_eq!(reader.bytes_read(), 40);
    }

    #[test]
    fn cmap_selection_prefers_wider_supported_encoding_and_charges_reads() {
        let mut data = vec![0; 64];
        data[2..4].copy_from_slice(&2u16.to_be_bytes());
        data[4..6].copy_from_slice(&3u16.to_be_bytes());
        data[6..8].copy_from_slice(&1u16.to_be_bytes());
        data[8..12].copy_from_slice(&24u32.to_be_bytes());
        data[12..14].copy_from_slice(&3u16.to_be_bytes());
        data[14..16].copy_from_slice(&10u16.to_be_bytes());
        data[16..20].copy_from_slice(&32u32.to_be_bytes());
        data[24..28].copy_from_slice(&[0, 4, 0, 40]);
        data[32..36].copy_from_slice(&[0, 12, 0, 0]);
        let tables = ProbeTables {
            entries: vec![(u32::from_be_bytes(*b"cmap"), 0, 64)],
            is_postscript: false,
            final_offset: 64,
        };
        let mut reader = BoundedReader::new(Cursor::new(data), 26);
        assert_eq!(reader.font_cmap_selection(&tables).unwrap(), (0, 32, 0, 12));
        assert_eq!(reader.bytes_read(), 26);
    }

    #[test]
    fn kern_and_post_validate_before_charging_later_reads() {
        let mut data = vec![0; 96];
        data[2..4].copy_from_slice(&1_u16.to_be_bytes());
        data[6..8].copy_from_slice(&20_u16.to_be_bytes());
        data[9] = 1;
        data[10..12].copy_from_slice(&1_u16.to_be_bytes());
        data[32..36].copy_from_slice(&0x20000_u32.to_be_bytes());
        let mut tables = ProbeTables {
            entries: vec![
                (u32::from_be_bytes(*b"kern"), 0, 24),
                (u32::from_be_bytes(*b"post"), 32, 38),
            ],
            is_postscript: false,
            final_offset: 70,
        };
        let mut reader = BoundedReader::new(Cursor::new(data.clone()), 34);
        reader.font_kern(&tables).unwrap();
        reader.font_post(&tables, 2).unwrap();
        assert_eq!(reader.bytes_read(), 34);
        tables.entries[1].2 = 37;
        let mut reader = BoundedReader::new(Cursor::new(data), 34);
        reader.font_kern(&tables).unwrap();
        assert!(reader.font_post(&tables, 2).is_err());
        assert_eq!(reader.bytes_read(), 22);
    }

    #[test]
    fn horizontal_metrics_obey_table_lengths_and_read_budget() {
        let mut data = vec![0; 160];
        data[34..36].copy_from_slice(&2_u16.to_be_bytes());
        data[64..66].copy_from_slice(&2_u16.to_be_bytes());
        let mut tables = ProbeTables {
            entries: vec![
                (u32::from_be_bytes(*b"hhea"), 0, 36),
                (u32::from_be_bytes(*b"hmtx"), 36, 10),
                (u32::from_be_bytes(*b"OS/2"), 64, 96),
            ],
            is_postscript: false,
            final_offset: 160,
        };
        let mut reader = BoundedReader::new(Cursor::new(data.clone()), 18);
        reader.font_horizontal_metrics(&tables, 3).unwrap();
        assert_eq!(reader.bytes_read(), 18);
        tables.entries[1].2 = 9;
        let mut reader = BoundedReader::new(Cursor::new(data), 18);
        assert!(reader.font_horizontal_metrics(&tables, 3).is_err());
        assert_eq!(reader.bytes_read(), 12);
    }

    #[test]
    fn head_and_postscript_maxp_charge_individual_field_reads() {
        let mut data = vec![0; 128];
        data[18..20].copy_from_slice(&1000_u16.to_be_bytes());
        data[50..52].copy_from_slice(&1_u16.to_be_bytes());
        data[68..70].copy_from_slice(&42_u16.to_be_bytes());
        let tables = ProbeTables {
            entries: vec![
                (u32::from_be_bytes(*b"head"), 0, 54),
                (u32::from_be_bytes(*b"maxp"), 64, 6),
            ],
            is_postscript: true,
            final_offset: 70,
        };
        let mut reader = BoundedReader::new(Cursor::new(data), 14);
        assert_eq!(reader.font_head_maxp(&tables).unwrap(), (42, true));
        assert_eq!(reader.bytes_read(), 14);
    }

    #[test]
    fn loca_probe_reads_the_complete_table_once() {
        let tables = ProbeTables {
            entries: vec![
                (u32::from_be_bytes(*b"glyf"), 100, 20),
                (u32::from_be_bytes(*b"loca"), 0, 6),
            ],
            is_postscript: false,
            final_offset: 120,
        };
        let mut reader = BoundedReader::new(Cursor::new(vec![0, 0, 0, 2, 0, 5]), 6);
        assert_eq!(
            reader.font_truetype_locations(&tables, 2, false).unwrap(),
            vec![100, 104, 110]
        );
        assert_eq!(reader.bytes_read(), 6);
    }

    #[test]
    fn face_directory_uses_dfont_relative_offsets_and_charges_reads() {
        let mut data = vec![0; 128];
        data[32..36].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        data[36..38].copy_from_slice(&1u16.to_be_bytes());
        data[44..48].copy_from_slice(b"cmap");
        data[52..56].copy_from_slice(&32u32.to_be_bytes());
        data[56..60].copy_from_slice(&8u32.to_be_bytes());
        let mut reader = BoundedReader::new(Cursor::new(data), 28);
        let tables = reader.font_tables(32, true).unwrap();
        assert_eq!(tables.entries, vec![(0x636d_6170, 64, 8)]);
        assert_eq!(tables.final_offset, 72);
        assert!(!tables.is_postscript);
        assert_eq!(reader.bytes_read(), 28);
    }

    #[test]
    fn charges_partial_reads_and_refuses_reads_over_budget() {
        let mut reader = BoundedReader::new(Cursor::new(vec![1, 2, 3]), 5);
        assert!(reader.view(1, 4).is_err());
        assert_eq!(reader.bytes_read(), 2);
        assert!(reader.view(0, 4).is_err());
        assert!(reader.limit_hit());
        assert_eq!(reader.bytes_read(), 2);
    }

    #[test]
    fn large_views_charge_the_existence_check_once_per_known_range() {
        let length = (1 << 20) + 1;
        let mut reader = BoundedReader::new(Cursor::new(vec![7; length]), (2 * length + 1) as u64);
        assert_eq!(reader.view(0, length as i64).unwrap().len(), length);
        assert_eq!(reader.bytes_read(), (length + 1) as u64);
        reader.view(0, length as i64).unwrap();
        assert_eq!(reader.bytes_read(), (2 * length + 1) as u64);
    }

    #[test]
    fn collection_offsets_read_dfont_resource_map() {
        let mut data = vec![0; 512];
        data[0..4].copy_from_slice(&0x100_u32.to_be_bytes());
        data[4..8].copy_from_slice(&64_u32.to_be_bytes());
        data[12..16].copy_from_slice(&64_u32.to_be_bytes());
        data[88..90].copy_from_slice(&28_u16.to_be_bytes());
        data[92..94].copy_from_slice(&1_u16.to_be_bytes());
        data[94..98].copy_from_slice(b"sfnt");
        data[100..102].copy_from_slice(&10_u16.to_be_bytes());
        data[106..110].copy_from_slice(&32_u32.to_be_bytes());
        let mut reader = BoundedReader::new(Cursor::new(data), 64);
        assert_eq!(reader.collection_offsets().unwrap(), (vec![0x124], true));
        assert_eq!(reader.bytes_read(), 40);
    }

    #[test]
    fn dfont_collection_loads_resource_relative_truetype_tables() {
        let font = d2_rs_text::assets::SOURCE_SANS_PRO_REGULAR_TTF;
        let map_offset = (0x104 + font.len() + 3) & !3;
        let mut data = vec![0; map_offset + 64];
        data[0..4].copy_from_slice(&0x100_u32.to_be_bytes());
        data[4..8].copy_from_slice(&(map_offset as u32).to_be_bytes());
        data[12..16].copy_from_slice(&64_u32.to_be_bytes());
        data[0x100..0x104].copy_from_slice(&(font.len() as u32).to_be_bytes());
        data[0x104..0x104 + font.len()].copy_from_slice(font);
        data[map_offset + 24..map_offset + 26].copy_from_slice(&28_u16.to_be_bytes());
        data[map_offset + 28..map_offset + 30].copy_from_slice(&1_u16.to_be_bytes());
        data[map_offset + 30..map_offset + 34].copy_from_slice(b"sfnt");
        data[map_offset + 36..map_offset + 38].copy_from_slice(&10_u16.to_be_bytes());
        let parsed = crate::parse_face_collection_with_limit(&data, 1).unwrap();
        let face = parsed.face(0).unwrap();
        let plain_face = crate::parse_face(font, 0).unwrap();
        assert_eq!(
            face.shaping().nominal_glyph('A'),
            plain_face.shaping().nominal_glyph('A')
        );
        let collection = super::parse_collection(data.into()).unwrap();
        assert_eq!(collection.num_fonts(), 1);
        let wrapped = collection.font(0).unwrap();
        let plain = super::parse(std::sync::Arc::from(font)).unwrap();
        assert_eq!(wrapped.num_glyphs(), plain.num_glyphs());
        assert_eq!(wrapped.glyph_index('∈'), plain.glyph_index('∈'));
    }
}

/// `sfnt.GlyphIndex`.
pub type GlyphIndex = u16;
/// `sfnt.Units`.
pub type Units = i32;

/// The sfnt package's error values, with Go's exact texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `sfnt.ErrColoredGlyph`.
    ColoredGlyph,
    /// `sfnt.ErrNotFound`.
    NotFound,
    /// One of sfnt's unexported error values (its exact text).
    Other(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::ColoredGlyph => "sfnt: colored glyph",
            Error::NotFound => "sfnt: not found",
            Error::Other(text) => text,
        })
    }
}

impl std::error::Error for Error {}

macro_rules! sfnt_errors {
    ($($name:ident = $text:literal;)*) => {
        $(pub(crate) const $name: Error = Error::Other($text);)*
    };
}

sfnt_errors! {
    ERR_INVALID_BOUNDS = "sfnt: invalid bounds";
    ERR_INVALID_CMAP_TABLE = "sfnt: invalid cmap table";
    ERR_INVALID_FONT = "sfnt: invalid font";
    ERR_INVALID_FONT_COLLECTION = "sfnt: invalid font collection";
    ERR_INVALID_GPOS_TABLE = "sfnt: invalid GPOS table";
    ERR_INVALID_GLYPH_DATA = "sfnt: invalid glyph data";
    ERR_INVALID_GLYPH_DATA_LENGTH = "sfnt: invalid glyph data length";
    ERR_INVALID_HEAD_TABLE = "sfnt: invalid head table";
    ERR_INVALID_HHEA_TABLE = "sfnt: invalid hhea table";
    ERR_INVALID_HMTX_TABLE = "sfnt: invalid hmtx table";
    ERR_INVALID_KERN_TABLE = "sfnt: invalid kern table";
    ERR_INVALID_LOCA_TABLE = "sfnt: invalid loca table";
    ERR_INVALID_LOCATION_DATA = "sfnt: invalid location data";
    ERR_INVALID_MAXP_TABLE = "sfnt: invalid maxp table";
    ERR_INVALID_OS2_TABLE = "sfnt: invalid OS/2 table";
    ERR_INVALID_POST_TABLE = "sfnt: invalid post table";
    ERR_INVALID_SINGLE_FONT = "sfnt: invalid single font (data is a font collection)";
    ERR_INVALID_SOURCE_DATA = "sfnt: invalid source data";
    ERR_INVALID_DFONT = "sfnt: invalid dfont";
    ERR_INVALID_TABLE_OFFSET = "sfnt: invalid table offset";
    ERR_INVALID_TABLE_TAG_ORDER = "sfnt: invalid table tag order";
    ERR_UNSUPPORTED_CLASS_DEF_FORMAT = "sfnt: unsupported class definition format";
    ERR_UNSUPPORTED_CMAP_ENCODINGS = "sfnt: unsupported cmap encodings";
    ERR_UNSUPPORTED_COMPOUND_GLYPH = "sfnt: unsupported compound glyph";
    ERR_UNSUPPORTED_COVERAGE_FORMAT = "sfnt: unsupported coverage format";
    ERR_UNSUPPORTED_EXTENSION_POS_FORMAT = "sfnt: unsupported extension positioning format";
    ERR_UNSUPPORTED_GPOS_TABLE = "sfnt: unsupported GPOS table";
    ERR_UNSUPPORTED_GLYPH_DATA_LENGTH = "sfnt: unsupported glyph data length";
    ERR_UNSUPPORTED_KERN_TABLE = "sfnt: unsupported kern table";
    ERR_UNSUPPORTED_NUMBER_OF_CMAP_SEGMENTS = "sfnt: unsupported number of cmap segments";
    ERR_UNSUPPORTED_NUMBER_OF_FONTS = "sfnt: unsupported number of fonts";
    ERR_UNSUPPORTED_NUMBER_OF_TABLES = "sfnt: unsupported number of tables";
    ERR_UNSUPPORTED_POST_TABLE = "sfnt: unsupported post table";
    ERR_UNSUPPORTED_TABLE_OFFSET_LENGTH = "sfnt: unsupported table offset or length";
    ERR_INVALID_CFF_TABLE = "sfnt: invalid CFF table";
    ERR_UNSUPPORTED_CFF_VERSION = "sfnt: unsupported CFF version";
}

/// `sfnt.SegmentOp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SegmentOp {
    MoveTo = 0,
    LineTo = 1,
    QuadTo = 2,
    CubeTo = 3,
}

/// `sfnt.Segment`. Args holds up to three points; unused entries keep the
/// values Go leaves there (including compound-glyph transforms of zero).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Segment {
    pub op: SegmentOp,
    pub args: [Point26_6; 3],
}

/// `sfnt.Segments`.
pub type Segments = Vec<Segment>;

/// `Segments.Bounds`.
pub fn segments_bounds(segments: &[Segment]) -> Rectangle26_6 {
    if segments.is_empty() {
        return Rectangle26_6::default();
    }
    let mut bounds = Rectangle26_6 {
        min: Point26_6 {
            x: i32::MAX,
            y: i32::MAX,
        },
        max: Point26_6 {
            x: i32::MIN,
            y: i32::MIN,
        },
    };
    for segment in segments {
        let n = match segment.op {
            SegmentOp::QuadTo => 2,
            SegmentOp::CubeTo => 3,
            _ => 1,
        };
        for arg in &segment.args[..n] {
            if bounds.max.x < arg.x {
                bounds.max.x = arg.x;
            }
            if bounds.min.x > arg.x {
                bounds.min.x = arg.x;
            }
            if bounds.max.y < arg.y {
                bounds.max.y = arg.y;
            }
            if bounds.min.y > arg.y {
                bounds.min.y = arg.y;
            }
        }
    }
    bounds
}

/// `font.Hinting`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hinting {
    None,
    Vertical,
    Full,
}

/// `font.Metrics`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metrics {
    pub height: Int26_6,
    pub ascent: Int26_6,
    pub descent: Int26_6,
    pub x_height: Int26_6,
    pub cap_height: Int26_6,
    /// `CaretSlope` (image.Point X, Y).
    pub caret_slope: (i32, i32),
}

/// `sfnt.PostTable`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostTable {
    pub version: u32,
    pub italic_angle: f64,
    pub underline_position: i16,
    pub underline_thickness: i16,
    pub is_fixed_pitch: bool,
}

/// `scale`: x divided by unitsPerEm, rounded to the nearest 26.6 value.
pub(crate) fn scale(mut x: Int26_6, units_per_em: Units) -> Int26_6 {
    if x >= 0 {
        x = x.wrapping_add(units_per_em / 2);
    } else {
        x = x.wrapping_sub(units_per_em / 2);
    }
    x.wrapping_div(units_per_em)
}

pub(crate) fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

pub(crate) fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Table {
    pub offset: u32,
    pub length: u32,
}

/// `source` for in-memory data (`source.b`).
#[derive(Clone)]
pub(crate) struct Source {
    pub b: Arc<[u8]>,
}

impl Source {
    pub fn view(&self, offset: i64, length: i64) -> Result<&[u8], Error> {
        if 0 > offset || offset > offset + length {
            return Err(ERR_INVALID_BOUNDS);
        }
        if offset + length > self.b.len() as i64 {
            return Err(ERR_INVALID_BOUNDS);
        }
        Ok(&self.b[offset as usize..(offset + length) as usize])
    }

    pub fn var_len_view(
        &self,
        offset: i64,
        static_length: i64,
        count_offset: i64,
        item_length: i64,
    ) -> Result<(&[u8], usize), Error> {
        if 0 > offset || offset > offset + static_length {
            return Err(ERR_INVALID_BOUNDS);
        }
        if 0 > count_offset || count_offset + 1 >= static_length {
            return Err(ERR_INVALID_BOUNDS);
        }
        let buf = self.view(offset, static_length)?;
        let count = u16_at(buf, count_offset as usize) as i64;
        let buf = self.view(offset, static_length + count * item_length)?;
        Ok((buf, count as usize))
    }

    pub fn u16(&self, t: Table, i: i64) -> Result<u16, Error> {
        if i < 0 || (t.length as u64) < (i + 2) as u64 {
            return Err(ERR_INVALID_BOUNDS);
        }
        Ok(u16_at(self.view(t.offset as i64 + i, 2)?, 0))
    }

    pub fn u32(&self, t: Table, i: i64) -> Result<u32, Error> {
        if i < 0 || (t.length as u64) < (i + 4) as u64 {
            return Err(ERR_INVALID_BOUNDS);
        }
        Ok(u32_at(self.view(t.offset as i64 + i, 4)?, 0))
    }
}

/// `sfnt.Collection`, restricted to in-memory sources.
#[derive(Clone)]
pub struct Collection {
    src: Source,
    offsets: Vec<u32>,
    is_dfont: bool,
}

/// `sfnt.ParseCollection` (and `opentype.ParseCollection`).
pub fn parse_collection(src: Arc<[u8]>) -> Result<Collection, Error> {
    let src = Source { b: src };
    let buf = src.view(0, 16)?;
    let is_dfont = u32_at(buf, 0) == DFONT_RESOURCE_DATA_OFFSET;
    let offsets = match u32_at(buf, 0) {
        DFONT_RESOURCE_DATA_OFFSET => parse_dfont(&src, u32_at(buf, 4), u32_at(buf, 12))?,
        0x0001_0000 | 0x4f54_544f | 0x7472_7565 => vec![0],
        0x7474_6366 => {
            let num_fonts = u32_at(buf, 8);
            if num_fonts == 0 || num_fonts > MAX_NUM_FONTS {
                return Err(ERR_UNSUPPORTED_NUMBER_OF_FONTS);
            }
            let buf = src.view(12, 4 * i64::from(num_fonts))?;
            let mut offsets = Vec::with_capacity(num_fonts as usize);
            for index in 0..num_fonts as usize {
                let offset = u32_at(buf, 4 * index);
                if offset > MAX_TABLE_OFFSET {
                    return Err(ERR_UNSUPPORTED_TABLE_OFFSET_LENGTH);
                }
                offsets.push(offset);
            }
            offsets
        }
        _ => return Err(ERR_INVALID_FONT_COLLECTION),
    };
    Ok(Collection {
        src,
        offsets,
        is_dfont,
    })
}

// x/image/font/sfnt: Collection.parseDfont.
fn parse_dfont(src: &Source, map_offset: u32, map_length: u32) -> Result<Vec<u32>, Error> {
    if map_offset > MAX_TABLE_OFFSET || map_length > MAX_TABLE_LENGTH {
        return Err(ERR_UNSUPPORTED_TABLE_OFFSET_LENGTH);
    }
    if map_length < 28 {
        return Err(ERR_INVALID_DFONT);
    }
    let type_offset = i32::from(u16_at(src.view(i64::from(map_offset) + 24, 2)?, 0) as i16);
    if type_offset < 28 || type_offset as u32 + 2 > map_length {
        return Err(ERR_INVALID_DFONT);
    }
    let base = i64::from(map_offset) + i64::from(type_offset);
    let count = i32::from(u16_at(src.view(base, 2)?, 0) as i16);
    if count < 0 || count as u32 * 8 > map_length - type_offset as u32 - 2 {
        return Err(ERR_INVALID_DFONT);
    }
    let entries = src.view(base + 2, i64::from(count) * 8)?;
    let mut resource = None;
    for entry in entries.chunks_exact(8) {
        if u32_at(entry, 0) == 0x7366_6e74 {
            let n = i32::from(u16_at(entry, 4) as i16);
            let offset = i32::from(u16_at(entry, 6) as i16);
            if n < 0 || offset < 0 {
                return Err(ERR_INVALID_DFONT);
            }
            resource = Some((n as u32 + 1, offset as u32));
            break;
        }
    }
    let (count, list_offset) = resource.ok_or(ERR_INVALID_DFONT)?;
    if count > MAX_NUM_FONTS {
        return Err(ERR_UNSUPPORTED_NUMBER_OF_FONTS);
    }
    let list_offset = type_offset as u32 + list_offset;
    if list_offset > map_length || count * 12 > map_length - list_offset {
        return Err(ERR_INVALID_DFONT);
    }
    let entries = src.view(
        i64::from(map_offset) + i64::from(list_offset),
        i64::from(count) * 12,
    )?;
    entries
        .chunks_exact(12)
        .map(|entry| {
            let offset = (u32_at(entry, 4) & 0x00ff_ffff) + DFONT_RESOURCE_DATA_OFFSET + 4;
            if offset > MAX_TABLE_OFFSET {
                Err(ERR_UNSUPPORTED_TABLE_OFFSET_LENGTH)
            } else {
                Ok(offset)
            }
        })
        .collect()
}

impl Collection {
    /// `Collection.NumFonts`.
    pub fn num_fonts(&self) -> usize {
        self.offsets.len()
    }

    /// dfont tables use offsets relative to each embedded sfnt resource.
    pub(crate) fn dfont_offsets(&self) -> Option<&[u32]> {
        self.is_dfont.then_some(&self.offsets)
    }

    /// `Collection.Font`.
    pub fn font(&self, index: usize) -> Result<Font, Error> {
        let Some(&offset) = self.offsets.get(index) else {
            return Err(Error::NotFound);
        };
        Font::initialize(self.src.clone(), offset as i64, self.is_dfont)
    }
}

/// `sfnt.Parse`.
pub fn parse(src: Arc<[u8]>) -> Result<Font, Error> {
    Font::initialize(Source { b: src }, 0, false)
}

pub(crate) type KernFunc = Box<dyn Fn(GlyphIndex, GlyphIndex) -> Result<i16, Error> + Send + Sync>;

/// `sfnt.Font` for in-memory fonts. Like Go's, it has no
/// mutable state and may be shared between concurrent renders.
pub struct Font {
    src: Source,
    hmtx: Table,
    glyf_locations: Vec<u32>,
    cff: Option<probe_cff::GlyphData>,
    cmap: cmap::GlyphIndexFunc,
    ascent: i32,
    cap_height: i32,
    bounds: [i16; 4],
    descent: i32,
    is_color_bitmap: bool,
    kern_num_pairs: i32,
    kern_offset: i32,
    kern_funcs: Option<Vec<KernFunc>>,
    line_gap: i32,
    num_h_metrics: i32,
    post: PostTable,
    slope: [i32; 2],
    units_per_em: Units,
    x_height: i32,
}

impl fmt::Debug for Font {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("sfnt::Font")
            .field("num_glyphs", &self.num_glyphs())
            .field("units_per_em", &self.units_per_em)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Tables {
    cblc: Table,
    cff: Table,
    os2: Table,
    cmap: Table,
    glyf: Table,
    gpos: Table,
    head: Table,
    hhea: Table,
    hmtx: Table,
    kern: Table,
    loca: Table,
    maxp: Table,
    post: Table,
}

impl Font {
    fn initialize(src: Source, offset: i64, is_dfont: bool) -> Result<Font, Error> {
        let (tables, is_post_script) = initialize_tables(&src, offset, is_dfont)?;
        // parseHead
        if tables.head.length != 54 {
            return Err(ERR_INVALID_HEAD_TABLE);
        }
        let u = src.u16(tables.head, 18)?;
        if u == 0 {
            return Err(ERR_INVALID_HEAD_TABLE);
        }
        let units_per_em = Units::from(u);
        let mut bounds = [0i16; 4];
        for (index, bound) in bounds.iter_mut().enumerate() {
            *bound = src.u16(tables.head, 36 + 2 * index as i64)? as i16;
        }
        let index_to_loc_format = src.u16(tables.head, 50)? != 0;
        // parseMaxp
        if tables.maxp.length != if is_post_script { 6 } else { 32 } {
            return Err(ERR_INVALID_MAXP_TABLE);
        }
        let num_glyphs = i32::from(src.u16(tables.maxp, 4)?);
        // parseGlyphData
        let mut is_color_bitmap = false;
        let mut locations = Vec::new();
        let cff = if is_post_script {
            let mut reader = BoundedReader::new(std::io::Cursor::new(src.b.as_ref()), u64::MAX);
            let probe_tables = ProbeTables {
                entries: vec![(
                    u32::from_be_bytes(*b"CFF "),
                    tables.cff.offset,
                    tables.cff.length,
                )],
                is_postscript: true,
                final_offset: 0,
            };
            let data = probe_cff::probe_prefix(&mut reader, &probe_tables, num_glyphs as u16)
                .map_err(|e| {
                    if e.to_string() == "sfnt: unsupported CFF version" {
                        ERR_UNSUPPORTED_CFF_VERSION
                    } else {
                        ERR_INVALID_CFF_TABLE
                    }
                })?;
            locations = data
                .locations
                .iter()
                .map(|&x| u32::try_from(x).map_err(|_| ERR_INVALID_CFF_TABLE))
                .collect::<Result<_, _>>()?;
            Some(data)
        } else {
            None
        };
        if !is_post_script && tables.loca.length != 0 {
            locations = truetype::parse_loca(
                &src,
                tables.loca,
                tables.glyf.offset,
                index_to_loc_format,
                num_glyphs,
            )?;
        } else if !is_post_script && tables.cblc.length != 0 {
            is_color_bitmap = true;
            locations = vec![0; (num_glyphs + 1) as usize];
        }
        if locations.len() != (num_glyphs + 1) as usize {
            return Err(ERR_INVALID_LOCATION_DATA);
        }
        let glyph_index = cmap::parse_cmap(&src, tables.cmap)?;
        let (kern_num_pairs, kern_offset) = parse_kern(&src, tables.kern)?;
        let kern_funcs = gpos::parse_gpos_kern(&src, tables.gpos)?;
        // parseHhea
        if tables.hhea.length != 36 {
            return Err(ERR_INVALID_HHEA_TABLE);
        }
        let u = src.u16(tables.hhea, 34)?;
        if i32::from(u) > num_glyphs || u == 0 {
            return Err(ERR_INVALID_HHEA_TABLE);
        }
        let ascent = i32::from(src.u16(tables.hhea, 4)? as i16);
        let descent = i32::from(src.u16(tables.hhea, 6)? as i16);
        let line_gap = i32::from(src.u16(tables.hhea, 8)? as i16);
        let run = i32::from(src.u16(tables.hhea, 20)? as i16);
        let rise = i32::from(src.u16(tables.hhea, 18)? as i16);
        let num_h_metrics = i32::from(u);
        // parseHmtx
        if tables.hmtx.length != (4 * num_h_metrics) as u32
            && tables.hmtx.length != (4 * num_h_metrics + 2 * (num_glyphs - num_h_metrics)) as u32
        {
            return Err(ERR_INVALID_HMTX_TABLE);
        }
        // parseOS2
        let (has_x_height_cap_height, x_height, cap_height) = parse_os2(&src, tables.os2)?;
        let post = parse_post(&src, tables.post, num_glyphs)?;
        let mut font = Font {
            src,
            hmtx: tables.hmtx,
            glyf_locations: locations,
            cff,
            cmap: glyph_index,
            ascent,
            cap_height,
            bounds,
            descent,
            is_color_bitmap,
            kern_num_pairs,
            kern_offset,
            kern_funcs,
            line_gap,
            num_h_metrics,
            post,
            slope: [run, rise],
            units_per_em,
            x_height,
        };
        if !has_x_height_cap_height {
            let ppem = font.units_per_em;
            font.x_height = font.glyph_top_os2(ppem, 'x')?;
            font.cap_height = font.glyph_top_os2(ppem, 'H')?;
        }
        Ok(font)
    }

    fn glyph_top_os2(&self, ppem: Int26_6, r: char) -> Result<i32, Error> {
        let index = match self.glyph_index(r) {
            Ok(index) => index,
            Err(Error::NotFound) => 0,
            Err(error) => return Err(error),
        };
        if index == 0 {
            return Ok(0);
        }
        let mut min = 0;
        for segment in self.load_glyph(index, ppem)? {
            for arg in segment.args {
                if arg.y < min {
                    min = arg.y;
                }
            }
        }
        Ok(min)
    }

    /// `Font.NumGlyphs`.
    pub fn num_glyphs(&self) -> usize {
        self.glyf_locations.len() - 1
    }

    /// `Font.UnitsPerEm`.
    pub fn units_per_em(&self) -> Units {
        self.units_per_em
    }

    /// `Font.PostTable`.
    pub fn post_table(&self) -> &PostTable {
        &self.post
    }

    /// The raw font source shared by this reader.
    pub fn source(&self) -> &Arc<[u8]> {
        &self.src.b
    }

    /// `Font.Bounds`.
    pub fn bounds(&self, ppem: Int26_6, h: Hinting) -> Rectangle26_6 {
        let upem = self.units_per_em;
        let b = &self.bounds;
        let mut r = Rectangle26_6 {
            min: Point26_6 {
                x: scale(i32::from(b[0]).wrapping_mul(ppem), upem),
                y: scale(i32::from(b[3]).wrapping_mul(ppem), upem).wrapping_neg(),
            },
            max: Point26_6 {
                x: scale(i32::from(b[2]).wrapping_mul(ppem), upem),
                y: scale(i32::from(b[1]).wrapping_mul(ppem), upem).wrapping_neg(),
            },
        };
        if h == Hinting::Full {
            r.min.x &= !63;
            r.min.y &= !63;
            r.max.x = r.max.x.wrapping_add(63) & !63;
            r.max.y = r.max.y.wrapping_add(63) & !63;
        }
        r
    }

    /// `Font.GlyphIndex`: Ok(0) when the rune has no glyph.
    pub fn glyph_index(&self, r: char) -> Result<GlyphIndex, Error> {
        self.cmap.lookup(&self.src, u32::from(r))
    }

    /// `Font.GlyphIndex` for an arbitrary Go rune value (including values
    /// that are not Unicode scalar values).
    pub fn glyph_index_rune(&self, r: i32) -> Result<GlyphIndex, Error> {
        self.cmap.lookup(&self.src, r as u32)
    }

    pub(crate) fn view_glyph_data(&self, x: GlyphIndex) -> Result<&[u8], Error> {
        let xx = usize::from(x);
        if self.num_glyphs() <= xx {
            return Err(Error::NotFound);
        }
        let i = self.glyf_locations[xx];
        let j = self.glyf_locations[xx + 1];
        if j < i {
            return Err(ERR_INVALID_GLYPH_DATA_LENGTH);
        }
        if j - i > MAX_GLYPH_DATA_LENGTH {
            return Err(ERR_UNSUPPORTED_GLYPH_DATA_LENGTH);
        }
        self.src.view(i64::from(i), i64::from(j - i))
    }

    /// `Font.LoadGlyph`: the glyph's segments scaled to `ppem` (26.6), with
    /// the Y axis increasing down.
    pub fn load_glyph(&self, x: GlyphIndex, ppem: Int26_6) -> Result<Segments, Error> {
        if self.is_color_bitmap {
            return Err(Error::ColoredGlyph);
        }
        let mut segments = if let Some(cff) = &self.cff {
            self.view_glyph_data(x)?;
            let mut reader =
                BoundedReader::new(std::io::Cursor::new(self.src.b.as_ref()), u64::MAX);
            probe_cff::type2_outline(&mut reader, cff, x).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error::NotFound
                } else {
                    ERR_INVALID_CFF_TABLE
                }
            })?
        } else {
            let mut segments = Vec::new();
            let mut stack = [truetype::CompoundElem::default(); MAX_COMPOUND_STACK_SIZE as usize];
            truetype::load_glyf(self, &mut segments, &mut stack, x, 0, 0)?;
            segments
        };
        let upem = self.units_per_em;
        for segment in &mut segments {
            for arg in &mut segment.args {
                arg.x = scale(arg.x.wrapping_mul(ppem), upem);
                arg.y = scale(arg.y.wrapping_mul(ppem), upem).wrapping_neg();
            }
        }
        Ok(segments)
    }

    fn h_advance_units(&self, x: GlyphIndex) -> Result<Int26_6, Error> {
        let mut metric_index = x;
        let n = (self.num_h_metrics - 1) as GlyphIndex;
        if x > n {
            metric_index = n;
        }
        let buf = self
            .src
            .view(i64::from(self.hmtx.offset) + 4 * i64::from(metric_index), 2)?;
        Ok(Int26_6::from(u16_at(buf, 0)))
    }

    /// `Font.GlyphBounds`.
    pub fn glyph_bounds(
        &self,
        x: GlyphIndex,
        ppem: Int26_6,
        h: Hinting,
    ) -> Result<(Rectangle26_6, Int26_6), Error> {
        if usize::from(x) >= self.num_glyphs() {
            return Err(Error::NotFound);
        }
        let mut advance = scale(
            self.h_advance_units(x)?.wrapping_mul(ppem),
            self.units_per_em,
        );
        if h == Hinting::Full {
            advance = advance.wrapping_add(32) & !63;
        }
        let segments = self.load_glyph(x, ppem)?;
        Ok((segments_bounds(&segments), advance))
    }

    /// `Font.GlyphAdvance`.
    pub fn glyph_advance(
        &self,
        x: GlyphIndex,
        ppem: Int26_6,
        h: Hinting,
    ) -> Result<Int26_6, Error> {
        if usize::from(x) >= self.num_glyphs() {
            return Err(Error::NotFound);
        }
        let mut advance = scale(
            self.h_advance_units(x)?.wrapping_mul(ppem),
            self.units_per_em,
        );
        if h == Hinting::Full {
            advance = advance.wrapping_add(32) & !63;
        }
        Ok(advance)
    }

    /// `Font.Kern`.
    pub fn kern(
        &self,
        x0: GlyphIndex,
        x1: GlyphIndex,
        ppem: Int26_6,
        h: Hinting,
    ) -> Result<Int26_6, Error> {
        let quantize = |kern: Int26_6| {
            let kern = scale(kern.wrapping_mul(ppem), self.units_per_em);
            if h == Hinting::Full {
                kern.wrapping_add(32) & !63
            } else {
                kern
            }
        };
        if let Some(funcs) = &self.kern_funcs {
            for kf in funcs {
                match kf(x0, x1) {
                    Err(Error::NotFound) => continue,
                    Err(error) => return Err(error),
                    Ok(adv) => return Ok(quantize(Int26_6::from(adv))),
                }
            }
            return Err(Error::NotFound);
        }
        let n = self.num_glyphs();
        if usize::from(x0) >= n || usize::from(x1) >= n {
            return Err(Error::NotFound);
        }
        if self.kern_num_pairs == 0 {
            return Ok(0);
        }
        let key = (u32::from(x0) << 16) | u32::from(x1);
        let (mut lo, mut hi) = (0i32, self.kern_num_pairs);
        while lo < hi {
            let i = (lo + hi) / 2;
            const ENTRY_SIZE: i32 = 6;
            let buf = self.src.view(
                i64::from(self.kern_offset + i * ENTRY_SIZE),
                i64::from(ENTRY_SIZE),
            )?;
            let k = u32_at(buf, 0);
            if k < key {
                lo = i + 1;
            } else if k > key {
                hi = i;
            } else {
                return Ok(quantize(Int26_6::from(u16_at(buf, 4) as i16)));
            }
        }
        Ok(0)
    }

    /// `Font.Metrics`.
    pub fn metrics(&self, ppem: Int26_6, h: Hinting) -> Metrics {
        let upem = self.units_per_em;
        let mut m = Metrics {
            height: scale(
                (self.ascent - self.descent + self.line_gap).wrapping_mul(ppem),
                upem,
            ),
            ascent: scale(self.ascent.wrapping_mul(ppem), upem),
            descent: scale(self.descent.wrapping_mul(ppem), upem).wrapping_neg(),
            x_height: scale(self.x_height.wrapping_mul(ppem), upem),
            cap_height: scale(self.cap_height.wrapping_mul(ppem), upem),
            caret_slope: (self.slope[0], self.slope[1]),
        };
        if h == Hinting::Full {
            for value in [
                &mut m.height,
                &mut m.ascent,
                &mut m.descent,
                &mut m.x_height,
                &mut m.cap_height,
            ] {
                *value = value.wrapping_add(63) & !63;
            }
        }
        m
    }
}

fn initialize_tables(src: &Source, offset: i64, is_dfont: bool) -> Result<(Tables, bool), Error> {
    if i64::from(offset as i32) != offset {
        return Err(ERR_UNSUPPORTED_TABLE_OFFSET_LENGTH);
    }
    let buf = src.view(offset, 12)?;
    let mut is_post_script = false;
    match u32_at(buf, 0) {
        DFONT_RESOURCE_DATA_OFFSET | 0x7474_6366 => return Err(ERR_INVALID_SINGLE_FONT),
        0x0001_0000 | 0x7472_7565 => {}
        0x4f54_544f => is_post_script = true,
        _ => return Err(ERR_INVALID_FONT),
    }
    let num_tables = usize::from(u16_at(buf, 4));
    if num_tables > MAX_NUM_TABLES {
        return Err(ERR_UNSUPPORTED_NUMBER_OF_TABLES);
    }
    let buf = src.view(offset + 12, 16 * num_tables as i64)?;
    let mut tables = Tables::default();
    let mut final_table_offset: i32 = 0;
    let mut prev_tag = 0u32;
    for (index, entry) in buf.as_chunks::<16>().0.iter().enumerate() {
        let tag = u32_at(entry, 0);
        if index != 0 && tag <= prev_tag {
            return Err(ERR_INVALID_TABLE_TAG_ORDER);
        }
        prev_tag = tag;
        let (mut o, n) = (u32_at(entry, 8), u32_at(entry, 12));
        if is_dfont {
            o = o
                .checked_add(offset as u32)
                .ok_or(ERR_UNSUPPORTED_TABLE_OFFSET_LENGTH)?;
        }
        if o > MAX_TABLE_OFFSET || n > MAX_TABLE_LENGTH {
            return Err(ERR_UNSUPPORTED_TABLE_OFFSET_LENGTH);
        }
        if o & 3 != 0 {
            return Err(ERR_INVALID_TABLE_OFFSET);
        }
        if final_table_offset < o.wrapping_add(n) as i32 {
            final_table_offset = o.wrapping_add(n) as i32;
        }
        let table = Table {
            offset: o,
            length: n,
        };
        match tag {
            0x4342_4c43 => tables.cblc = table,
            0x4346_4620 => tables.cff = table,
            0x4f53_2f32 => tables.os2 = table,
            0x636d_6170 => tables.cmap = table,
            0x676c_7966 => tables.glyf = table,
            0x4750_4f53 => tables.gpos = table,
            0x6865_6164 => tables.head = table,
            0x6868_6561 => tables.hhea = table,
            0x686d_7478 => tables.hmtx = table,
            0x6b65_726e => tables.kern = table,
            0x6c6f_6361 => tables.loca = table,
            0x6d61_7870 => tables.maxp = table,
            0x706f_7374 => tables.post = table,
            _ => {}
        }
    }
    if final_table_offset as i64 > src.b.len() as i64 {
        return Err(ERR_INVALID_SOURCE_DATA);
    }
    Ok((tables, is_post_script))
}

fn parse_kern(src: &Source, kern: Table) -> Result<(i32, i32), Error> {
    if kern.length == 0 {
        return Ok((0, 0));
    }
    const HEADER_SIZE: i64 = 4;
    if i64::from(kern.length) < HEADER_SIZE {
        return Err(ERR_INVALID_KERN_TABLE);
    }
    let buf = src.view(i64::from(kern.offset), HEADER_SIZE)?;
    let offset = i64::from(kern.offset) + HEADER_SIZE;
    let length = i64::from(kern.length) - HEADER_SIZE;
    match u16_at(buf, 0) {
        0 => {
            if u16_at(buf, 2) == 0 {
                return Ok((0, 0));
            }
            parse_kern_version0(src, offset, length)
        }
        1 => {
            if buf[2] != 0 || buf[3] != 0 {
                return Err(ERR_UNSUPPORTED_KERN_TABLE);
            }
            Ok((0, 0))
        }
        _ => Err(ERR_UNSUPPORTED_KERN_TABLE),
    }
}

fn parse_kern_version0(
    src: &Source,
    mut offset: i64,
    mut length: i64,
) -> Result<(i32, i32), Error> {
    const HEADER_SIZE: i64 = 6;
    if length < HEADER_SIZE {
        return Err(ERR_INVALID_KERN_TABLE);
    }
    let buf = src.view(offset, HEADER_SIZE)?;
    if u16_at(buf, 0) != 0 {
        return Err(ERR_UNSUPPORTED_KERN_TABLE);
    }
    let mut subtable_length = u16_at(buf, 2);
    if i64::from(subtable_length) < HEADER_SIZE || length < i64::from(subtable_length) {
        return Err(ERR_INVALID_KERN_TABLE);
    }
    if buf[5] != 0x01 {
        return Err(ERR_UNSUPPORTED_KERN_TABLE);
    }
    let format = buf[4];
    offset += HEADER_SIZE;
    length -= HEADER_SIZE;
    subtable_length -= HEADER_SIZE as u16;
    if format != 0 {
        return Err(ERR_UNSUPPORTED_KERN_TABLE);
    }
    // parseKernFormat0
    const FORMAT0_HEADER: i64 = 8;
    const ENTRY_SIZE: i64 = 6;
    if length < FORMAT0_HEADER {
        return Err(ERR_INVALID_KERN_TABLE);
    }
    let buf = src.view(offset, FORMAT0_HEADER)?;
    let num_pairs = i32::from(u16_at(buf, 0));
    let n = FORMAT0_HEADER + ENTRY_SIZE * i64::from(num_pairs);
    if length < n || subtable_length != n as u16 {
        return Err(ERR_INVALID_KERN_TABLE);
    }
    Ok((num_pairs, (offset + FORMAT0_HEADER) as i32))
}

fn parse_os2(src: &Source, os2: Table) -> Result<(bool, i32, i32), Error> {
    if os2.length == 0 {
        return Ok((false, 0, 0));
    } else if os2.length < 2 {
        return Err(ERR_INVALID_OS2_TABLE);
    }
    let version = src.u16(os2, 0)?;
    if version < 2 {
        if os2.length < 68 {
            return Err(ERR_INVALID_OS2_TABLE);
        }
        return Ok((false, 0, 0));
    }
    if os2.length < 96 {
        return Err(ERR_INVALID_OS2_TABLE);
    }
    let xh = src.u16(os2, 86)?;
    let ch = src.u16(os2, 88)?;
    Ok((true, i32::from(xh as i16), i32::from(ch as i16)))
}

fn parse_post(src: &Source, post: Table, num_glyphs: i32) -> Result<PostTable, Error> {
    const HEADER_SIZE: u32 = 32;
    if post.length < HEADER_SIZE {
        return Err(ERR_INVALID_POST_TABLE);
    }
    let u = src.u32(post, 0)?;
    match u {
        0x10000 | 0x30000 => {}
        0x20000 => {
            if post.length < HEADER_SIZE + 2 + 2 * num_glyphs as u32 {
                return Err(ERR_INVALID_POST_TABLE);
            }
        }
        _ => return Err(ERR_UNSUPPORTED_POST_TABLE),
    }
    let angle = src.u32(post, 4)?;
    let up = src.u16(post, 8)?;
    let ut = src.u16(post, 10)?;
    let fp = src.u32(post, 12)?;
    Ok(PostTable {
        version: u,
        italic_angle: f64::from(angle as i32) / f64::from(0x10000),
        underline_position: up as i16,
        underline_thickness: ut as i16,
        is_fixed_pitch: fp != 0,
    })
}
