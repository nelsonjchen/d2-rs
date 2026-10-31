// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2016 The Go Authors. All rights reserved.
// Rust translation of golang.org/x/image v0.45.0 font/sfnt (sfnt.go,
// cmap.go, gpos.go, truetype.go) for D2's fontface package. The CFF/Type 2
// charstring interpreter (postscript.go) and dfont resources are not ported;
// such sources are rejected with explicit errors instead of approximated.

mod cmap;
mod gpos;
mod truetype;

use crate::fixed::{Int26_6, Point26_6, Rectangle26_6};
use std::fmt;
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
    // Not a Go error value: this translation omits postscript.go and the
    // dfont resource parser rather than approximating them.
    ERR_UNPORTED_CFF = "sfnt: CFF/PostScript outlines are not supported by the d2-rs translation";
    ERR_UNPORTED_DFONT = "sfnt: dfont resources are not supported by the d2-rs translation";
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
}

/// `sfnt.ParseCollection` (and `opentype.ParseCollection`).
pub fn parse_collection(src: Arc<[u8]>) -> Result<Collection, Error> {
    let src = Source { b: src };
    let buf = src.view(0, 16)?;
    let offsets = match u32_at(buf, 0) {
        DFONT_RESOURCE_DATA_OFFSET => return Err(ERR_UNPORTED_DFONT),
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
    Ok(Collection { src, offsets })
}

impl Collection {
    /// `Collection.NumFonts`.
    pub fn num_fonts(&self) -> usize {
        self.offsets.len()
    }

    /// `Collection.Font`.
    pub fn font(&self, index: usize) -> Result<Font, Error> {
        let Some(&offset) = self.offsets.get(index) else {
            return Err(Error::NotFound);
        };
        Font::initialize(self.src.clone(), offset as i64)
    }
}

/// `sfnt.Parse`.
pub fn parse(src: Arc<[u8]>) -> Result<Font, Error> {
    Font::initialize(Source { b: src }, 0)
}

pub(crate) type KernFunc = Box<dyn Fn(GlyphIndex, GlyphIndex) -> Result<i16, Error> + Send + Sync>;

/// `sfnt.Font` for glyf-flavored, in-memory fonts. Like Go's, it has no
/// mutable state and may be shared between concurrent renders.
pub struct Font {
    src: Source,
    hmtx: Table,
    glyf_locations: Vec<u32>,
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
    fn initialize(src: Source, offset: i64) -> Result<Font, Error> {
        let (tables, is_post_script) = initialize_tables(&src, offset)?;
        if is_post_script {
            // postscript.go's cffParser and Type 2 charstrings are not
            // translated; refuse instead of approximating CFF outlines.
            let _ = tables.cff;
            return Err(ERR_UNPORTED_CFF);
        }
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
        if tables.maxp.length != 32 {
            return Err(ERR_INVALID_MAXP_TABLE);
        }
        let num_glyphs = i32::from(src.u16(tables.maxp, 4)?);
        // parseGlyphData
        let mut is_color_bitmap = false;
        let mut locations = Vec::new();
        if tables.loca.length != 0 {
            locations = truetype::parse_loca(
                &src,
                tables.loca,
                tables.glyf.offset,
                index_to_loc_format,
                num_glyphs,
            )?;
        } else if tables.cblc.length != 0 {
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
        let mut segments = Vec::new();
        let mut stack = [truetype::CompoundElem::default(); MAX_COMPOUND_STACK_SIZE as usize];
        truetype::load_glyf(self, &mut segments, &mut stack, x, 0, 0)?;
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

fn initialize_tables(src: &Source, offset: i64) -> Result<(Tables, bool), Error> {
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
        let (o, n) = (u32_at(entry, 8), u32_at(entry, 12));
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
