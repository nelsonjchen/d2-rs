// SPDX-License-Identifier: MPL-2.0
//! Source d2renderers/internal/fontface/face.go: dual-parser faces with
//! parser-issued provenance, renderable-rune coverage, COLRv0 layers and
//! glyph paint bounds.
//!
//! Go pairs an `sfnt.Font` (outlines) with a go-text `font.Face` (shaping).
//! Here the outline reader is this crate's `sfnt` translation and the shaping
//! face wraps the vendored go-v090 rustybuzz/ttf-parser parse of the same
//! bytes. Provenance is a private field, so a face can only carry it when
//! this crate's parser or registry issued it (Go's pointer-identity checks).

use crate::colr::{ColorRecord, Colr, ColrPaint, cpal_palette0};
use crate::colrv1::{COLRv1ClipBox, COLRv1Plan};
use crate::error::Error;
use crate::fixed::{Int26_6, Point26_6, Rectangle26_6};
use crate::sfnt;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const MAX_PARSED_FONT_FACES: usize = 256;
const MAX_COLR0_GLYPH_LAYERS: usize = 1_024;

/// A SHA-256 digest (`[sha256.Size]byte`).
pub type Digest32 = [u8; 32];

pub(crate) fn sha256(data: &[u8]) -> Digest32 {
    Sha256::digest(data).into()
}

/// `color.NRGBA`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Nrgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Go's `%g` for a float64 (shortest representation; exponent form when the
/// decimal exponent is < -4 or >= 21).
pub(crate) fn go_format_g(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "+Inf".into()
        } else {
            "-Inf".into()
        };
    }
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    if !(-4..21).contains(&exponent) {
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", exponent.abs())
    } else {
        format!("{value}")
    }
}

static NEXT_SHAPING_FACE_ID: AtomicU64 = AtomicU64::new(1);

/// The read-only shaping font (go-text `font.Font`), shared by clones.
pub struct ShapingFont {
    // Field order matters: `face` borrows `data` and must drop first.
    face: rustybuzz::Face<'static>,
    data: Arc<[u8]>,
    index: u32,
    colr: Option<(usize, usize)>,
    palette0: Option<Vec<ColorRecord>>,
}

impl std::fmt::Debug for ShapingFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShapingFont")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

fn table_range(data: &[u8], face: &ttf_parser::Face<'_>, tag: &[u8; 4]) -> Option<(usize, usize)> {
    let table = face.raw_face().table(ttf_parser::Tag::from_bytes(tag))?;
    let start = table.as_ptr() as usize - data.as_ptr() as usize;
    Some((start, start + table.len()))
}

impl ShapingFont {
    fn parse(data: Arc<[u8]>, index: u32) -> Result<ShapingFont, Error> {
        Self::parse_at(data, index, 0, index)
    }

    fn parse_at(
        data: Arc<[u8]>,
        index: u32,
        offset: usize,
        parser_index: u32,
    ) -> Result<ShapingFont, Error> {
        // SAFETY: `data` is an immutable shared allocation owned by this
        // struct; `face` is declared first, so it drops before `data`, and
        // the borrowed slice never escapes with a longer lifetime.
        let bytes: &'static [u8] = unsafe { &*Arc::as_ptr(&data) };
        let source = bytes
            .get(offset..)
            .ok_or_else(|| Error::msg(format!("failed to parse font face {index}")))?;
        let face = rustybuzz::Face::from_slice(source, parser_index)
            .ok_or_else(|| Error::msg(format!("failed to parse font face {index}")))?;
        let colr = table_range(bytes, &face, b"COLR");
        let palette0 = face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"CPAL"))
            .and_then(cpal_palette0);
        Ok(ShapingFont {
            face,
            data,
            index,
            colr,
            palette0,
        })
    }

    /// The go-v090 rustybuzz face used for HarfBuzz-compatible shaping.
    pub fn rustybuzz_face(&self) -> &rustybuzz::Face<'_> {
        &self.face
    }

    /// The shared font bytes and face index.
    pub fn data(&self) -> (&Arc<[u8]>, u32) {
        (&self.data, self.index)
    }

    /// `Font.NominalGlyph`.
    pub fn nominal_glyph(&self, value: char) -> Option<u32> {
        self.face.glyph_index(value).map(|glyph| u32::from(glyph.0))
    }

    pub(crate) fn colr(&self) -> Option<Colr<'_>> {
        let (start, end) = self.colr?;
        Colr::new(&self.data[start..end])
    }

    pub(crate) fn cpal_palette0(&self) -> Option<Vec<ColorRecord>> {
        self.palette0.clone()
    }

    pub(crate) fn palette0(&self) -> Option<&[ColorRecord]> {
        self.palette0.as_deref()
    }

    /// `Font.IsMonospace` (go-text metadata.go, adapted from fontconfig).
    pub fn is_monospace(&self) -> bool {
        let face: &ttf_parser::Face<'_> = &self.face;
        if face.is_monospaced() {
            return true;
        }
        let Some(hmtx) = face.raw_face().table(ttf_parser::Tag::from_bytes(b"hmtx")) else {
            return false;
        };
        let metrics = face.tables().hhea.number_of_metrics as usize;
        if hmtx.is_empty() || metrics == 0 {
            return false;
        }
        if metrics == 1 {
            return true;
        }
        let abs = |v: i64| v.abs();
        let mut first_advance = 0i64;
        for gid in 1..metrics {
            let Some(bytes) = hmtx.get(gid * 4..gid * 4 + 2) else {
                break;
            };
            let advance = i64::from(u16::from_be_bytes([bytes[0], bytes[1]]));
            if advance == 0 {
                continue;
            }
            if first_advance == 0 {
                first_advance = advance;
                continue;
            }
            if abs(advance - first_advance) * 33 <= abs(advance).max(abs(first_advance)) {
                continue;
            }
            return false;
        }
        true
    }
}

/// A go-text `font.Face`: a shaping font plus per-face identity. Go keys
/// shaping caches and duplicate detection by the Face pointer; here each
/// parsed face and clone receives a fresh identity.
#[derive(Debug)]
pub struct ShapingFace {
    font: Arc<ShapingFont>,
    id: u64,
}

impl ShapingFace {
    fn new(font: Arc<ShapingFont>) -> ShapingFace {
        ShapingFace {
            font,
            id: NEXT_SHAPING_FACE_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// The shared read-only shaping font (`Face.Font`).
    pub fn font(&self) -> &Arc<ShapingFont> {
        &self.font
    }

    /// Identity used where Go compares `*font.Face` pointers.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// `Face.NominalGlyph`.
    pub fn nominal_glyph(&self, value: char) -> Option<u32> {
        self.font.nominal_glyph(value)
    }

    /// `glyphDataKind(face.GlyphData(gid))`: "color", "bitmap", "SVG",
    /// "outline" or "missing".
    pub fn glyph_data_kind(&self, glyph_id: u32) -> &'static str {
        let font = &self.font;
        let Ok(gid) = u16::try_from(glyph_id) else {
            return "missing";
        };
        if let Some(colr) = font.colr()
            && matches!(colr.search(gid), Ok(Some(_)))
        {
            return "color";
        }
        let face: &ttf_parser::Face<'_> = &font.face;
        let glyph = ttf_parser::GlyphId(gid);
        if face.glyph_raster_image(glyph, u16::MAX).is_some() {
            return "bitmap";
        }
        if face.glyph_svg_image(glyph).is_some() {
            return "SVG";
        }
        let has_outlines = face.tables().glyf.is_some()
            || face.tables().cff.is_some()
            || face.tables().cff2.is_some();
        if has_outlines && gid < face.number_of_glyphs() {
            return "outline";
        }
        "missing"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParsedFaceSource {
    digest: Digest32,
    bundled_outline: bool,
}

/// `ParsedFace`: one matching outline/shaping face. Its shaping identity must
/// not be shared across concurrent renders; use `clone` for another render.
#[derive(Debug)]
pub struct ParsedFace {
    outline: Arc<sfnt::Font>,
    shaping: ShapingFace,
    source: Option<ParsedFaceSource>,
}

/// `ColorGlyphLayer`: one COLRv0 layer in paint order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColorGlyphLayer {
    pub glyph_id: u32,
    pub color: Nrgba,
    pub foreground: bool,
}

/// `Collection`: one source parsed by both libraries with agreeing topology.
pub struct Collection {
    data: Arc<[u8]>,
    outline: sfnt::Collection,
    shaping: Vec<Arc<ShapingFont>>,
    digest: Digest32,
}

/// `ParseFaceCollectionWithLimit`.
pub fn parse_face_collection_with_limit(
    data: &[u8],
    max_faces: usize,
) -> Result<Collection, Error> {
    let max_faces = max_faces.min(MAX_PARSED_FONT_FACES);
    let mut collection = parse_face_collection_with_digest(Arc::from(data), max_faces, [0; 32])?;
    if data.len() == crate::noto_color_emoji::BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SIZE {
        collection.digest = sha256(data);
    }
    Ok(collection)
}

pub(crate) fn parse_face_collection_with_digest(
    data: Arc<[u8]>,
    max_faces: usize,
    digest: Digest32,
) -> Result<Collection, Error> {
    if data.is_empty() {
        return Err(Error::msg("empty font data"));
    }
    if max_faces == 0 {
        return Err(Error::msg("font face limit must be positive"));
    }
    let outline = sfnt::parse_collection(data.clone())
        .map_err(|error| Error::wrap("parse TrueType/OpenType font", error))?;
    if outline.num_fonts() > max_faces {
        return Err(Error::FaceCountLimit {
            count: outline.num_fonts(),
            limit: max_faces,
        });
    }
    let dfont_offsets = outline.dfont_offsets();
    let count = dfont_offsets.map_or_else(
        || ttf_parser::fonts_in_collection(&data).unwrap_or(1) as usize,
        |offsets| offsets.len(),
    );
    let mut shaping = Vec::with_capacity(count);
    for index in 0..count {
        let font = if let Some(offsets) = dfont_offsets {
            ShapingFont::parse_at(data.clone(), index as u32, offsets[index] as usize, 0)
        } else {
            ShapingFont::parse(data.clone(), index as u32)
        }
        .map_err(|error| Error::wrap("parse shaping font", error))?;
        shaping.push(Arc::new(font));
    }
    if outline.num_fonts() != shaping.len() {
        return Err(Error::msg(format!(
            "font parser face-count mismatch: outline={} shaping={}",
            outline.num_fonts(),
            shaping.len()
        )));
    }
    Ok(Collection {
        data,
        outline,
        shaping,
        digest,
    })
}

impl Collection {
    /// `Collection.NumFaces`.
    pub fn num_faces(&self) -> usize {
        self.shaping.len()
    }

    /// The collection's source bytes.
    pub fn data(&self) -> &Arc<[u8]> {
        &self.data
    }

    /// `Collection.Face`.
    pub fn face(&self, index: usize) -> Result<ParsedFace, Error> {
        if index >= self.num_faces() {
            return Err(Error::msg(format!(
                "load font face {index}: collection has {} faces",
                self.num_faces()
            )));
        }
        let outline = self
            .outline
            .font(index)
            .map_err(|error| Error::wrap(format!("load outline font face {index}"), error))?;
        Ok(ParsedFace {
            outline: Arc::new(outline),
            shaping: ShapingFace::new(self.shaping[index].clone()),
            source: Some(ParsedFaceSource {
                digest: self.digest,
                bundled_outline: false,
            }),
        })
    }
}

/// `ParseFace`: one face from a TrueType, OpenType or collection resource.
pub fn parse_face(data: &[u8], face_index: u16) -> Result<ParsedFace, Error> {
    let collection = parse_face_collection_with_limit(data, MAX_PARSED_FONT_FACES)?;
    let face = collection.face(usize::from(face_index))?;
    if face.is_bundled_noto_color_emoji() {
        crate::noto_color_emoji::register_authenticated_bundled_noto_color_emoji(
            collection.data.clone(),
            collection.digest,
        )?;
    }
    Ok(face)
}

impl ParsedFace {
    /// The go-text metadata used to rank system fallback faces.
    pub fn fallback_description(&self) -> crate::FallbackFaceDescription {
        crate::metadata::fallback_face_description(self.shaping.font().rustybuzz_face())
    }
    /// The immutable sfnt outline reader (`ParsedFace.Outline`).
    pub fn outline(&self) -> &sfnt::Font {
        &self.outline
    }

    /// The shared outline reader.
    pub fn outline_arc(&self) -> &Arc<sfnt::Font> {
        &self.outline
    }

    /// The shaping face (`ParsedFace.Shaping`).
    pub fn shaping(&self) -> &ShapingFace {
        &self.shaping
    }

    pub(crate) fn set_bundled_outline(&mut self) {
        if let Some(source) = &mut self.source {
            source.bundled_outline = true;
        }
    }

    /// `Clone`: fresh shaping caches (a new shaping identity) sharing the
    /// read-only parsed tables and this face's provenance.
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> Result<ParsedFace, Error> {
        Ok(ParsedFace {
            outline: self.outline.clone(),
            shaping: ShapingFace::new(self.shaping.font.clone()),
            source: self.source,
        })
    }

    /// `CloneInto`.
    pub fn clone_into(&self, clone: &mut ParsedFace) -> Result<(), Error> {
        *clone = self.clone()?;
        Ok(())
    }

    pub(crate) fn has_parsed_source_components(&self) -> bool {
        self.source.is_some()
    }

    pub(crate) fn has_parsed_source(&self, digest: &Digest32) -> bool {
        *digest != [0; 32] && self.source.is_some_and(|source| source.digest == *digest)
    }

    fn bundled_outline(&self) -> bool {
        self.source.is_some_and(|source| source.bundled_outline)
    }

    /// `IsBundledNotoColorEmoji`.
    pub fn is_bundled_noto_color_emoji(&self) -> bool {
        self.has_parsed_source(&crate::noto_color_emoji::BUNDLED_NOTO_COLOR_EMOJI_COLRV1_SHA256)
    }

    fn outline_glyph(&self, value: char) -> Option<u32> {
        self.outline.glyph_index(value).ok().map(u32::from)
    }

    /// `SupportsRenderableRune`.
    pub fn supports_renderable_rune(&self, value: char) -> Result<bool, Error> {
        if self.has_parsed_source_components() && self.bundled_outline() {
            let Some(shaping_glyph) = self.shaping.font.nominal_glyph(value).filter(|&g| g != 0)
            else {
                return Ok(false);
            };
            return Ok(self
                .outline_glyph(value)
                .is_some_and(|glyph| glyph != 0 && glyph == shaping_glyph));
        }
        let Some(shaping_glyph) = self.shaping.nominal_glyph(value).filter(|&g| g != 0) else {
            return Ok(false);
        };
        let Some(outline_glyph) = self
            .outline_glyph(value)
            .filter(|&g| g != 0 && g == shaping_glyph)
        else {
            return Ok(false);
        };
        if let Some(plan) = self.trusted_colrv1_plan(outline_glyph)? {
            return Ok(plan_has_clip(&plan));
        }
        supports_renderable_rune(self, value)
    }

    /// `trustedCOLRv1Plan`: Ok(None) both for an untrusted face and for a
    /// trusted glyph without COLR paint (Go's `trusted == false` / nil plan).
    fn trusted_colrv1_plan(&self, glyph_id: u32) -> Result<Option<COLRv1Plan>, Error> {
        if !self.is_bundled_noto_color_emoji() {
            return Ok(None);
        }
        self.compile_bundled_noto_color_emoji_colrv1_plan(glyph_id)
    }

    /// `COLR0GlyphLayers`: Ok(None) means the glyph uses its ordinary outline.
    pub fn colr0_glyph_layers(&self, glyph_id: u32) -> Result<Option<Vec<ColorGlyphLayer>>, Error> {
        if glyph_id == 0
            || glyph_id > u32::from(u16::MAX)
            || glyph_id as usize >= self.outline.num_glyphs()
        {
            return Err(Error::msg(format!("glyph ID {glyph_id} is out of range")));
        }
        let Some(colr) = self.shaping.font.colr() else {
            return Ok(None);
        };
        let Some(paint) = colr.search(glyph_id as u16)? else {
            return Ok(None);
        };
        self.resolve_color_paint(glyph_id, &colr, paint)
    }

    fn resolve_color_paint(
        &self,
        glyph_id: u32,
        colr: &Colr<'_>,
        paint: ColrPaint,
    ) -> Result<Option<Vec<ColorGlyphLayer>>, Error> {
        let ColrPaint::V0Layers { first, count } = paint else {
            let segments = self
                .outline
                .load_glyph(glyph_id as u16, crate::fixed::i(1_000))
                .map_err(|error| {
                    Error::wrap(
                        format!("load fallback outline for color glyph {glyph_id}"),
                        error,
                    )
                })?;
            if segments.is_empty() {
                return Err(Error::msg(format!(
                    "color glyph {glyph_id} uses unsupported COLRv1 paint without an outline fallback"
                )));
            }
            return Ok(None);
        };
        if count == 0 {
            return Err(Error::msg(format!("COLRv0 glyph {glyph_id} has no layers")));
        }
        let Some(palette) = self.shaping.font.palette0() else {
            return Err(Error::msg(format!(
                "COLRv0 glyph {glyph_id} has no CPAL palette"
            )));
        };
        let layers = (first..first + count)
            .map(|index| colr.v0_layer(index))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Error::msg(crate::colr::LOOKUP_PANIC))?;
        validate_colr0_layers(&layers, palette, self.outline.num_glyphs())
            .map(Some)
            .map_err(|error| Error::wrap(format!("COLRv0 glyph {glyph_id}"), error))
    }

    /// `GlyphRenderBounds`: paint bounds in sfnt.LoadGlyph's Y-down 26.6
    /// coordinates, and whether the glyph has ink.
    pub fn glyph_render_bounds(
        &self,
        glyph_id: u32,
        size: Int26_6,
    ) -> Result<(Rectangle26_6, bool), Error> {
        if size <= 0 {
            return Err(Error::msg("glyph size must be positive"));
        }
        if let Some(plan) = self.trusted_colrv1_plan(glyph_id)? {
            let Some(clip) = &plan.clip else {
                return Err(Error::msg(format!(
                    "trusted COLRv1 glyph {glyph_id} has no static clip box"
                )));
            };
            return colrv1_clip_bounds(clip, self.outline.units_per_em(), size);
        }
        let Some(layers) = self.colr0_glyph_layers(glyph_id)? else {
            let segments = self
                .outline
                .load_glyph(glyph_id as u16, size)
                .map_err(|error| Error::wrap(format!("load outline glyph {glyph_id}"), error))?;
            return Ok((sfnt::segments_bounds(&segments), !segments.is_empty()));
        };
        let mut bounds = Rectangle26_6::default();
        let mut has_ink = false;
        for layer in layers {
            let segments = self
                .outline
                .load_glyph(layer.glyph_id as u16, size)
                .map_err(|error| {
                    Error::wrap(format!("load COLRv0 layer glyph {}", layer.glyph_id), error)
                })?;
            if segments.is_empty() {
                continue;
            }
            if has_ink {
                bounds = bounds.union(sfnt::segments_bounds(&segments));
            } else {
                bounds = sfnt::segments_bounds(&segments);
                has_ink = true;
            }
        }
        Ok((bounds, has_ink))
    }
}

fn plan_has_clip(plan: &COLRv1Plan) -> bool {
    plan.clip
        .as_ref()
        .is_some_and(|clip| clip.x_min < clip.x_max && clip.y_min < clip.y_max)
}

/// `supportsRenderableRune`: the digest-neutral outline/COLRv0 predicate.
fn supports_renderable_rune(face: &ParsedFace, value: char) -> Result<bool, Error> {
    let Some(shaping_glyph) = face.shaping.nominal_glyph(value).filter(|&g| g != 0) else {
        return Ok(false);
    };
    let Some(outline_glyph) = face
        .outline_glyph(value)
        .filter(|&g| g != 0 && g == shaping_glyph)
    else {
        return Ok(false);
    };
    load_renderable(face, outline_glyph)
}

fn load_renderable(face: &ParsedFace, outline_glyph: u32) -> Result<bool, Error> {
    if let Some(layers) = face.colr0_glyph_layers(outline_glyph)? {
        for layer in layers {
            face.outline
                .load_glyph(layer.glyph_id as u16, crate::fixed::i(1_000))
                .map_err(|error| {
                    Error::wrap(format!("load COLRv0 layer glyph {}", layer.glyph_id), error)
                })?;
        }
        return Ok(true);
    }
    Ok(face
        .outline
        .load_glyph(outline_glyph as u16, crate::fixed::i(1_000))
        .is_ok())
}

/// `colrv1ClipBounds`.
pub(crate) fn colrv1_clip_bounds(
    clip: &COLRv1ClipBox,
    units_per_em: sfnt::Units,
    size: Int26_6,
) -> Result<(Rectangle26_6, bool), Error> {
    let invalid = || Error::msg("invalid COLRv1 clip bounds");
    if units_per_em == 0 {
        return Err(invalid());
    }
    let finite = |v: f64| v.is_finite();
    if !finite(clip.x_min)
        || !finite(clip.y_min)
        || !finite(clip.x_max)
        || !finite(clip.y_max)
        || clip.x_min >= clip.x_max
        || clip.y_min >= clip.y_max
    {
        return Err(invalid());
    }
    let scale = f64::from(size) / f64::from(units_per_em);
    let convert = |value: f64| -> Result<Int26_6, Error> {
        let value = value * scale;
        if !value.is_finite() || value > f64::from(i32::MAX) || value < f64::from(i32::MIN) {
            return Err(Error::msg("COLRv1 clip bounds exceed fixed-point range"));
        }
        Ok(value.round() as Int26_6)
    };
    let x_min = convert(clip.x_min)?;
    // COLR clip coordinates are Y-up; reflect around the baseline.
    let y_min = convert(-clip.y_max)?;
    let x_max = convert(clip.x_max)?;
    let y_max = convert(-clip.y_min)?;
    if x_min >= x_max || y_min >= y_max {
        return Err(Error::msg("COLRv1 clip bounds collapse at requested size"));
    }
    Ok((
        Rectangle26_6 {
            min: Point26_6 { x: x_min, y: y_min },
            max: Point26_6 { x: x_max, y: y_max },
        },
        true,
    ))
}

/// `validateCOLR0Layers`.
fn validate_colr0_layers(
    layers: &[crate::colr::ColrLayer],
    palette: &[ColorRecord],
    num_glyphs: usize,
) -> Result<Vec<ColorGlyphLayer>, Error> {
    if num_glyphs == 0 {
        return Err(Error::msg("font has no glyphs"));
    }
    if layers.is_empty() {
        return Err(Error::msg("glyph has no layers"));
    }
    if layers.len() > MAX_COLR0_GLYPH_LAYERS {
        return Err(Error::msg(format!(
            "layer count {} exceeds limit {MAX_COLR0_GLYPH_LAYERS}",
            layers.len()
        )));
    }
    let mut validated = Vec::with_capacity(layers.len());
    for (index, layer) in layers.iter().enumerate() {
        if usize::from(layer.glyph_id) >= num_glyphs {
            return Err(Error::msg(format!(
                "layer {index} glyph ID {} is out of range",
                layer.glyph_id
            )));
        }
        let mut resolved = ColorGlyphLayer {
            glyph_id: u32::from(layer.glyph_id),
            ..ColorGlyphLayer::default()
        };
        if layer.palette_index == u16::MAX {
            resolved.foreground = true;
        } else {
            let Some(entry) = palette.get(usize::from(layer.palette_index)) else {
                return Err(Error::msg(format!(
                    "layer {index} palette index {} is out of range",
                    layer.palette_index
                )));
            };
            resolved.color = Nrgba {
                r: entry.red,
                g: entry.green,
                b: entry.blue,
                a: entry.alpha,
            };
        }
        validated.push(resolved);
    }
    Ok(validated)
}

/// `BundledFaceSource`: an authenticated, package-owned clone source.
#[derive(Debug)]
pub struct BundledFaceSource {
    pub(crate) face: ParsedFace,
}

/// `BundledNotoColorEmojiSource`.
pub type BundledNotoColorEmojiSource = BundledFaceSource;

impl BundledFaceSource {
    /// `Outline`: the immutable shared sfnt reader.
    pub fn outline(&self) -> Result<&Arc<sfnt::Font>, Error> {
        Ok(&self.face.outline)
    }

    /// `IsBundledNotoColorEmoji`.
    pub fn is_bundled_noto_color_emoji(&self) -> bool {
        self.face.is_bundled_noto_color_emoji()
    }

    /// `COLR0GlyphLayers`.
    pub fn colr0_glyph_layers(&self, glyph_id: u32) -> Result<Option<Vec<ColorGlyphLayer>>, Error> {
        self.face.colr0_glyph_layers(glyph_id)
    }

    /// `CompileBundledNotoColorEmojiCOLRv1Plan`.
    pub fn compile_bundled_noto_color_emoji_colrv1_plan(
        &self,
        glyph_id: u32,
    ) -> Result<Option<COLRv1Plan>, Error> {
        self.face
            .compile_bundled_noto_color_emoji_colrv1_plan(glyph_id)
    }

    /// `GlyphRenderBounds`.
    pub fn glyph_render_bounds(
        &self,
        glyph_id: u32,
        size: Int26_6,
    ) -> Result<(Rectangle26_6, bool), Error> {
        self.face.glyph_render_bounds(glyph_id, size)
    }

    /// `GlyphDataKind`.
    pub fn glyph_data_kind(&self, glyph_id: u32) -> Result<&'static str, Error> {
        Ok(self.face.shaping.glyph_data_kind(glyph_id))
    }

    /// `CloneReadOnly`.
    pub fn clone_read_only(&self) -> Result<ParsedFace, Error> {
        self.face.clone()
    }

    /// `CloneReadOnlyInto`.
    pub fn clone_read_only_into(&self, clone: &mut ParsedFace) -> Result<(), Error> {
        self.face.clone_into(clone)
    }

    /// `SupportsRenderableRune` without a shaping-cache clone.
    pub fn supports_renderable_rune(&self, value: char) -> Result<bool, Error> {
        let face = &self.face;
        let Some(shaping_glyph) = face.shaping.font.nominal_glyph(value).filter(|&g| g != 0) else {
            return Ok(false);
        };
        let Some(outline_glyph) = face
            .outline_glyph(value)
            .filter(|&g| g != 0 && g == shaping_glyph)
        else {
            return Ok(false);
        };
        if !face.is_bundled_noto_color_emoji() {
            return Ok(true);
        }
        if let Some(plan) = face.compile_bundled_noto_color_emoji_colrv1_plan(outline_glyph)? {
            return Ok(plan_has_clip(&plan));
        }
        load_renderable(face, outline_glyph)
    }
}
