// SPDX-License-Identifier: MPL-2.0
//! Small TrueType measurement primitives.
//!
//! D2's v0.9.0 ruler obtains ordinary label advances and line metrics from
//! OpenType tables through `golang.org/x/image/font/sfnt`.  This module reads
//! the corresponding stable tables directly so native callers can measure
//! text from bundled or user-supplied font bytes without a Go or browser
//! runtime. It matches D2's TrueType horizontal ink bounds where the font
//! exposes `loca`/`glyf`; shaping and fallback still require GPOS/GSUB and
//! additional font faces.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
#[path = "markdown_ignorable_ranges.rs"]
mod markdown_ignorable_ranges;

#[derive(Clone, Debug, PartialEq, Eq)]
/// Failure while reading the subset of OpenType tables needed for advances.
pub enum FontError {
    /// The byte slice ended before a requested field.
    Truncated(&'static str),
    /// A required table is absent.
    MissingTable(&'static str),
    /// A table has an unsupported or malformed layout.
    InvalidTable(&'static str),
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated(table) => write!(f, "truncated {table} table"),
            Self::MissingTable(table) => write!(f, "font is missing the {table} table"),
            Self::InvalidTable(table) => write!(f, "invalid {table} table"),
        }
    }
}

impl std::error::Error for FontError {}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
/// Measured dimensions in CSS/layout pixels.
pub struct TextMetrics {
    /// Maximum line width from glyph advances.
    pub width: f64,
    /// Number of logical lines.
    pub lines: usize,
    /// Line height multiplied by [`Self::lines`].
    pub height: f64,
}

#[derive(Clone, Debug)]
/// Parsed TrueType metrics for one regular/bold/italic face.
pub struct TrueTypeFont {
    metrics: Arc<FontMetrics>,
}

#[derive(Debug)]
struct FontMetrics {
    units_per_em: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    advances: Vec<u16>,
    cmap: Cmap,
    /// Legacy horizontal kerning pairs. D2's v0.9.0 ruler deliberately uses
    /// the `kern` table through its metric-preserving sfnt face rather than
    /// OpenType shaping, so these adjustments are part of ordinary width
    /// measurement as well as Markdown measurement.
    kern: BTreeMap<u32, i16>,
    /// Optional horizontal ink bounds from `loca`/`glyf`. Fonts without
    /// TrueType outlines retain advance-only measurement.
    outlines: GlyphOutlines,
}

type OutlineBox = (i16, i16, i16, i16);

/// Codepoint to glyph pairs sorted by codepoint. Subtables are read in table
/// order and a later pair for a codepoint replaces an earlier one, as
/// successive map inserts would.
#[derive(Debug)]
struct Cmap(Vec<(u32, u16)>);

impl Cmap {
    fn from_table_order(mut pairs: Vec<(u32, u16)>) -> Self {
        // The stable sort keeps equal codepoints in table order; the dedup
        // then keeps the last value of each run in its first slot.
        pairs.sort_by_key(|&(codepoint, _)| codepoint);
        pairs.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                kept.1 = later.1;
            }
            same
        });
        Self(pairs)
    }

    fn get(&self, codepoint: &u32) -> Option<&u16> {
        let index = self
            .0
            .binary_search_by_key(codepoint, |&(key, _)| key)
            .ok()?;
        Some(&self.0[index].1)
    }
}

/// Font bytes retained for lazy outlining. Bundled faces borrow their static
/// data; custom input is copied, as callers may release it after parse.
#[derive(Debug)]
enum OutlineData {
    Static(&'static [u8]),
    Owned(Arc<[u8]>),
}

impl OutlineData {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Static(data) => data,
            Self::Owned(data) => data,
        }
    }
}

#[derive(Debug)]
struct GlyphOutlines {
    data: Option<OutlineData>,
    glyphs: usize,
    /// One cell per glyph, allocated on the first outline request.
    boxes: OnceLock<Box<[OnceLock<Option<OutlineBox>>]>>,
    fallback: Vec<Option<(i16, i16)>>,
    has_ink: OnceLock<bool>,
}

impl GlyphOutlines {
    fn outline_box(&self, glyph: u16) -> Option<OutlineBox> {
        if usize::from(glyph) >= self.glyphs {
            return None;
        }
        let cell = &self
            .boxes
            .get_or_init(|| (0..self.glyphs).map(|_| OnceLock::new()).collect())
            [usize::from(glyph)];
        *cell.get_or_init(|| {
            // Face parsing only reads table descriptors. Outlining (including
            // composite glyph traversal) runs once for each requested glyph.
            let face = ttf_parser::Face::parse(self.data.as_ref()?.bytes(), 0).ok()?;
            face.glyph_bounding_box(ttf_parser::GlyphId(glyph))
                .map(|b| (b.x_min, b.y_min, b.x_max, b.y_max))
        })
    }

    fn bounds(&self, glyph: u16) -> Option<(i16, i16)> {
        if self.data.is_none() {
            return self.fallback.get(usize::from(glyph)).copied().flatten();
        }
        self.outline_box(glyph)
            .and_then(|(left, _, right, _)| (left < right).then_some((left, right)))
    }

    fn has_ink(&self) -> bool {
        *self.has_ink.get_or_init(|| {
            if self.data.is_none() {
                self.fallback.iter().any(Option::is_some)
            } else {
                (0..self.glyphs).any(|glyph| self.bounds(glyph as u16).is_some())
            }
        })
    }
}

fn u16_at(data: &[u8], offset: usize, table: &'static str) -> Result<u16, FontError> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or(FontError::Truncated(table))?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn i16_at(data: &[u8], offset: usize, table: &'static str) -> Result<i16, FontError> {
    Ok(i16::from_be_bytes(
        u16_at(data, offset, table)?.to_be_bytes(),
    ))
}

fn u32_at(data: &[u8], offset: usize, table: &'static str) -> Result<u32, FontError> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or(FontError::Truncated(table))?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn table<'a>(data: &'a [u8], tag: &[u8; 4]) -> Result<&'a [u8], FontError> {
    let number = usize::from(u16_at(data, 4, "offset")?);
    for index in 0..number {
        let entry = 12 + index * 16;
        if data.get(entry..entry + 16).is_none() {
            return Err(FontError::Truncated("table directory"));
        }
        if data[entry..entry + 4] == *tag {
            let offset = usize::try_from(u32_at(data, entry + 8, "table directory")?)
                .map_err(|_| FontError::InvalidTable("table directory"))?;
            let length = usize::try_from(u32_at(data, entry + 12, "table directory")?)
                .map_err(|_| FontError::InvalidTable("table directory"))?;
            return data
                .get(offset..offset.saturating_add(length))
                .ok_or(FontError::Truncated("font table"));
        }
    }
    let name = match tag {
        b"head" => "head",
        b"hhea" => "hhea",
        b"maxp" => "maxp",
        b"hmtx" => "hmtx",
        b"cmap" => "cmap",
        _ => "font",
    };
    Err(FontError::MissingTable(name))
}

fn parse_cmap12(data: &[u8], offset: usize) -> Result<Cmap, FontError> {
    let format = u16_at(data, offset, "cmap format 12")?;
    if format != 12 {
        return Err(FontError::InvalidTable("cmap format 12"));
    }
    let groups = usize::try_from(u32_at(data, offset + 12, "cmap format 12")?)
        .map_err(|_| FontError::InvalidTable("cmap format 12"))?;
    let mut cmap = Vec::new();
    for index in 0..groups {
        let entry = offset + 16 + index * 12;
        let start = u32_at(data, entry, "cmap format 12")?;
        let end = u32_at(data, entry + 4, "cmap format 12")?;
        let glyph = u32_at(data, entry + 8, "cmap format 12")?;
        if end < start {
            return Err(FontError::InvalidTable("cmap format 12"));
        }
        for codepoint in start..=end {
            let glyph_id = glyph.saturating_add(codepoint - start);
            if glyph_id <= u32::from(u16::MAX) {
                cmap.push((codepoint, glyph_id as u16));
            }
        }
    }
    Ok(Cmap::from_table_order(cmap))
}

fn parse_cmap4(data: &[u8], offset: usize) -> Result<Cmap, FontError> {
    let format = u16_at(data, offset, "cmap format 4")?;
    if format != 4 {
        return Err(FontError::InvalidTable("cmap format 4"));
    }
    let segments = usize::from(u16_at(data, offset + 6, "cmap format 4")?) / 2;
    let end_codes = offset + 14;
    let start_codes = end_codes + segments * 2 + 2;
    let deltas = start_codes + segments * 2;
    let ranges = deltas + segments * 2;
    let mut cmap = Vec::new();
    for index in 0..segments {
        let start = u32::from(u16_at(data, start_codes + index * 2, "cmap format 4")?);
        let end = u32::from(u16_at(data, end_codes + index * 2, "cmap format 4")?);
        let delta = i32::from(i16_at(data, deltas + index * 2, "cmap format 4")?);
        let range = u16_at(data, ranges + index * 2, "cmap format 4")?;
        if end < start {
            return Err(FontError::InvalidTable("cmap format 4"));
        }
        for codepoint in start..=end {
            let glyph = if range == 0 {
                ((codepoint as i32 + delta) & 0xffff) as u16
            } else {
                let range_address = ranges + index * 2;
                let glyph_address = range_address
                    .checked_add(usize::from(range))
                    .and_then(|address| address.checked_add((codepoint - start) as usize * 2))
                    .ok_or(FontError::InvalidTable("cmap format 4"))?;
                let glyph = u32::from(u16_at(data, glyph_address, "cmap format 4")?);
                if glyph == 0 {
                    0
                } else {
                    ((glyph as i32 + delta) & 0xffff) as u16
                }
            };
            if glyph != 0 {
                cmap.push((codepoint, glyph));
            }
        }
    }
    Ok(Cmap::from_table_order(cmap))
}

fn parse_cmap(data: &[u8]) -> Result<Cmap, FontError> {
    let records = usize::from(u16_at(data, 2, "cmap")?);
    let mut format12 = None;
    let mut format4 = None;
    for index in 0..records {
        let entry = 4 + index * 8;
        let subtable = usize::try_from(u32_at(data, entry + 4, "cmap")?)
            .map_err(|_| FontError::InvalidTable("cmap"))?;
        match u16_at(data, subtable, "cmap")? {
            12 if format12.is_none() => format12 = Some(subtable),
            4 if format4.is_none() => format4 = Some(subtable),
            _ => {}
        }
    }
    if let Some(offset) = format12 {
        parse_cmap12(data, offset)
    } else if let Some(offset) = format4 {
        parse_cmap4(data, offset)
    } else {
        Err(FontError::InvalidTable("cmap"))
    }
}

fn parse_legacy_kern(data: &[u8]) -> Result<BTreeMap<u32, i16>, FontError> {
    let table = match table(data, b"kern") {
        Ok(table) => table,
        Err(FontError::MissingTable("font")) => return Ok(BTreeMap::new()),
        Err(FontError::MissingTable(_)) => return Ok(BTreeMap::new()),
        Err(error) => return Err(error),
    };
    if table.len() < 4 {
        return Err(FontError::Truncated("kern"));
    }
    // D2 v0.9.0 consumes the first format-0 horizontal subtable, matching
    // the legacy kern parser used by lib/textmeasure/fontface.go.
    let subtables = usize::from(u16_at(table, 2, "kern")?);
    let mut offset = 4usize;
    for _ in 0..subtables {
        if table.get(offset..offset + 14).is_none() {
            return Err(FontError::Truncated("kern"));
        }
        let version = u16_at(table, offset, "kern")?;
        let length = usize::from(u16_at(table, offset + 2, "kern")?);
        let coverage = u16_at(table, offset + 4, "kern")?;
        if length < 14 || offset.saturating_add(length) > table.len() {
            return Err(FontError::InvalidTable("kern"));
        }
        // Version 0, horizontal, format 0 is the only table accepted by the
        // Go implementation. Ignore other subtables rather than inventing
        // shaping semantics for them.
        let format = coverage >> 8;
        let horizontal = coverage & 0x0001 != 0;
        if version == 0 && format == 0 && horizontal {
            let pairs = usize::from(u16_at(table, offset + 6, "kern")?);
            let required = 14usize.saturating_add(pairs.saturating_mul(6));
            if required > length {
                return Err(FontError::InvalidTable("kern"));
            }
            let mut result = BTreeMap::new();
            for index in 0..pairs {
                let pair = offset + 14 + index * 6;
                let key = u32_at(table, pair, "kern")?;
                let value = i16_at(table, pair + 4, "kern")?;
                result.insert(key, value);
            }
            return Ok(result);
        }
        offset += length;
    }
    Ok(BTreeMap::new())
}

fn parse_glyph_bounds(
    head: &[u8],
    loca: &[u8],
    glyf: &[u8],
    glyphs: usize,
) -> Result<Vec<Option<(i16, i16)>>, FontError> {
    let format = i16_at(head, 50, "head")?;
    if !matches!(format, 0 | 1) {
        return Err(FontError::InvalidTable("head"));
    }
    let mut offsets = Vec::with_capacity(glyphs + 1);
    for index in 0..=glyphs {
        let offset = if format == 0 {
            usize::from(u16_at(loca, index * 2, "loca")?) * 2
        } else {
            usize::try_from(u32_at(loca, index * 4, "loca")?)
                .map_err(|_| FontError::InvalidTable("loca"))?
        };
        offsets.push(offset);
    }
    let mut bounds = Vec::with_capacity(glyphs);
    for index in 0..glyphs {
        let start = offsets[index];
        let end = offsets[index + 1];
        if end < start || end > glyf.len() {
            return Err(FontError::InvalidTable("loca"));
        }
        if end - start < 10 {
            bounds.push(None);
            continue;
        }
        let glyph = &glyf[start..end];
        let x_min = i16_at(glyph, 2, "glyf")?;
        let x_max = i16_at(glyph, 6, "glyf")?;
        bounds.push((x_min < x_max).then_some((x_min, x_max)));
    }
    Ok(bounds)
}

fn scaled_bound(units: i16, size: f64, units_per_em: u16) -> f64 {
    // x/image/font/opentype stores coordinates as signed 26.6 fixed-point
    // values. Match its round-to-nearest scale before converting back to
    // pixels instead of retaining an unrounded f64 coordinate.
    let scale = (size * 64.0 + 0.5) as i64;
    let denominator = i64::from(units_per_em) * 64;
    let numerator = scale * i64::from(units) * 64;
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        (numerator - denominator / 2) / denominator
    };
    rounded as f64 / 64.0
}

fn scaled_advance(units: u16, size: f64, units_per_em: u16) -> f64 {
    scaled_bound(i16::try_from(units).unwrap_or(i16::MAX), size, units_per_em)
}

fn scaled_metric(units: i16, size: f64, units_per_em: u16) -> f64 {
    let scale = (size * 64.0 + 0.5) as i64;
    let numerator = scale * i64::from(units.unsigned_abs());
    let denominator = i64::from(units_per_em);
    let rounded = (numerator + denominator - 1) / denominator;
    rounded as f64 / 64.0
}

fn atlas_rune_supported(character: char) -> bool {
    let codepoint = character as u32;
    character.is_ascii()
        || (0x80..=0xff).contains(&codepoint)
        || (0x25a0..=0x25ff).contains(&codepoint)
}

/// Original lib/textmeasure isDefaultIgnorableRune (pinned Go1.27 tables).
pub fn is_markdown_default_ignorable(character: char) -> bool {
    let codepoint = character as u32;
    let rows = markdown_ignorable_ranges::RANGES;
    let index = rows.partition_point(|&(start, _)| start <= codepoint);
    index > 0 && codepoint <= rows[index - 1].1
}

/// Raw sfnt table values, for callers that reproduce x/image/font's
/// fixed-point face arithmetic themselves (d2svg's native Markdown ruler).
impl TrueTypeFont {
    /// head.unitsPerEm.
    pub fn units_per_em(&self) -> u16 {
        self.metrics.units_per_em
    }

    /// hhea ascender, descender and lineGap in font units.
    pub fn hhea_metrics(&self) -> (i16, i16, i16) {
        (
            self.metrics.ascent,
            self.metrics.descent,
            self.metrics.line_gap,
        )
    }

    /// sfnt Font.GlyphIndex: 0 for an unmapped rune.
    pub fn glyph_index(&self, character: char) -> u16 {
        self.metrics
            .cmap
            .get(&(character as u32))
            .copied()
            .unwrap_or(0)
    }

    /// hmtx advance width in font units (the last record repeats).
    pub fn glyph_advance_units(&self, glyph: u16) -> u16 {
        self.metrics
            .advances
            .get(usize::from(glyph))
            .copied()
            .unwrap_or_else(|| self.metrics.advances.last().copied().unwrap_or(0))
    }

    /// Legacy `kern` format-0 pair value in font units.
    pub fn kern_units(&self, left: u16, right: u16) -> Option<i16> {
        self.metrics
            .kern
            .get(&(u32::from(left) << 16 | u32::from(right)))
            .copied()
    }

    /// Outline bounding box (xMin, yMin, xMax, yMax) in font units, or None
    /// for a glyph without contours.
    pub fn glyph_outline_box(&self, glyph: u16) -> Option<(i16, i16, i16, i16)> {
        self.metrics.outlines.outline_box(glyph)
    }
}

impl TrueTypeFont {
    /// Parses a standalone TrueType/OpenType font with TrueType outlines.
    pub fn parse(data: &[u8]) -> Result<Self, FontError> {
        Self::parse_retaining(data, || OutlineData::Owned(Arc::from(data)))
    }

    /// [`Self::parse`] for bundled bytes, which outlining borrows in place.
    fn parse_static(data: &'static [u8]) -> Result<Self, FontError> {
        Self::parse_retaining(data, || OutlineData::Static(data))
    }

    fn parse_retaining(
        data: &[u8],
        retain: impl FnOnce() -> OutlineData,
    ) -> Result<Self, FontError> {
        if data.len() < 12 {
            return Err(FontError::Truncated("offset"));
        }
        let head = table(data, b"head")?;
        let hhea = table(data, b"hhea")?;
        let maxp = table(data, b"maxp")?;
        let hmtx = table(data, b"hmtx")?;
        let cmap = table(data, b"cmap")?;
        let units_per_em = u16_at(head, 18, "head")?;
        if units_per_em == 0 {
            return Err(FontError::InvalidTable("head"));
        }
        let glyphs = usize::from(u16_at(maxp, 4, "maxp")?);
        let metrics = usize::from(u16_at(hhea, 34, "hhea")?);
        if metrics == 0 || metrics > glyphs || hmtx.len() < metrics * 4 {
            return Err(FontError::InvalidTable("hmtx"));
        }
        let mut advances = Vec::with_capacity(glyphs);
        for index in 0..metrics {
            advances.push(u16_at(hmtx, index * 4, "hmtx")?);
        }
        let last = *advances.last().unwrap();
        advances.resize(glyphs, last);
        let outlines = if ttf_parser::Face::parse(data, 0).is_ok() {
            GlyphOutlines {
                data: Some(retain()),
                glyphs,
                boxes: OnceLock::new(),
                fallback: Vec::new(),
                has_ink: OnceLock::new(),
            }
        } else {
            let fallback = match (table(data, b"loca"), table(data, b"glyf")) {
                (Ok(loca), Ok(glyf)) => parse_glyph_bounds(head, loca, glyf, glyphs)?,
                _ => vec![None; glyphs],
            };
            GlyphOutlines {
                data: None,
                glyphs: 0,
                boxes: OnceLock::new(),
                fallback,
                has_ink: OnceLock::new(),
            }
        };
        Ok(Self {
            metrics: Arc::new(FontMetrics {
                units_per_em,
                ascent: i16_at(hhea, 4, "hhea")?,
                descent: i16_at(hhea, 6, "hhea")?,
                line_gap: i16_at(hhea, 8, "hhea")?,
                advances,
                cmap: parse_cmap(cmap)?,
                kern: parse_legacy_kern(data)?,
                outlines,
            }),
        })
    }

    /// Original Ruler.cssFontBoxMetrics: sfnt metrics at rounded 26.6 ppem,
    /// then round ascent/descent to whole CSS pixels. Ordinary label metrics
    /// deliberately keep their separate historical rounding contract.
    pub fn css_box_metrics(&self, size: f64) -> (f64, f64) {
        let ppem = (size * 64.0).round() as i64;
        let denominator = i64::from(self.metrics.units_per_em);
        let scaled = |units: i16| {
            let numerator = i64::from(units) * ppem;
            let adjusted = if numerator >= 0 {
                numerator + denominator / 2
            } else {
                numerator - denominator / 2
            };
            adjusted / denominator
        };
        (
            (scaled(self.metrics.ascent) as f64 / 64.0).round(),
            (-scaled(self.metrics.descent) as f64 / 64.0).round(),
        )
    }

    /// Original Ruler.cssNormalLineHeight, using sfnt's rounded 26.6 height.
    pub fn css_normal_line_height(&self, size: f64) -> f64 {
        let units = i64::from(self.metrics.ascent) - i64::from(self.metrics.descent)
            + i64::from(self.metrics.line_gap);
        let denominator = i64::from(self.metrics.units_per_em);
        let numerator = units * (size * 64.0).round() as i64;
        (((numerator + denominator / 2) / denominator) as f64 / 64.0).round()
    }

    /// Original markdownInlineLineHeight's host-font fallback contribution.
    pub fn markdown_fallback_line_height(&self, text: &str, size: f64) -> f64 {
        let supported = |text: &str| {
            text.chars().all(|ch| {
                ch == '\0'
                    || self
                        .metrics
                        .cmap
                        .get(&(ch as u32))
                        .is_some_and(|glyph| *glyph != 0)
            })
        };
        let ignorable = |ch: char| {
            let rows = markdown_ignorable_ranges::RANGES;
            let index = rows.partition_point(|&(start, _)| start <= ch as u32);
            index > 0 && ch as u32 <= rows[index - 1].1
        };
        let mut result: f64 = 0.0;
        let mut start = 0;
        for end in crate::uniseg::grapheme_boundaries(text) {
            let cluster = &text[start..end];
            start = end;
            if supported(cluster) {
                continue;
            }
            let color = crate::uniseg::is_color_emoji(cluster);
            let filtered: String = cluster.chars().filter(|ch| !ignorable(*ch)).collect();
            if !color && (filtered.is_empty() || supported(&filtered)) {
                continue;
            }
            let factor = if color {
                1.625
            } else if filtered.chars().any(crate::uniseg::is_symbol) {
                1.5
            } else {
                1.375
            };
            result = result.max((size * factor).round());
        }
        result
    }

    /// Returns the glyph advance at a requested pixel size.
    pub fn advance(&self, character: char, size: f64) -> f64 {
        let glyph = self
            .metrics
            .cmap
            .get(&(character as u32))
            .copied()
            .unwrap_or(0);
        self.advance_glyph(glyph, size)
    }

    fn advance_glyph(&self, glyph: u16, size: f64) -> f64 {
        let units = self
            .metrics
            .advances
            .get(usize::from(glyph))
            .copied()
            .unwrap_or_else(|| self.metrics.advances.first().copied().unwrap_or(0));
        scaled_advance(units, size, self.metrics.units_per_em)
    }

    fn kern(&self, previous: Option<u16>, current: u16, size: f64) -> f64 {
        let Some(previous) = previous else { return 0.0 };
        let key = u32::from(previous) << 16 | u32::from(current);
        self.metrics
            .kern
            .get(&key)
            .map(|units| scaled_metric(*units, size, self.metrics.units_per_em))
            .unwrap_or(0.0)
    }

    /// Cursor advances for Source's code token runs. Unlike label measurement,
    /// this keeps signed sfnt kerning rounded to 26.6 units and performs no
    /// missing-glyph cell substitution or tab-stop interpretation. The caller
    /// expands tabs to four literal spaces before measuring each token.
    pub fn measure_code_advance(&self, text: &str, size: f64) -> f64 {
        let mut pen = 0.0;
        let mut previous = None;
        for character in text.chars() {
            let glyph = self
                .metrics
                .cmap
                .get(&(character as u32))
                .copied()
                .unwrap_or(0);
            if let Some(previous) = previous {
                let key = u32::from(previous) << 16 | u32::from(glyph);
                if let Some(units) = self.metrics.kern.get(&key) {
                    pen += scaled_bound(*units, size, self.metrics.units_per_em);
                }
            }
            pen += self.advance_glyph(glyph, size);
            previous = Some(glyph);
        }
        pen
    }

    /// Original scene code metric contract, including missing-glyph errors and per-rune cancellation.
    pub fn measure_code_advance_checked(
        &self,
        text: &str,
        size: f64,
        cancel: Option<&d2_rs_core::CancellationToken>,
    ) -> Result<f64, String> {
        let mut pen = 0.0;
        let mut previous = None;
        for (byte_offset, character) in text.char_indices() {
            if let Some(error) = cancel.and_then(d2_rs_core::CancellationToken::error) {
                return Err(error.to_string());
            }
            let glyph = self
                .metrics
                .cmap
                .get(&(character as u32))
                .copied()
                .unwrap_or(0);
            if glyph == 0 {
                return Err(format!(
                    "missing glyph U+{:04X} at byte {byte_offset}",
                    character as u32
                ));
            }
            if let Some(previous) = previous {
                let key = u32::from(previous) << 16 | u32::from(glyph);
                if let Some(units) = self.metrics.kern.get(&key) {
                    pen += scaled_bound(*units, size, self.metrics.units_per_em);
                }
            }
            pen += self.advance_glyph(glyph, size);
            previous = Some(glyph);
        }
        Ok(pen)
    }

    /// Measures line advances, preserving D2's four-space tab behavior.
    pub fn measure(&self, text: &str, size: f64) -> TextMetrics {
        self.measure_mode(text, size, true)
    }

    /// Returns the cursor advance used by D2's SVG Markdown painter.
    ///
    /// This deliberately excludes ink bearings.  The Go implementation uses
    /// `font.MeasureString` when placing adjacent inline runs, while the
    /// legacy box measurement returned by [`Self::measure`] includes the
    /// rasterized ink rectangle.  Conflating the two leaves visible gaps (or
    /// shifts later runs) after bold and italic spans.
    pub fn measure_advance(&self, text: &str, size: f64) -> f64 {
        let mut pen: f64 = 0.0;
        let mut maximum: f64 = 0.0;
        let mut previous_glyph = None;
        let space = self.advance(' ', size);
        for character in text.chars() {
            match character {
                '\n' => {
                    maximum = maximum.max(pen);
                    pen = 0.0;
                    previous_glyph = None;
                }
                '\r' => {}
                '\t' => {
                    let tab = (space * 4.0).max(1.0);
                    pen = (pen / tab).floor().mul_add(tab, tab);
                    maximum = maximum.max(pen);
                    previous_glyph = None;
                }
                _ => {
                    let glyph = self
                        .metrics
                        .cmap
                        .get(&(character as u32))
                        .copied()
                        .unwrap_or(0);
                    if glyph == 0
                        && !character.is_ascii()
                        && !character.is_control()
                        && !matches!(character as u32, 0x200c..=0x200f | 0x202a..=0x202e | 0xfe00..=0xfe0f)
                    {
                        let codepoint = character as u32;
                        let wide = matches!(
                            codepoint,
                            0x1100..=0x11ff
                                | 0x2329..=0x232a
                                | 0x2e80..=0xa4cf
                                | 0xac00..=0xd7a3
                                | 0xf900..=0xfaff
                                | 0xfe10..=0xfe19
                                | 0xfe30..=0xfe6f
                                | 0xff00..=0xff60
                                | 0xffe0..=0xffe6
                                | 0x1f000..=0x1faff
                        );
                        if wide {
                            pen += size * 1.1875;
                            maximum = maximum.max(pen);
                            previous_glyph = Some(0);
                            continue;
                        }
                    }
                    pen += self.kern(previous_glyph, glyph, size);
                    pen += self.advance_glyph(glyph, size);
                    maximum = maximum.max(pen);
                    previous_glyph = Some(glyph);
                }
            }
        }
        maximum.max(pen)
    }

    /// Original inlineTextAdvance/scaleUnicodeCSS: supported text uses the
    /// original font advance; missing graphemes use the pinned CSS fallback.
    pub fn measure_markdown_css_advance(&self, text: &str, size: f64) -> f64 {
        let supported = |text: &str| {
            text.chars().all(|ch| {
                ch == '\0'
                    || self
                        .metrics
                        .cmap
                        .get(&(ch as u32))
                        .is_some_and(|glyph| *glyph != 0)
            })
        };
        if supported(text) {
            return self.measure_advance(text, size);
        }
        let ignorable = |ch: char| {
            let cp = ch as u32;
            let rows = markdown_ignorable_ranges::RANGES;
            let index = rows.partition_point(|&(start, _)| start <= cp);
            index > 0 && cp <= rows[index - 1].1
        };
        let original_width = |text: &str| {
            let mut pen = 0.0;
            let mut left = 0.0_f64;
            let mut right = 0.0_f64;
            let mut previous = None;
            for ch in text.chars() {
                let glyph = self.metrics.cmap.get(&(ch as u32)).copied().unwrap_or(0);
                pen += self.kern(previous, glyph, size);
                if let Some((xmin, xmax)) = self.metrics.outlines.bounds(glyph) {
                    left = left.min(pen + scaled_bound(xmin, size, self.metrics.units_per_em));
                    right = right.max(pen + scaled_bound(xmax, size, self.metrics.units_per_em));
                }
                pen += self.advance_glyph(glyph, size);
                previous = Some(glyph);
            }
            right.max(pen) - left
        };
        let mut maximum = 0.0_f64;
        for line in text.split('\n') {
            if supported(line) {
                maximum = maximum.max(original_width(line));
                continue;
            }
            let mut width = 0.0;
            let mut start = 0;
            for end in crate::uniseg::grapheme_boundaries(line) {
                let mut cluster = line[start..end].to_owned();
                start = end;
                if supported(&cluster) {
                    width += original_width(&cluster);
                    continue;
                }
                let color_emoji = crate::uniseg::is_color_emoji(&cluster);
                if !color_emoji {
                    let filtered: String = cluster.chars().filter(|ch| !ignorable(*ch)).collect();
                    if filtered != cluster {
                        if filtered.is_empty() {
                            continue;
                        }
                        if supported(&filtered) {
                            width += original_width(&filtered);
                            continue;
                        }
                        cluster = filtered;
                    }
                }
                if cluster
                    .chars()
                    .all(|ch| crate::uniseg::is_mark(ch) || ignorable(ch))
                {
                    continue;
                }
                let mut advance = size * crate::uniseg::string_width(&cluster).max(1) as f64 / 2.0;
                if color_emoji {
                    advance = advance.max(size * 1.25);
                } else if cluster.chars().any(crate::uniseg::is_symbol) {
                    advance = advance.max(size);
                }
                width += advance;
            }
            maximum = maximum.max(width);
        }
        maximum
    }

    /// Measures text with D2's Source Code Pro cell fallback for graphemes
    /// missing from the embedded atlas. The Go ruler first measures the
    /// replacement glyph, then replaces the rounded atlas ink bounds of wide
    /// graphemes with two monospace cells. Keeping that pass separate lets
    /// callers choose the exact fallback face while retaining the historical
    /// single-face API above.
    pub fn measure_with_fallback(
        &self,
        text: &str,
        size: f64,
        fallback: &TrueTypeFont,
    ) -> TextMetrics {
        let mut metrics = self.measure_mode(text, size, false);
        metrics.width = metrics
            .width
            .max(self.scale_unicode(text, size, fallback, false));
        metrics
    }

    fn measure_mode(&self, text: &str, size: f64, direct_wide_fallback: bool) -> TextMetrics {
        let mut width: f64 = 0.0;
        let mut pen: f64 = 0.0;
        // Ruler.bounds is a single rectangle for the complete string. A
        // newline resets the pen but does not reset its x extents, so lines
        // with different side bearings are measured relative to the same
        // origin rather than by taking an independent max width.
        let mut bounds_left: f64 = 0.0;
        let mut bounds_right: f64 = 0.0;
        let mut has_bounds = false;
        let mut max_line_advance: f64 = 0.0;
        let mut has_content = false;
        let mut lines: usize = 1;
        let mut previous_glyph = None;
        let space = self.advance(' ', size);
        for character in text.chars() {
            match character {
                '\n' => {
                    max_line_advance = max_line_advance.max(pen);
                    pen = 0.0;
                    lines += 1;
                }
                '\r' => {
                    pen = 0.0;
                }
                '\t' => {
                    let tab = (space * 4.0).max(1.0);
                    let remainder = pen % tab;
                    pen += if remainder == 0.0 { tab } else { remainder };
                    max_line_advance = max_line_advance.max(pen);
                }
                _ => {
                    if !character.is_whitespace() {
                        has_content = true;
                    }
                    // The Go ruler's atlas intentionally contains only ASCII,
                    // Latin-1 and geometric shapes. A font cmap hit outside
                    // those sets is still rendered through the replacement
                    // glyph, which is observable in legacy dimensions.
                    let glyph = if atlas_rune_supported(character) {
                        self.metrics
                            .cmap
                            .get(&(character as u32))
                            .copied()
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    // Source Sans Pro intentionally omits many East Asian
                    // and symbol glyphs. D2's ruler measures those through
                    // the host fallback font. Wide East Asian graphemes use
                    // two Source Code Pro cells; narrow symbols use one.
                    // Preserve those advances instead of using glyph 0 as a
                    // replacement box.
                    let variation_selector = matches!(
                        character as u32,
                        0x200c..=0x200f | 0x202a..=0x202e | 0xfe00..=0xfe0f
                    );
                    if glyph == 0
                        && !character.is_ascii()
                        && !character.is_control()
                        && !variation_selector
                    {
                        let codepoint = character as u32;
                        let wide = matches!(
                            codepoint,
                            0x1100..=0x11ff
                                | 0x2329..=0x232a
                                | 0x2e80..=0xa4cf
                                | 0xac00..=0xd7a3
                                | 0xf900..=0xfaff
                                | 0xfe10..=0xfe19
                                | 0xfe30..=0xfe6f
                                | 0xff00..=0xff60
                                | 0xffe0..=0xffe6
                                | 0x1f000..=0x1faff
                        );
                        if wide && direct_wide_fallback {
                            // D2's atlas contains only ASCII, Latin-1 and
                            // geometric-shape runes. Wider missing graphemes
                            // are corrected by scaleUnicode to two-cell
                            // fallback advances; width-one symbols continue
                            // through the replacement glyph below.
                            pen += size * 1.1875;
                            max_line_advance = max_line_advance.max(pen);
                            if !has_bounds {
                                bounds_left = 0.0;
                                bounds_right = pen;
                                has_bounds = true;
                            } else {
                                bounds_right = bounds_right.max(pen);
                            }
                            previous_glyph = Some(0);
                            continue;
                        }
                    }
                    pen += self.kern(previous_glyph, glyph, size);
                    if let Some((x_min, x_max)) = self.metrics.outlines.bounds(glyph) {
                        // x/image's atlas first scales the outline to 26.6
                        // fixed point and then floors/ceils each edge to a
                        // whole pixel before packing the glyph. Preserve
                        // that order; unioning fractional outline bounds
                        // directly produces narrower labels than D2.
                        let left =
                            pen + scaled_bound(x_min, size, self.metrics.units_per_em).floor();
                        let right =
                            pen + scaled_bound(x_max, size, self.metrics.units_per_em).ceil();
                        if !has_bounds {
                            bounds_left = left;
                            bounds_right = right;
                            has_bounds = true;
                        } else {
                            bounds_left = bounds_left.min(left);
                            bounds_right = bounds_right.max(right);
                        }
                    } else if has_bounds {
                        // Atlas DrawRune returns a zero-width rectangle at
                        // the current pen for spaces and other empty glyphs.
                        // Ruler.drawBuf still unions that point once another
                        // line has ink, so a leading space on a later line
                        // can move the global left edge back to the origin.
                        bounds_left = bounds_left.min(pen);
                        bounds_right = bounds_right.max(pen);
                    }
                    pen += self.advance_glyph(glyph, size);
                    max_line_advance = max_line_advance.max(pen);
                    previous_glyph = Some(glyph);
                }
            }
        }
        max_line_advance = max_line_advance.max(pen);
        let measured_width = if has_bounds {
            bounds_right - bounds_left
        } else if has_content {
            max_line_advance
        } else {
            0.0
        };
        width = width.max(measured_width);
        // D2's ruler advances ordinary text lines by the requested CSS font
        // size. The first line retains the font's ink height; each additional
        // line contributes one em rather than another full ascent+descent box.
        let height = self.ruler_height(text, size, false);
        TextMetrics {
            width,
            lines,
            height,
        }
    }

    // Go Ruler.drawBuf grows bounds only when drawing a glyph. Controls
    // move the pen; an ordinary empty rectangle is replaced by the first ink
    // rectangle, whereas boundsWithDot retains the initial origin.
    fn ruler_height(&self, text: &str, size: f64, with_dot: bool) -> f64 {
        // Metric-only font inputs have no outlines from which to compute
        // ink. Preserve their existing em-based estimate.
        if !self.metrics.outlines.has_ink() {
            return scaled_metric(self.metrics.ascent, size, self.metrics.units_per_em)
                + scaled_metric(
                    self.metrics.descent.saturating_abs(),
                    size,
                    self.metrics.units_per_em,
                )
                + size * text.matches('\n').count() as f64;
        }
        let ascent = scaled_metric(self.metrics.ascent, size, self.metrics.units_per_em);
        let descent = scaled_metric(
            self.metrics.descent.saturating_abs(),
            size,
            self.metrics.units_per_em,
        );
        let (mut dot_y, mut lower, mut upper) = (0.0f64, 0.0f64, 0.0f64);
        let mut has_ink = false;
        for c in text.chars() {
            match c {
                '\n' => dot_y -= size,
                '\r' | '\t' => {}
                _ => {
                    let glyph = if atlas_rune_supported(c) {
                        self.metrics.cmap.get(&(c as u32)).copied().unwrap_or(0)
                    } else {
                        0
                    };
                    let ink = self
                        .metrics
                        .outlines
                        .bounds(glyph)
                        .is_some_and(|(left, right)| left != right);
                    let (lo, hi) = if ink {
                        (dot_y - descent, dot_y + ascent)
                    } else {
                        (dot_y, dot_y)
                    };
                    if with_dot || has_ink {
                        lower = lower.min(lo);
                        upper = upper.max(hi);
                    } else {
                        lower = lo;
                        upper = hi;
                    }
                    has_ink |= ink;
                }
            }
        }
        upper - lower
    }

    /// Source d2graph.GetTextDimensions code branch: exact 1.3em ruler
    /// bounds, then separately rounded leading/trailing blank-line padding.
    pub fn measure_d2_code_label(
        &self,
        text: &str,
        size: f64,
        fallback: &TrueTypeFont,
    ) -> TextMetrics {
        let mut metrics = self.measure_markdown_code(text, size, 1.3);
        metrics.width = metrics
            .width
            .max(self.scale_unicode(text, size, fallback, true));
        let lines = text.split('\n').collect::<Vec<_>>();
        let trailing = lines
            .iter()
            .rev()
            .take_while(|line| line.trim().is_empty())
            .count();
        let leading = lines.first().is_some_and(|line| line.trim().is_empty());
        metrics.height = metrics.height.ceil();
        if leading && trailing < lines.len() {
            metrics.height += size;
        }
        metrics.height += (1.3 * size * trailing as f64).ceil();
        metrics
    }

    fn scale_unicode(&self, text: &str, size: f64, fallback: &TrueTypeFont, with_dot: bool) -> f64 {
        let mut width: f64 = 0.0;
        for line in text.split('\n') {
            let mut line_width = if with_dot {
                self.measure_with_dot_mode(line, size, false, false).width
            } else {
                self.measure_mode(line, size, false).width
            };
            for grapheme in line.graphemes(true) {
                let cells = grapheme.width();
                if cells == 1 {
                    continue;
                }
                // DrawRune rounds each replacement glyph's outline to atlas
                // pixels before ScaleUnicode subtracts it. This is subtly
                // wider than the fractional MeasurePrecise bounds.
                line_width -= self.atlas_grapheme_width_with_origin(grapheme, size, true);
                line_width += fallback.advance(' ', size) * cells as f64;
            }
            width = width.max(line_width);
        }
        width
    }

    fn atlas_grapheme_width_with_origin(&self, text: &str, size: f64, origin: bool) -> f64 {
        let mut previous = None;
        let mut pen = 0.0;
        let mut left = 0.0;
        let mut right = 0.0;
        let mut has_bounds = false;
        for character in text.chars() {
            if matches!(character, '\n' | '\r' | '\t') {
                continue;
            }
            let glyph = if atlas_rune_supported(character) {
                self.metrics
                    .cmap
                    .get(&(character as u32))
                    .copied()
                    .unwrap_or(0)
            } else {
                0
            };
            pen += self.kern(previous, glyph, size);
            if let Some((x_min, x_max)) = self.metrics.outlines.bounds(glyph) {
                let glyph_left = pen + scaled_bound(x_min, size, self.metrics.units_per_em).floor();
                // The Go atlas packs each glyph with one fixed-pixel padding
                // column. DrawRune's returned frame therefore has one more
                // column than the raw sfnt ink bounds used by MeasurePrecise.
                let glyph_right = pen
                    + scaled_bound(x_max, size, self.metrics.units_per_em).ceil()
                    + if origin { 0.0 } else { 1.0 };
                if !has_bounds {
                    left = glyph_left;
                    right = glyph_right;
                    has_bounds = true;
                } else {
                    left = left.min(glyph_left);
                    right = right.max(glyph_right);
                }
            }
            pen += self.advance_glyph(glyph, size);
            previous = Some(glyph);
        }
        if has_bounds {
            if origin {
                right.max(0.0) - left.min(0.0)
            } else {
                right - left
            }
        } else {
            0.0
        }
    }

    /// Measures text with D2's Markdown `boundsWithDot` behavior. The
    /// Markdown ruler includes the pen position after each line in the union,
    /// so a glyph whose ink ends before its advance still contributes its
    /// trailing side bearing to the block width.
    pub fn measure_with_dot(&self, text: &str, size: f64) -> TextMetrics {
        self.measure_with_dot_mode(text, size, true, false)
    }

    /// Original Markdown ruler bounds before Unicode correction. Code spans
    /// retain these raw bounds; ordinary labels use different fallback rules.
    pub fn measure_markdown_raw(&self, text: &str, size: f64) -> TextMetrics {
        self.measure_with_dot_mode(text, size, false, true)
    }

    // Original atlas_metrics.go packing, including the operation order used
    // when converting packed fixed-point coordinates back into ruler bounds.
    fn markdown_code_atlas(&self, size: f64) -> BTreeMap<u32, (u16, [f64; 6])> {
        let runes = std::iter::once(0xfffd)
            .chain(0..=0xff)
            .chain(0x25a0..=0x25ff);
        let glyphs: Vec<_> = runes
            .map(|r| {
                let glyph = self.metrics.cmap.get(&r).copied().unwrap_or(0);
                let frame = self
                    .glyph_outline_box(glyph)
                    .map(|(x0, y0, x1, y1)| {
                        [
                            scaled_bound(x0, size, self.metrics.units_per_em).floor() as i64 * 64,
                            scaled_bound(-y1, size, self.metrics.units_per_em).floor() as i64 * 64,
                            scaled_bound(x1, size, self.metrics.units_per_em).ceil() as i64 * 64,
                            scaled_bound(-y0, size, self.metrics.units_per_em).ceil() as i64 * 64,
                        ]
                    })
                    .unwrap_or([0; 4]);
                (r, glyph, frame)
            })
            .collect();
        let line_height = ((scaled_metric(self.metrics.ascent, size, self.metrics.units_per_em)
            + scaled_metric(
                self.metrics.descent.saturating_abs(),
                size,
                self.metrics.units_per_em,
            ))
            * 64.0) as i64;
        let arrange = |width: i64| {
            let mut dot = [0i64; 2];
            let mut bounds = [0i64; 4];
            let mut mapping = Vec::new();
            for &(r, glyph, original) in &glyphs {
                dot[0] -= original[0];
                let frame = [
                    original[0] + dot[0],
                    original[1] + dot[1],
                    original[2] + dot[0],
                    original[3] + dot[1],
                ];
                mapping.push((r, glyph, dot, frame));
                // fixed.Rectangle26_6.Union ignores empty rectangles.
                if frame[0] < frame[2] && frame[1] < frame[3] {
                    if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
                        bounds = frame;
                    } else {
                        bounds = [
                            bounds[0].min(frame[0]),
                            bounds[1].min(frame[1]),
                            bounds[2].max(frame[2]),
                            bounds[3].max(frame[3]),
                        ];
                    }
                }
                dot[0] = frame[2] + 128;
                dot[0] = (dot[0] + 63).div_euclid(64) * 64;
                if frame[2] >= width {
                    dot[0] = 0;
                    dot[1] = (dot[1] + line_height + 128 + 63).div_euclid(64) * 64;
                }
            }
            (bounds, mapping)
        };
        let mut low = 0i64;
        let mut high = 1024 * 1024 * 64;
        while low < high {
            let mid = low + (high - low) / 2;
            let (b, _) = arrange(mid);
            if b[2] - b[0] >= b[3] - b[1] {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        let (bounds, mapping) = arrange(low);
        let y_min = bounds[1] as f64 / 64.0;
        let y_max = bounds[3] as f64 / 64.0;
        mapping
            .into_iter()
            .map(|(r, glyph, dot, frame)| {
                let flip = |y: i64| y_max - (y as f64 / 64.0 - y_min);
                (
                    r,
                    (
                        glyph,
                        [
                            dot[0] as f64 / 64.0,
                            flip(dot[1]),
                            frame[0] as f64 / 64.0,
                            flip(frame[3]),
                            frame[2] as f64 / 64.0,
                            flip(frame[1]),
                        ],
                    ),
                )
            })
            .collect()
    }

    /// Original raw code pen/ink union, including packed atlas coordinates.
    /// Controls move the pen without growing bounds; a later glyph unions both
    /// its bounds and the pen into the rectangle initially at the origin.
    pub fn measure_markdown_code(&self, text: &str, size: f64, line_factor: f64) -> TextMetrics {
        let atlas = self.markdown_code_atlas(size);
        let ascent = scaled_metric(self.metrics.ascent, size, self.metrics.units_per_em);
        let descent = scaled_metric(
            self.metrics.descent.saturating_abs(),
            size,
            self.metrics.units_per_em,
        );
        let line_height = (size * 64.0 + 0.5) as i64 as f64 / 64.0;
        let mut dot_x = 0.0;
        let mut dot_y = 0.0;
        let mut left: f64 = 0.0;
        let mut right: f64 = 0.0;
        let mut lower: f64 = 0.0;
        let mut upper: f64 = 0.0;
        // Source normalizes even the initial previous rune to ReplacementChar.
        let mut previous = Some(atlas[&0xfffd].0);
        let tab_width = self.advance(' ', size) * 4.0;
        for character in text.chars() {
            match character {
                '\n' => {
                    dot_x = 0.0;
                    dot_y -= line_factor * line_height;
                }
                '\r' => dot_x = 0.0,
                '\t' => {
                    let rem = dot_x % tab_width;
                    let rem = rem % (rem + tab_width);
                    dot_x += if rem == 0.0 { tab_width } else { rem };
                }
                _ => {
                    let &(glyph, frame) = atlas.get(&(character as u32)).unwrap_or(&atlas[&0xfffd]);
                    dot_x += self.kern(previous, glyph, size);
                    let x = dot_x - frame[0];
                    let y = dot_y - frame[1];
                    let x0 = frame[2] + x;
                    let x1 = frame[4] + x;
                    let mut y0 = frame[3] + y;
                    let mut y1 = frame[5] + y;
                    if (x1 - x0) * (y1 - y0) != 0.0 {
                        y0 = dot_y - descent;
                        y1 = dot_y + ascent;
                    }
                    dot_x += self.advance_glyph(glyph, size);
                    previous = Some(glyph);
                    left = left.min(dot_x).min(x0);
                    right = right.max(dot_x).max(x1);
                    lower = lower.min(dot_y).min(y0);
                    upper = upper.max(dot_y).max(y1);
                }
            }
        }
        TextMetrics {
            width: right - left,
            height: upper - lower,
            lines: text.matches('\n').count() + 1,
        }
    }
    /// D2 scaleUnicodeLegacy after boundsWithDot measurement. Its correction
    /// starts from the actual pen/ink union and applies Source's control filter,
    /// retaining wide emoji clusters and the original raw-width floor.
    pub fn measure_markdown_legacy(
        &self,
        text: &str,
        size: f64,
        mono: &TrueTypeFont,
    ) -> TextMetrics {
        let mut metrics = self.measure_markdown_raw(text, size);
        if text.is_ascii() {
            return metrics;
        }
        let ignorable = |c: char| {
            let cp = c as u32;
            let ranges = markdown_ignorable_ranges::RANGES;
            let index = ranges.partition_point(|&(start, _)| start <= cp);
            index > 0 && cp <= ranges[index - 1].1
        };
        let mut filtered_text = String::new();
        let mut filtered = false;
        for cluster in text.graphemes(true) {
            if cluster.chars().all(ignorable) {
                filtered = true;
                continue;
            }
            if cluster.width() != 1 {
                filtered_text.push_str(cluster);
                continue;
            }
            for c in cluster.chars() {
                if ignorable(c) {
                    filtered = true;
                } else {
                    filtered_text.push(c);
                }
            }
        }
        let mut width = if filtered { 0.0 } else { metrics.width };
        for line in filtered_text.split('\n') {
            let floor = self.measure_markdown_raw(line, size).width;
            let mut line_width = floor;
            for cluster in line.graphemes(true) {
                let cells = cluster.width();
                if cells == 1 {
                    continue;
                }
                line_width -= self.atlas_grapheme_width_with_origin(cluster, size, true);
                line_width += mono.advance(' ', size) * cells as f64;
            }
            if filtered {
                line_width = line_width.max(floor);
            }
            width = width.max(line_width);
        }
        metrics.width = width;
        metrics
    }

    /// `measure_with_dot` with the same explicit fallback pass used by
    /// ordinary labels. Markdown's bounds-with-dot width still includes the
    /// line's trailing pen position after Unicode replacement.
    pub fn measure_with_dot_and_fallback(
        &self,
        text: &str,
        size: f64,
        fallback: &TrueTypeFont,
    ) -> TextMetrics {
        let mut metrics = self.measure_with_dot_mode(text, size, false, false);
        metrics.width = metrics
            .width
            .max(self.scale_unicode(text, size, fallback, true));
        metrics
    }

    fn measure_with_dot_mode(
        &self,
        text: &str,
        size: f64,
        direct_wide_fallback: bool,
        markdown_raw: bool,
    ) -> TextMetrics {
        let mut metrics = self.measure_mode(text, size, direct_wide_fallback);
        let space = self.advance(' ', size);
        let mut line_width: f64 = 0.0;
        let mut width: f64 = 0.0;
        // boundsWithDot starts from a zero-sized rectangle at the origin, so
        // the origin remains in the union even when the first glyph has a
        // positive left side bearing. Track the atlas ink union separately
        // from the pen advance used below.
        let mut ink_left = 0.0;
        let mut ink_right = 0.0;
        let mut has_ink = false;
        let mut previous_glyph = None;
        for character in text.chars() {
            match character {
                '\n' => {
                    width = width.max(line_width);
                    line_width = 0.0;
                }
                '\r' => {
                    width = width.max(line_width);
                    line_width = 0.0;
                }
                '\t' => {
                    let tab = (space * 4.0).max(1.0);
                    // Source controlRune applies the current remainder for
                    // both mono and Markdown bounds, not the next tab stop.
                    let remainder = line_width % tab;
                    let remainder = remainder % (remainder + tab);
                    line_width += if remainder == 0.0 { tab } else { remainder };
                }
                _ => {
                    let glyph = if atlas_rune_supported(character) {
                        self.metrics
                            .cmap
                            .get(&(character as u32))
                            .copied()
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    let variation_selector = matches!(
                        character as u32,
                        0x200c..=0x200f | 0x202a..=0x202e | 0xfe00..=0xfe0f
                    );
                    if glyph == 0
                        && !character.is_ascii()
                        && !character.is_control()
                        && !variation_selector
                    {
                        let codepoint = character as u32;
                        let wide = matches!(
                            codepoint,
                            0x1100..=0x11ff
                                | 0x2329..=0x232a
                                | 0x2e80..=0xa4cf
                                | 0xac00..=0xd7a3
                                | 0xf900..=0xfaff
                                | 0xfe10..=0xfe19
                                | 0xfe30..=0xfe6f
                                | 0xff00..=0xff60
                                | 0xffe0..=0xffe6
                                | 0x1f000..=0x1faff
                        );
                        if wide && direct_wide_fallback {
                            line_width += size * 1.1875;
                            previous_glyph = Some(0);
                        } else {
                            line_width += self.kern(previous_glyph, glyph, size);
                            if let Some((x_min, x_max)) = self.metrics.outlines.bounds(glyph) {
                                let left = line_width
                                    + scaled_bound(x_min, size, self.metrics.units_per_em).floor();
                                let right = line_width
                                    + scaled_bound(x_max, size, self.metrics.units_per_em).ceil();
                                if !has_ink {
                                    ink_left = left;
                                    ink_right = right;
                                    has_ink = true;
                                } else {
                                    ink_left = ink_left.min(left);
                                    ink_right = ink_right.max(right);
                                }
                            }
                            line_width += self.advance_glyph(glyph, size);
                            previous_glyph = Some(glyph);
                        }
                    } else {
                        line_width += self.kern(previous_glyph, glyph, size);
                        if let Some((x_min, x_max)) = self.metrics.outlines.bounds(glyph) {
                            let left = line_width
                                + scaled_bound(x_min, size, self.metrics.units_per_em).floor();
                            let right = line_width
                                + scaled_bound(x_max, size, self.metrics.units_per_em).ceil();
                            if !has_ink {
                                ink_left = left;
                                ink_right = right;
                                has_ink = true;
                            } else {
                                ink_left = ink_left.min(left);
                                ink_right = ink_right.max(right);
                            }
                        }
                        line_width += self.advance_glyph(glyph, size);
                        previous_glyph = Some(glyph);
                    }
                }
            }
        }
        width = width.max(line_width);
        if has_ink {
            // Ruler.drawBuf unions Dot and glyph bounds into one rectangle.
            // A negative left bearing must also extend the advance endpoint,
            // rather than taking the maximum of two independent widths.
            width = ink_right.max(width).max(0.0) - ink_left.min(0.0);
        }
        metrics.width = if markdown_raw {
            width
        } else {
            metrics.width.max(width)
        };
        metrics.height = self.ruler_height(text, size, true);
        metrics
    }
}

/// The Source Sans Pro Regular face bundled by D2 v0.9.0.
///
/// D2 uses this face when no custom font is supplied. Keeping the bytes in
/// the text crate makes the native CLI and future WASM bindings use the same
/// deterministic metrics without reaching into the reference checkout.
pub fn bundled_source_sans_pro_regular() -> Result<TrueTypeFont, FontError> {
    static FONT: OnceLock<Result<TrueTypeFont, FontError>> = OnceLock::new();
    FONT.get_or_init(|| TrueTypeFont::parse_static(bundled_source_sans_pro_regular_bytes()))
        .clone()
}

/// Shared metrics for a bundled face, using the same style fallbacks as
/// [`crate::d2fonts::font_face`]. Custom registered fonts remain separate.
pub fn bundled_face(
    family: crate::d2fonts::FontFamily,
    style: crate::d2fonts::FontStyle,
) -> Result<TrueTypeFont, FontError> {
    use crate::d2fonts::{FontFamily::*, FontStyle::*};
    match (family, style) {
        (SourceSansPro, Regular) => bundled_source_sans_pro_regular(),
        (SourceSansPro, Bold) => bundled_source_sans_pro_bold(),
        (SourceSansPro, Semibold) => bundled_source_sans_pro_semibold(),
        (SourceSansPro, Italic) => bundled_source_sans_pro_italic(),
        (SourceCodePro, Regular) => bundled_source_code_pro_regular(),
        (SourceCodePro, Bold) => bundled_source_code_pro_bold(),
        (SourceCodePro, Semibold) => bundled_source_code_pro_semibold(),
        (SourceCodePro, Italic) => bundled_source_code_pro_italic(),
        (HandDrawn, Regular | Italic) => bundled_fuzzy_bubbles_regular(),
        (HandDrawn, Bold | Semibold) => bundled_fuzzy_bubbles_bold(),
    }
}

#[cfg(test)]
mod lazy_font_contract {
    use super::*;

    #[test]
    fn lazy_bounds_match_eager_outlining_for_every_bundled_glyph() {
        let assets = [
            include_bytes!("../assets/SourceSansPro-Regular.ttf").as_slice(),
            include_bytes!("../assets/SourceSansPro-Bold.ttf").as_slice(),
            include_bytes!("../assets/SourceSansPro-Italic.ttf").as_slice(),
            include_bytes!("../assets/SourceSansPro-Semibold.ttf").as_slice(),
            include_bytes!("../assets/SourceCodePro-Regular.ttf").as_slice(),
            include_bytes!("../assets/SourceCodePro-Bold.ttf").as_slice(),
            include_bytes!("../assets/SourceCodePro-Italic.ttf").as_slice(),
            include_bytes!("../assets/SourceCodePro-Semibold.ttf").as_slice(),
            include_bytes!("../assets/FuzzyBubbles-Regular.ttf").as_slice(),
            include_bytes!("../assets/FuzzyBubbles-Bold.ttf").as_slice(),
        ];
        for data in assets {
            let font = TrueTypeFont::parse(data).unwrap();
            let face = ttf_parser::Face::parse(data, 0).unwrap();
            assert!(font.metrics.outlines.boxes.get().is_none());
            for glyph in 0..face.number_of_glyphs() {
                let expected = face.glyph_bounding_box(ttf_parser::GlyphId(glyph));
                assert_eq!(
                    font.glyph_outline_box(glyph),
                    expected.map(|b| (b.x_min, b.y_min, b.x_max, b.y_max)),
                    "glyph {glyph}"
                );
                assert_eq!(
                    font.metrics.outlines.bounds(glyph),
                    expected.and_then(|b| (b.x_min < b.x_max).then_some((b.x_min, b.x_max)))
                );
            }
            assert_eq!(font.glyph_outline_box(u16::MAX), None);
        }
    }

    #[test]
    fn owned_input_and_shared_fonts_support_concurrent_measurement() {
        // The source buffer may be dropped before an outline is requested.
        let font = {
            let bytes = bundled_source_sans_pro_regular_bytes().to_vec();
            TrueTypeFont::parse(&bytes).unwrap()
        };
        let glyph = font.glyph_index('Å');
        assert!(font.glyph_outline_box(glyph).is_some());
        assert_eq!(
            font.metrics
                .outlines
                .boxes
                .get()
                .unwrap()
                .iter()
                .filter(|b| b.get().is_some())
                .count(),
            1
        );

        let first = bundled_source_sans_pro_regular().unwrap();
        let expected = first.measure("Åglyph\ntext", 16.0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let first = &first;
                scope.spawn(move || {
                    let next = bundled_source_sans_pro_regular().unwrap();
                    assert!(Arc::ptr_eq(&first.metrics, &next.metrics));
                    assert_eq!(next.measure("Åglyph\ntext", 16.0), expected);
                });
            }
        });
    }
}

/// Original D2 v0.9.0 Source Sans Pro regular font data for embedders that
/// need to preserve the font in a packaged document.
pub fn bundled_source_sans_pro_regular_bytes() -> &'static [u8] {
    include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/SourceSansPro-Regular.ttf"
    ))
}

/// The Fuzzy Bubbles face D2 v0.9.0 selects for sketch mode. It is bundled by
/// the upstream release under the `HandDrawn` family name and changes label
/// metrics before the sketch overlay is rendered.
pub fn bundled_fuzzy_bubbles_regular() -> Result<TrueTypeFont, FontError> {
    static FONT: OnceLock<Result<TrueTypeFont, FontError>> = OnceLock::new();
    FONT.get_or_init(|| {
        TrueTypeFont::parse_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/FuzzyBubbles-Regular.ttf"
        )))
    })
    .clone()
}

/// Bold Fuzzy Bubbles face used by bold sketch labels.
pub fn bundled_fuzzy_bubbles_bold() -> Result<TrueTypeFont, FontError> {
    static FONT: OnceLock<Result<TrueTypeFont, FontError>> = OnceLock::new();
    FONT.get_or_init(|| {
        TrueTypeFont::parse_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/FuzzyBubbles-Bold.ttf"
        )))
    })
    .clone()
}

/// The Source Sans Pro Bold face bundled by D2 v0.9.0.
pub fn bundled_source_sans_pro_bold() -> Result<TrueTypeFont, FontError> {
    static FONT: OnceLock<Result<TrueTypeFont, FontError>> = OnceLock::new();
    FONT.get_or_init(|| TrueTypeFont::parse_static(bundled_source_sans_pro_bold_bytes()))
        .clone()
}

/// Original D2 v0.9.0 Source Sans Pro bold font data for embedders that need
/// to preserve the font in a packaged document.
pub fn bundled_source_sans_pro_bold_bytes() -> &'static [u8] {
    include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/SourceSansPro-Bold.ttf"
    ))
}

/// The Source Sans Pro Semibold face used by Markdown headings and table
/// headers in D2's GitHub-style measurement path.
pub fn bundled_source_sans_pro_semibold() -> Result<TrueTypeFont, FontError> {
    static FONT: OnceLock<Result<TrueTypeFont, FontError>> = OnceLock::new();
    FONT.get_or_init(|| {
        TrueTypeFont::parse_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/SourceSansPro-Semibold.ttf"
        )))
    })
    .clone()
}

/// The Source Sans Pro Italic face used by D2 edge labels and italic text.
pub fn bundled_source_sans_pro_italic() -> Result<TrueTypeFont, FontError> {
    static FONT: OnceLock<Result<TrueTypeFont, FontError>> = OnceLock::new();
    FONT.get_or_init(|| {
        TrueTypeFont::parse_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/SourceSansPro-Italic.ttf"
        )))
    })
    .clone()
}

/// The Source Code Pro face used for D2 `code` labels and class members.
pub fn bundled_source_code_pro_regular() -> Result<TrueTypeFont, FontError> {
    static FONT: OnceLock<Result<TrueTypeFont, FontError>> = OnceLock::new();
    FONT.get_or_init(|| {
        TrueTypeFont::parse_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/SourceCodePro-Regular.ttf"
        )))
    })
    .clone()
}

/// The Source Code Pro Bold face used when a diagram explicitly requests the
/// monospace family together with `style.bold`.
pub fn bundled_source_code_pro_bold() -> Result<TrueTypeFont, FontError> {
    static FONT: OnceLock<Result<TrueTypeFont, FontError>> = OnceLock::new();
    FONT.get_or_init(|| {
        TrueTypeFont::parse_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/SourceCodePro-Bold.ttf"
        )))
    })
    .clone()
}

/// The Source Code Pro Italic face used by italic monospace labels.
pub fn bundled_source_code_pro_italic() -> Result<TrueTypeFont, FontError> {
    static FONT: OnceLock<Result<TrueTypeFont, FontError>> = OnceLock::new();
    FONT.get_or_init(|| {
        TrueTypeFont::parse_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/SourceCodePro-Italic.ttf"
        )))
    })
    .clone()
}

#[cfg(test)]
mod markdown_legacy_source_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn raw_and_unicode_corrected_markdown_widths_match_pristine_bits() {
        let rows: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/markdown-legacy-source-v090.json"))
                .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let mono_family = row["family"] == "SourceCodePro";
            let style = row["style"].as_str().unwrap();
            let mono = match style {
                "bold" => bundled_source_code_pro_bold(),
                "italic" => bundled_source_code_pro_italic(),
                _ => bundled_source_code_pro_regular(),
            }
            .unwrap();
            let primary = if mono_family {
                mono.clone()
            } else {
                match style {
                    "bold" => bundled_source_sans_pro_bold(),
                    "italic" => bundled_source_sans_pro_italic(),
                    _ => bundled_source_sans_pro_regular(),
                }
                .unwrap()
            };
            let text = row["text"].as_str().unwrap();
            let size = row["size"].as_f64().unwrap();
            let raw = primary.measure_markdown_raw(text, size).width;
            let legacy = primary.measure_markdown_legacy(text, size, &mono).width;
            if raw.to_bits() != row["raw_width_bits"].as_u64().unwrap()
                || legacy.to_bits() != row["legacy_width_bits"].as_u64().unwrap()
            {
                differences.push(serde_json::json!({"family":row["family"],"style":style,"size":size,"text":text,"raw":raw,"expected_raw":row["raw_width"],"legacy":legacy,"expected_legacy":row["legacy_width"]}));
            }
        }
        assert!(
            differences.is_empty(),
            "Source raw/legacy width differences: {differences:#?}"
        );
    }
}

#[cfg(test)]
mod ordinary_width_source_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn ordinary_widths_match_pristine_bits() {
        let rows: serde_json::Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/ordinary-width-source-v090.json")).unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let mono_family = row["family"] == "SourceCodePro";
            let style = row["style"].as_str().unwrap();
            let mono = match style {
                "bold" => bundled_source_code_pro_bold(),
                "italic" => bundled_source_code_pro_italic(),
                _ => bundled_source_code_pro_regular(),
            }
            .unwrap();
            let primary = if mono_family {
                mono.clone()
            } else {
                match style {
                    "bold" => bundled_source_sans_pro_bold(),
                    "italic" => bundled_source_sans_pro_italic(),
                    _ => bundled_source_sans_pro_regular(),
                }
                .unwrap()
            };
            let text = row["text"].as_str().unwrap();
            let size = row["size"].as_f64().unwrap();
            let raw = primary.measure_mode(text, size, false).width;
            let legacy = primary.measure_with_fallback(text, size, &mono).width;
            if raw.to_bits() != row["raw_width_bits"].as_u64().unwrap()
                || legacy.to_bits() != row["legacy_width_bits"].as_u64().unwrap()
            {
                differences.push(serde_json::json!({"family":row["family"],"style":style,"size":size,"text":text,"raw":raw,"expected_raw":row["raw_width"],"legacy":legacy,"expected_legacy":row["legacy_width"]}));
            }
        }
        assert!(
            differences.is_empty(),
            "Source raw/legacy width differences: {differences:#?}"
        );
    }
}

/// Bundled Source Code Pro Semibold used by Markdown monospace headings.
pub fn bundled_source_code_pro_semibold() -> Result<TrueTypeFont, FontError> {
    static FONT: OnceLock<Result<TrueTypeFont, FontError>> = OnceLock::new();
    FONT.get_or_init(|| {
        TrueTypeFont::parse_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/SourceCodePro-Semibold.ttf"
        )))
    })
    .clone()
}

#[cfg(test)]
mod code_glyph_source_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_code_ruler_retains_dot_bounds_during_unicode_correction() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/code-ruler-unicode-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 48);
        let font = bundled_source_code_pro_regular().unwrap();
        for row in rows.as_array().unwrap() {
            let size = row["size"].as_f64().unwrap();
            let metrics =
                font.measure_with_dot_and_fallback(row["text"].as_str().unwrap(), size, &font);
            assert_eq!(
                metrics.width.ceil() as i64,
                row["width"].as_i64().unwrap(),
                "{row}"
            );
            assert_eq!(
                (metrics.height + size * 0.3 * metrics.lines.saturating_sub(1) as f64).ceil()
                    as i64,
                row["height"].as_i64().unwrap(),
                "{row}"
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_code_metrics_errors_offsets_fixed_advances_and_context_causes() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/code-glyph-metrics-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 312);
        let regular = bundled_source_code_pro_regular().unwrap();
        let bold = bundled_source_code_pro_bold().unwrap();
        let italic = bundled_source_code_pro_italic().unwrap();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let face = match r["font"].as_str().unwrap() {
                "bold" => &bold,
                "italic" => &italic,
                _ => &regular,
            };
            let token = d2_rs_core::CancellationToken::new();
            match r["cancel"].as_str().unwrap() {
                "before" => token.cancel(),
                "deadline_before" => token.expire(),
                _ => {}
            }
            let actual = face.measure_code_advance_checked(
                r["text"].as_str().unwrap(),
                r["size"].as_f64().unwrap(),
                Some(&token),
            );
            let error = actual.as_ref().err().cloned().unwrap_or_default();
            assert_eq!(error, row["error"].as_str().unwrap(), "{r}");
            if let Ok(advance) = actual {
                assert_eq!(
                    advance.to_bits(),
                    row["advance_bits"].as_u64().unwrap(),
                    "{r}"
                );
            }
        }
    }
}
