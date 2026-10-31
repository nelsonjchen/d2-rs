// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Go-text table layout Copyright 2021 The go-text authors (go-text/typesetting
// v0.3.4 font/opentype/tables/glyphs_colr_src.go and CPAL); Rust translation
// Copyright 2026 Nelson Chen.
//! Raw COLR/CPAL readers with go-text's search semantics: the COLRv1
//! BaseGlyphList is consulted before COLRv0 base glyph records, both with a
//! first-match lower bound, and the ClipList with a binary search.

use crate::error::Error;

/// A color-glyph lookup result (`tables.PaintTable`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColrPaint {
    /// A COLRv1 paint table at this absolute offset in the COLR table.
    V1(usize),
    /// `tables.PaintColrLayersResolved`: COLRv0 layer record range.
    V0Layers { first: usize, count: usize },
}

/// `tables.Layer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ColrLayer {
    pub glyph_id: u16,
    pub palette_index: u16,
}

/// `tables.ColorRecord` (stored BGRA in CPAL).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ColorRecord {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

pub(crate) struct Colr<'a> {
    pub data: &'a [u8],
    version: u16,
}

pub(crate) fn be16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([
        *data.get(at)?,
        *data.get(at.checked_add(1)?)?,
    ]))
}

pub(crate) fn be24(data: &[u8], at: usize) -> Option<usize> {
    let b = data.get(at..at.checked_add(3)?)?;
    Some((usize::from(b[0]) << 16) | (usize::from(b[1]) << 8) | usize::from(b[2]))
}

pub(crate) fn be32(data: &[u8], at: usize) -> Option<u32> {
    let b = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Go `sort.Search`.
fn search(n: usize, mut f: impl FnMut(usize) -> Option<bool>) -> Option<usize> {
    let (mut i, mut j) = (0usize, n);
    while i < j {
        let h = (i + j) >> 1;
        if !f(h)? {
            i = h + 1;
        } else {
            j = h;
        }
    }
    Some(i)
}

pub(crate) const LOOKUP_PANIC: &str = "color glyph table lookup panicked";

impl<'a> Colr<'a> {
    pub fn new(data: &'a [u8]) -> Option<Self> {
        let version = be16(data, 0)?;
        (version <= 1 && data.len() >= 14).then_some(Self { data, version })
    }

    fn v1_offset(&self, at: usize) -> usize {
        if self.version >= 1 {
            be32(self.data, at).unwrap_or(0) as usize
        } else {
            0
        }
    }

    /// `COLR1.Search` wrapped by fontface's `searchCOLRPaint` panic guard.
    pub fn search(&self, gid: u16) -> Result<Option<ColrPaint>, Error> {
        let corrupt = || Error::msg(LOOKUP_PANIC);
        // BaseGlyphList.paintForGlyph
        let list = self.v1_offset(14);
        if list != 0 {
            let n = be32(self.data, list).ok_or_else(corrupt)? as usize;
            let index = search(n, |i| Some(gid <= be16(self.data, list + 4 + i * 6)?))
                .ok_or_else(corrupt)?;
            if index < n && be16(self.data, list + 4 + index * 6).ok_or_else(corrupt)? == gid {
                let offset = be32(self.data, list + 6 + index * 6).ok_or_else(corrupt)? as usize;
                return Ok(Some(ColrPaint::V1(list + offset)));
            }
        }
        // colr0.paintForGlyph
        let n = usize::from(be16(self.data, 2).ok_or_else(corrupt)?);
        let records = be32(self.data, 4).ok_or_else(corrupt)? as usize;
        if n == 0 {
            return Ok(None);
        }
        let index =
            search(n, |i| Some(gid <= be16(self.data, records + i * 6)?)).ok_or_else(corrupt)?;
        if index >= n {
            return Ok(None);
        }
        let at = records + index * 6;
        if be16(self.data, at).ok_or_else(corrupt)? != gid {
            return Ok(None);
        }
        let first = usize::from(be16(self.data, at + 2).ok_or_else(corrupt)?);
        let count = usize::from(be16(self.data, at + 4).ok_or_else(corrupt)?);
        // Go slices layerRecords[first:first+count]; out of range panics.
        let num_layers = usize::from(be16(self.data, 12).ok_or_else(corrupt)?);
        if first + count > num_layers {
            return Err(corrupt());
        }
        Ok(Some(ColrPaint::V0Layers { first, count }))
    }

    pub fn v0_layer(&self, index: usize) -> Option<ColrLayer> {
        let layers = be32(self.data, 8)? as usize;
        let at = layers + index * 4;
        Some(ColrLayer {
            glyph_id: be16(self.data, at)?,
            palette_index: be16(self.data, at + 2)?,
        })
    }

    /// LayerList paint offsets (absolute in COLR), or None for a NULL list.
    pub fn layer_list(&self) -> Option<usize> {
        let list = self.v1_offset(18);
        (list != 0).then_some(list)
    }

    /// `ClipList.Search`: the absolute ClipBox offset.
    pub fn clip(&self, gid: u16) -> Option<usize> {
        let list = self.v1_offset(22);
        if list == 0 {
            return None;
        }
        let count = be32(self.data, list + 1)? as usize;
        let (mut i, mut j) = (0usize, count);
        while i < j {
            let h = i + (j - i) / 2;
            let at = list + 5 + h * 7;
            let start = be16(self.data, at)?;
            let end = be16(self.data, at + 2)?;
            if gid < start {
                j = h;
            } else if end < gid {
                i = h + 1;
            } else {
                return Some(list + be24(self.data, at + 4)?);
            }
        }
        None
    }
}

/// CPAL palette 0 (`face.CPAL[0]`), or None when the font has no palette.
pub(crate) fn cpal_palette0(cpal: &[u8]) -> Option<Vec<ColorRecord>> {
    let entries = usize::from(be16(cpal, 2)?);
    let palettes = be16(cpal, 4)?;
    if palettes == 0 {
        return None;
    }
    let records = be32(cpal, 8)? as usize;
    let first = usize::from(be16(cpal, 12)?);
    (0..entries)
        .map(|index| {
            let at = records + (first + index) * 4;
            let b = cpal.get(at..at + 4)?;
            Some(ColorRecord {
                blue: b[0],
                green: b[1],
                red: b[2],
                alpha: b[3],
            })
        })
        .collect()
}
