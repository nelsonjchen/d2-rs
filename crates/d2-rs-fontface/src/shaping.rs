// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Copyright 2021 The go-text authors (script/bidi segmentation, language
// enforcement and delimiter data, fixed GPOS anchor arithmetic); Rust
// translation Copyright 2026 Nelson Chen.
//! Source d2renderers/internal/fontface/shaping.go: grapheme-affine font
//! selection, go-text segmentation (bidi, script, language), HarfBuzz
//! shaping and fixed-point glyph placement with ink bounds.
//!
//! go-text shapes at `XScale = Ceil(size) << 6` and reports 26.6 positions.
//! The vendored go-v090 rustybuzz shapes in font units, so `hb_run` rebuilds
//! go-text's per-glyph fixed values from the unscaled result, following the
//! validated translation in d2-rs-export's font_shaping.rs.

use crate::error::Error;
use crate::face::{ParsedFace, ShapingFont};
use crate::fixed::{Int26_6, Rectangle26_6, ceil};
use crate::sfnt::Hinting;
use crate::shaping_tables::{SCRIPTS, UNKNOWN_SCRIPT};
use crate::unicode::is_default_ignorable_rune;
use d2_rs_render::go_float::go_fmadd;
use std::collections::HashMap;

/// Cancellation, standing in for Go's `context.Context`.
pub trait Context {
    /// `ctx.Err()`: Err once the operation is canceled.
    fn err(&self) -> Result<(), Error>;
}

/// `context.Background()`.
impl Context for () {
    fn err(&self) -> Result<(), Error> {
        Ok(())
    }
}

impl<F: Fn() -> Result<(), Error>> Context for F {
    fn err(&self) -> Result<(), Error> {
        self()
    }
}

/// `ShapeFace`: a caller-owned identifier for one parsed face.
#[derive(Clone, Copy, Debug)]
pub struct ShapeFace<'a> {
    pub id: &'a str,
    pub face: &'a ParsedFace,
}

/// `ShapeLimits`: every field must be positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeLimits {
    pub runes: i64,
    pub faces: i64,
    pub coverage_checks: i64,
    pub runs: i64,
    pub glyphs: i64,
}

/// `ShapedGlyph`: renderer-neutral placement for one glyph. Ink is in the
/// font's 26.6 Y-down coordinates relative to the glyph position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShapedGlyph {
    pub id: u32,
    pub face: usize,
    pub position_x: f64,
    pub position_y: f64,
    pub advance: f64,
    pub ink: Rectangle26_6,
    pub has_ink: bool,
    pub empty: bool,
    /// The source rune (Go `rune`), 0 when the cluster is out of range.
    pub source: i32,
    pub source_index: i64,
}

/// `ShapedText`: one visually ordered, left-to-right placed line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapedText {
    pub glyphs: Vec<ShapedGlyph>,
    pub advance: f64,
    pub runes: i64,
    pub coverage_checks: i64,
    pub runs: i64,
}

const MAX_CACHED_GLYPH_OUTLINES: usize = 4_096;

/// `missingGlyphPlaceholderRunes`.
const MISSING_GLYPH_PLACEHOLDER_RUNES: [char; 4] = ['\u{fffd}', '\u{25a1}', '\u{2610}', '?'];

/// `ShapingWorkspace`: reusable scratch state for a sequence of shaping
/// calls. It must not be used concurrently.
#[derive(Default)]
pub struct ShapingWorkspace {
    outlines: HashMap<(u64, u16, Int26_6), (Rectangle26_6, bool)>,
}

impl std::fmt::Debug for ShapingWorkspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShapingWorkspace").finish_non_exhaustive()
    }
}

/// One `shaping.Input` run.
#[derive(Clone, Copy, Debug)]
struct Input {
    start: usize,
    end: usize,
    rtl: bool,
    script: Script,
    language: &'static str,
    face: usize,
}

/// A go-text output glyph rebuilt from rustybuzz.
struct RunGlyph {
    glyph_id: u32,
    cluster: usize,
    /// `fixedToFloat(XOffset)` relative to the run pen.
    x_offset: f64,
    /// `-fixedToFloat(YOffset)`.
    position_y: f64,
    /// `fixedToFloat(XAdvance)`.
    advance: f64,
}

struct ShapedRun {
    glyphs: Vec<RunGlyph>,
    rtl: bool,
    face: usize,
}

/// Go `%q` for a string.
pub(crate) fn go_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\u{b}' => out.push_str("\\v"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn msg(text: String) -> Error {
    Error::msg(text)
}

/// `clusterFaceResolver` coverage with its per-call check accounting: each
/// distinct (face, rune) costs one check per call.
struct Coverage<'f> {
    faces: &'f [ShapeFace<'f>],
    known: HashMap<(usize, char), bool>,
    checks: i64,
    limit: i64,
}

impl Coverage<'_> {
    fn supports(&mut self, face: usize, value: char) -> Result<bool, Error> {
        if let Some(&supported) = self.known.get(&(face, value)) {
            return Ok(supported);
        }
        if self.checks >= self.limit {
            return Err(msg(format!(
                "d2fonts: text font coverage checks exceed limit {}",
                self.limit
            )));
        }
        self.checks += 1;
        let supported = self.faces[face].face.supports_renderable_rune(value)?;
        self.known.insert((face, value), supported);
        Ok(supported)
    }

    fn resolve_cluster(&mut self, values: &[char], offset: usize) -> Result<(usize, u32), Error> {
        for face in 0..self.faces.len() {
            let mut covers = true;
            for &value in values {
                if is_default_ignorable_rune(value) {
                    continue;
                }
                if !self.supports(face, value)? {
                    covers = false;
                    break;
                }
            }
            if covers {
                return Ok((face, 0));
            }
        }
        for replacement in MISSING_GLYPH_PLACEHOLDER_RUNES {
            for face in 0..self.faces.len() {
                if !self.supports(face, replacement)? {
                    continue;
                }
                let Some(glyph) = self.faces[face].face.shaping().nominal_glyph(replacement) else {
                    continue;
                };
                if glyph == 0 || glyph > u32::from(u16::MAX) {
                    continue;
                }
                return Ok((face, glyph));
            }
        }
        Err(msg(format!(
            "d2fonts: no font face has a drawable missing-glyph placeholder for grapheme cluster beginning at rune {offset}"
        )))
    }
}

impl ShapingWorkspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// `ShapeTextTransient`: HarfBuzz-compatible shaping with bidi, script,
    /// ligature, mark and ordered, grapheme-affine font fallback.
    pub fn shape_text_transient(
        &mut self,
        ctx: &dyn Context,
        text: &str,
        size: Int26_6,
        faces: &[ShapeFace<'_>],
        limits: ShapeLimits,
    ) -> Result<ShapedText, Error> {
        ctx.err()?;
        if size <= 0 {
            return Err(Error::msg("d2fonts: shaping size must be positive"));
        }
        if limits.runes <= 0
            || limits.faces <= 0
            || limits.coverage_checks <= 0
            || limits.runs <= 0
            || limits.glyphs <= 0
        {
            return Err(Error::msg("d2fonts: every shaping limit must be positive"));
        }
        if faces.is_empty() {
            return Err(Error::msg(
                "d2fonts: shaping requires at least one font face",
            ));
        }
        if faces.len() as i64 > limits.faces {
            return Err(msg(format!(
                "d2fonts: font face count {} exceeds limit {}",
                faces.len(),
                limits.faces
            )));
        }
        let mut face_ids: HashMap<u64, usize> = HashMap::new();
        for (index, face) in faces.iter().enumerate() {
            if faces.len() > 1 && face_ids.insert(face.face.shaping().id(), index).is_some() {
                return Err(msg(format!(
                    "d2fonts: font face {index} ({}) duplicates an earlier shaping face",
                    go_quote(face.id)
                )));
            }
        }

        // boundedRuneCount (Rust strings are always valid UTF-8).
        let mut rune_count = 0i64;
        for _ in text.chars() {
            rune_count += 1;
            if rune_count > limits.runes {
                return Err(msg(format!(
                    "d2fonts: text rune count exceeds per-run limit {}",
                    limits.runes
                )));
            }
        }
        ctx.err()?;
        let mut result = ShapedText {
            runes: rune_count,
            ..ShapedText::default()
        };
        if rune_count == 0 {
            return Ok(result);
        }
        let runes: Vec<char> = text.chars().collect();
        let ascii = runes.iter().all(char::is_ascii);
        let ascii_latin = runes.iter().any(char::is_ascii_alphabetic);

        let mut coverage = Coverage {
            faces,
            known: HashMap::new(),
            checks: 0,
            limit: limits.coverage_checks,
        };
        let mut assignments = vec![0usize; runes.len()];
        let mut replacements = vec![0u32; runes.len()];
        for (offset, length) in crate::segmenter::graphemes(&runes) {
            ctx.err()?;
            let (face, replacement) =
                coverage.resolve_cluster(&runes[offset..offset + length], offset)?;
            for index in offset..offset + length {
                assignments[index] = face;
                replacements[index] = replacement;
            }
        }
        result.coverage_checks = coverage.checks;

        let semantic = if ascii {
            vec![Input {
                start: 0,
                end: runes.len(),
                rtl: false,
                script: if ascii_latin { LATIN } else { COMMON },
                language: "en",
                face: 0,
            }]
        } else {
            split_semantic(&runes)
        };
        ctx.err()?;
        let inputs = if faces.len() == 1 {
            if semantic.len() as i64 > limits.runs {
                return Err(msg(format!(
                    "d2fonts: text shaping run count exceeds limit {}",
                    limits.runs
                )));
            }
            semantic
        } else {
            split_inputs_by_assigned_face(&semantic, &assignments, faces.len(), limits.runs)?
        };
        if inputs.len() as i64 > limits.runs {
            return Err(msg(format!(
                "d2fonts: text shaping run count {} exceeds limit {}",
                inputs.len(),
                limits.runs
            )));
        }
        result.runs = inputs.len() as i64;

        let mut total_glyphs = 0i64;
        let mut runs = Vec::with_capacity(inputs.len());
        for input in &inputs {
            ctx.err()?;
            let face = faces[input.face].face;
            let glyphs = hb_run(face, &runes, &replacements, input, size);
            ctx.err()?;
            if glyphs.len() as i64 > limits.glyphs - total_glyphs {
                return Err(msg(format!(
                    "d2fonts: shaped glyph count exceeds limit {}",
                    limits.glyphs
                )));
            }
            total_glyphs += glyphs.len() as i64;
            runs.push(ShapedRun {
                glyphs,
                rtl: input.rtl,
                face: input.face,
            });
        }
        if runs.len() > 1 {
            order_shaped_runs_ltr(&mut runs);
        }

        let mut glyphs = Vec::with_capacity(total_glyphs as usize);
        let mut pen = 0.0f64;
        let mut overflow: HashMap<(u64, u16, Int26_6), (Rectangle26_6, bool)> = HashMap::new();
        for run in &runs {
            let parsed = faces[run.face].face;
            let face_id = faces[run.face].id;
            let mut run_pen = pen;
            for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
                ctx.err()?;
                let (source, source_at) = shaped_glyph_source(&runes, glyph.cluster);
                // shaping.go:321 (FMADDDx1): runPen + XOffset*2^-6; the
                // power-of-two product is exact, so fusion cannot round.
                let mut position_x = go_fmadd(glyph.x_offset * 64.0, 1.0 / 64.0, run_pen);
                let mut position_y = glyph.position_y;
                let mut advance = glyph.advance;
                let mut glyph_id = glyph.glyph_id;
                if glyph.glyph_id == 0 {
                    let replacement = usize::try_from(source_at)
                        .ok()
                        .and_then(|at| replacements.get(at).copied())
                        .unwrap_or(0);
                    if replacement == 0 {
                        return Err(missing_shaped_glyph_error(&runes, glyph.cluster, face_id));
                    }
                    glyph_id = replacement;
                    position_x = run_pen;
                    position_y = 0.0;
                    let replacement_advance = parsed
                        .outline()
                        .glyph_advance(glyph_id as u16, size, Hinting::None)
                        .map_err(|error| {
                            Error::wrap(
                                format!(
                                    "d2fonts: measure missing-glyph placeholder {glyph_id} in font {}",
                                    go_quote(face_id)
                                ),
                                error,
                            )
                        })?;
                    advance = fixed_to_float(replacement_advance);
                }
                if glyph_id > u32::from(u16::MAX)
                    || glyph_id as usize >= parsed.outline().num_glyphs()
                {
                    return Err(msg(format!(
                        "d2fonts: shaped glyph ID {glyph_id} at index {glyph_index} is out of range for font {}",
                        go_quote(face_id)
                    )));
                }
                let key = (parsed.shaping().id(), glyph_id as u16, size);
                let cached = self
                    .outlines
                    .get(&key)
                    .or_else(|| overflow.get(&key))
                    .copied();
                let (bounds, has_ink) = match cached {
                    Some(outline) => outline,
                    None => {
                        let outline =
                            parsed
                                .glyph_render_bounds(glyph_id, size)
                                .map_err(|error| {
                                    unsupported_shaped_glyph_error(
                                        parsed, glyph_id, source, source_at, face_id, error,
                                    )
                                })?;
                        if self.outlines.len() < MAX_CACHED_GLYPH_OUTLINES {
                            self.outlines.insert(key, outline);
                        } else {
                            overflow.insert(key, outline);
                        }
                        outline
                    }
                };
                glyphs.push(ShapedGlyph {
                    id: glyph_id,
                    face: run.face,
                    position_x,
                    position_y,
                    advance,
                    ink: bounds,
                    has_ink,
                    empty: false,
                    source,
                    source_index: source_at,
                });
                // shaping.go:329 (FMADDDx1): runPen + XAdvance*2^-6 (exact).
                run_pen = go_fmadd(advance * 64.0, 1.0 / 64.0, run_pen);
            }
            pen = run_pen;
        }
        result.advance = pen;
        result.glyphs = glyphs;
        Ok(result)
    }
}

fn fixed_to_float(value: Int26_6) -> f64 {
    f64::from(value) / 64.0
}

fn shaped_glyph_source(runes: &[char], cluster: usize) -> (i32, i64) {
    match runes.get(cluster) {
        Some(&value) => (value as i32, cluster as i64),
        None => (0, cluster as i64),
    }
}

fn missing_shaped_glyph_error(runes: &[char], cluster: usize, face: &str) -> Error {
    match runes.get(cluster) {
        Some(&value) => msg(format!(
            "d2fonts: missing shaped glyph for U+{:04X} at rune {cluster} in font {}",
            value as u32,
            go_quote(face)
        )),
        None => msg(format!(
            "d2fonts: missing shaped glyph at cluster {cluster} in font {}",
            go_quote(face)
        )),
    }
}

fn unsupported_shaped_glyph_error(
    face: &ParsedFace,
    glyph: u32,
    source: i32,
    source_at: i64,
    id: &str,
    cause: Error,
) -> Error {
    let kind = match face.shaping().glyph_data_kind(glyph) {
        "outline" | "color" | "bitmap" | "SVG" | "missing" => face.shaping().glyph_data_kind(glyph),
        _ => "non-outline",
    };
    if source_at >= 0 {
        Error::wrap(
            format!(
                "d2fonts: {kind} glyph U+{:04X} at rune {source_at} in font {} cannot be rasterized",
                source as u32,
                go_quote(id)
            ),
            cause,
        )
    } else {
        Error::wrap(
            format!(
                "d2fonts: {kind} glyph at cluster {source_at} in font {} cannot be rasterized",
                go_quote(id)
            ),
            cause,
        )
    }
}

/// `orderShapedRunsLTR`: reverse each maximal sequence of RTL runs.
fn order_shaped_runs_ltr(runs: &mut [ShapedRun]) {
    let mut bidi_start: Option<usize> = None;
    for index in 0..runs.len() {
        if !runs[index].rtl {
            if let Some(start) = bidi_start.take() {
                runs[start..index].reverse();
            }
        } else if bidi_start.is_none() {
            bidi_start = Some(index);
        }
    }
    if let Some(start) = bidi_start {
        runs[start..].reverse();
    }
}

/// `splitInputsByAssignedFace`.
fn split_inputs_by_assigned_face(
    inputs: &[Input],
    assignments: &[usize],
    num_faces: usize,
    max_runs: i64,
) -> Result<Vec<Input>, Error> {
    let mut result = Vec::with_capacity(inputs.len().min(max_runs as usize));
    for input in inputs {
        if input.start >= input.end || input.end > assignments.len() {
            return Err(msg(format!(
                "d2fonts: shaper segment has invalid rune range [{}:{}]",
                input.start, input.end
            )));
        }
        let mut start = input.start;
        let mut face = assignments[start];
        for index in start + 1..=input.end {
            if index < input.end && assignments[index] == face {
                continue;
            }
            if face >= num_faces {
                return Err(msg(format!("d2fonts: invalid assigned font face {face}")));
            }
            if result.len() as i64 >= max_runs {
                return Err(msg(format!(
                    "d2fonts: text shaping run count exceeds limit {max_runs}"
                )));
            }
            result.push(Input {
                start,
                end: index,
                face,
                ..*input
            });
            if index < input.end {
                start = index;
                face = assignments[index];
            }
        }
    }
    Ok(result)
}

/// A go-text `language.Script` as its ISO 15924 tag.
type Script = [u8; 4];

const COMMON: Script = *b"Zyyy";
const INHERITED: Script = *b"Zinh";
const LATIN: Script = *b"Latn";

/// `language.LookupScript`.
fn lookup_script(value: char) -> Script {
    let value = u32::from(value);
    match SCRIPTS.binary_search_by(|&(first, last, _)| {
        if value < first {
            std::cmp::Ordering::Greater
        } else if value > last {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(index) => SCRIPTS[index].2,
        Err(_) => UNKNOWN_SCRIPT,
    }
}

/// `Script.Strong`.
fn strong(script: Script) -> bool {
    script != COMMON && script != INHERITED
}

/// go-text `Segmenter.Split` for D2's LTR input with one face:
/// splitByBidi (x/text bidi, implicit paragraph level), splitByScript
/// (paired-delimiter aware) and enforceLanguages.
fn split_semantic(runes: &[char]) -> Vec<Input> {
    let bidi_runs = crate::bidi::order(runes).unwrap_or_else(|| vec![(0, runes.len(), false)]);
    let mut result = Vec::new();
    let mut delimiter_stack: Vec<(usize, Script)> = Vec::new();
    for (start, end, rtl) in bidi_runs {
        let mut current = Input {
            start,
            end,
            rtl,
            script: COMMON,
            language: "",
            face: 0,
        };
        for i in start..end {
            let r = runes[i];
            let mut script = lookup_script(r);
            let delimiter = if strong(script) {
                None
            } else {
                PAIRED_DELIMITERS.binary_search(&u32::from(r)).ok()
            };
            if let Some(index) = delimiter {
                if index % 2 == 0 {
                    delimiter_stack.push((index, current.script));
                } else {
                    let counterpart = index - 1;
                    let found = delimiter_stack
                        .iter()
                        .rposition(|(open, _)| *open == counterpart);
                    if let Some(j) = found {
                        script = delimiter_stack[j].1;
                    }
                    delimiter_stack.truncate(found.unwrap_or(0));
                }
            }
            if !strong(script) || script == current.script {
                continue;
            } else if current.script == COMMON {
                for entry in &mut delimiter_stack {
                    entry.1 = script;
                }
                current.script = script;
            } else {
                if i != current.start {
                    let mut pushed = current;
                    pushed.end = i;
                    result.push(pushed);
                }
                current.start = i;
                current.script = script;
            }
        }
        current.end = end;
        result.push(current);
    }
    for input in &mut result {
        input.language = enforce_language(input.script);
    }
    result
}

/// `enforceLang` from the default "en": English keeps Latin and weak
/// scripts; other scripts use go-text's `ScriptToLang` sample language.
fn enforce_language(script: Script) -> &'static str {
    match std::str::from_utf8(&script).unwrap_or("") {
        "Arab" => "ar",
        "Armn" => "hy",
        "Beng" => "bn",
        "Cher" => "chr",
        "Copt" => "cop",
        "Cyrl" => "ru",
        "Deva" => "hi",
        "Ethi" => "am",
        "Geor" => "ka",
        "Grek" => "el",
        "Gujr" => "gu",
        "Guru" => "pa",
        "Hang" => "ko",
        "Hebr" => "he",
        "Hira" | "Kana" => "ja",
        "Knda" => "kn",
        "Khmr" => "km",
        "Laoo" => "lo",
        "Mlym" => "ml",
        "Mong" => "mn",
        "Mymr" => "my",
        "Orya" => "or",
        "Sinh" => "si",
        "Syrc" => "syr",
        "Taml" => "ta",
        "Telu" => "te",
        "Thaa" => "dv",
        "Thai" => "th",
        "Tibt" => "bo",
        "Cans" => "iu",
        "Tglg" => "tl",
        "Hano" => "hnn",
        "Buhd" => "bku",
        "Tagb" => "tbw",
        "Ugar" => "uga",
        "Bugi" => "bug",
        "Sylo" => "syl",
        "Xpeo" => "peo",
        "Nkoo" => "nqo",
        _ => "en",
    }
}

/// GPOS attaches marks by rounding the difference of two scaled float32
/// anchors, then resolving the attachment chain against fixed advances.
fn fixed_mark_attachment(
    face: &rustybuzz::Face<'_>,
    glyph: u16,
    design: (f64, f64),
    previous: &[(u16, f64, f64, f64, f64)],
    scale: f64,
) -> Option<(f64, f64)> {
    use ttf_parser::gpos::{Anchor, PositioningSubtable};
    let table = face.tables().gpos?;
    let units = face.units_per_em() as f32;
    let coordinates = |anchor: Anchor<'_>| {
        (
            anchor.x,
            anchor.y,
            anchor.x_device.is_some() || anchor.y_device.is_some(),
        )
    };
    let difference = |base: (i16, i16, bool), mark: (i16, i16, bool)| {
        if base.2 || mark.2 {
            return None;
        }
        let scalar = |base: i16, mark: i16| {
            let base = f32::from(base) * scale as f32 / units;
            let mark = f32::from(mark) * scale as f32 / units;
            f64::from((base - mark).round()) / 64.0
        };
        Some((scalar(base.0, mark.0), scalar(base.1, mark.1)))
    };
    for lookup in table.lookups {
        for subtable in lookup.subtables.into_iter::<PositioningSubtable<'_>>() {
            for &(base_id, base_x, base_y, native_x, native_y) in previous.iter().rev() {
                let anchors = match subtable {
                    PositioningSubtable::MarkToBase(s) => {
                        let mark = s.mark_coverage.get(ttf_parser::GlyphId(glyph));
                        let base = s.base_coverage.get(ttf_parser::GlyphId(base_id));
                        mark.zip(base).and_then(|(mark, base)| {
                            s.marks.get(mark).and_then(|(class, mark)| {
                                s.anchors
                                    .get(base, class)
                                    .map(|base| (coordinates(base), coordinates(mark)))
                            })
                        })
                    }
                    PositioningSubtable::MarkToMark(s) => {
                        let mark = s.mark1_coverage.get(ttf_parser::GlyphId(glyph));
                        let base = s.mark2_coverage.get(ttf_parser::GlyphId(base_id));
                        mark.zip(base).and_then(|(mark, base)| {
                            s.marks.get(mark).and_then(|(class, mark)| {
                                s.mark2_matrix
                                    .get(base, class)
                                    .map(|base| (coordinates(base), coordinates(mark)))
                            })
                        })
                    }
                    _ => None,
                };
                if let Some((base, mark)) = anchors {
                    // Match the attachment actually applied by the shaper;
                    // inactive lookup entries must not change placement.
                    if design.0 - base_x == f64::from(base.0) - f64::from(mark.0)
                        && design.1 - base_y == -(f64::from(base.1) - f64::from(mark.1))
                        && let Some((x, y)) = difference(base, mark)
                    {
                        return Some((native_x + x, native_y - y));
                    }
                }
            }
        }
    }
    None
}

/// `HarfbuzzShaper.Shape` for one input: go-text adds the run with up to
/// five runes of pre/post context and rune-index clusters.
fn hb_run(
    parsed: &ParsedFace,
    runes: &[char],
    replacements: &[u32],
    input: &Input,
    size: Int26_6,
) -> Vec<RunGlyph> {
    let font: &ShapingFont = parsed.shaping().font();
    let face = font.rustybuzz_face();
    let units = f64::from(face.units_per_em());
    let scale = f64::from(ceil(size) << 6);
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    let pre: String = runes[input.start.saturating_sub(5)..input.start]
        .iter()
        .collect();
    buffer.set_pre_context(&pre);
    for (index, &value) in runes[input.start..input.end].iter().enumerate() {
        buffer.add(value, (input.start + index) as u32);
    }
    let post: String = runes[input.end..(input.end + 5).min(runes.len())]
        .iter()
        .collect();
    buffer.set_post_context(&post);
    buffer.set_direction(if input.rtl {
        rustybuzz::Direction::RightToLeft
    } else {
        rustybuzz::Direction::LeftToRight
    });
    if let Some(script) =
        rustybuzz::Script::from_iso15924_tag(ttf_parser::Tag::from_bytes(&input.script))
    {
        buffer.set_script(script);
    }
    if let Ok(language) = input.language.parse() {
        buffer.set_language(language);
    }
    let output = rustybuzz::shape(face, &[], buffer);
    let has_ignorable = runes[input.start..input.end]
        .iter()
        .any(|&value| is_default_ignorable_rune(value));
    let space = face.glyph_index(' ').map(|glyph| u32::from(glyph.0));
    let mut glyphs = Vec::with_capacity(output.len());
    let mut design_pen = 0.0;
    let mut pen = 0.0;
    let mut previous = Vec::new();
    for (info, position) in output.glyph_infos().iter().zip(output.glyph_positions()) {
        let cluster = info.cluster as usize;
        // The shaping loop substitutes the placeholder for glyph zero; place
        // it here too so later mark attachments see the same pen.
        let replacement = replacements
            .get(cluster)
            .copied()
            .filter(|&glyph| glyph != 0 && info.glyph_id == 0);
        let id = replacement.unwrap_or(info.glyph_id);
        let nominal = face
            .glyph_hor_advance(ttf_parser::GlyphId(id as u16))
            .map_or(0.0, f64::from);
        let is_mark = face
            .tables()
            .gdef
            .and_then(|gdef| gdef.glyph_class(ttf_parser::GlyphId(id as u16)))
            == Some(ttf_parser::gdef::GlyphClass::Mark);
        let hidden_space = space == Some(id) && has_ignorable;
        let advance = if position.x_advance == 0 && (is_mark || hidden_space) {
            0.0
        } else if replacement.is_some() {
            parsed
                .outline()
                .glyph_advance(id as u16, size, Hinting::None)
                .map_or(0.0, fixed_to_float)
        } else {
            (nominal * scale / units).round() / 64.0
                + ((f64::from(position.x_advance) - nominal) * scale / units).trunc() / 64.0
        };
        let design = (
            design_pen + f64::from(position.x_offset),
            -f64::from(position.y_offset),
        );
        let fixed_attachment = (replacement.is_none() && position.x_advance == 0)
            .then(|| fixed_mark_attachment(face, id as u16, design, &previous, scale))
            .flatten();
        let (x, y) = fixed_attachment.unwrap_or_else(|| {
            if replacement.is_some() {
                return (pen, 0.0);
            }
            (
                pen + (f64::from(position.x_offset) * scale / units).trunc() / 64.0,
                -(f64::from(position.y_offset) * scale / units).trunc() / 64.0,
            )
        });
        glyphs.push(RunGlyph {
            glyph_id: info.glyph_id,
            cluster,
            x_offset: x - pen,
            position_y: y,
            advance,
        });
        previous.push((id as u16, design.0, design.1, x, y));
        design_pen += f64::from(position.x_advance);
        pen += advance;
    }
    glyphs
}

// go-text/typesetting v0.3.4 shaping/paired_delims_table.go (Unlicense OR BSD).
const PAIRED_DELIMITERS: &[u32] = &[
    0x0028, 0x0029, 0x003c, 0x003e, 0x005b, 0x005d, 0x007b, 0x007d, 0x00ab, 0x00bb, 0x2018, 0x2019,
    0x201a, 0x201b, 0x201c, 0x201d, 0x201e, 0x201f, 0x2039, 0x203a, 0x2045, 0x2046, 0x207d, 0x207e,
    0x208d, 0x208e, 0x2308, 0x2309, 0x230a, 0x230b, 0x2329, 0x232a, 0x2768, 0x2769, 0x276a, 0x276b,
    0x276c, 0x276d, 0x276e, 0x276f, 0x2770, 0x2771, 0x2772, 0x2773, 0x2774, 0x2775, 0x27c5, 0x27c6,
    0x27e6, 0x27e7, 0x27e8, 0x27e9, 0x27ea, 0x27eb, 0x27ec, 0x27ed, 0x27ee, 0x27ef, 0x2983, 0x2984,
    0x2985, 0x2986, 0x2987, 0x2988, 0x2989, 0x298a, 0x298b, 0x298c, 0x298d, 0x298e, 0x298f, 0x2990,
    0x2991, 0x2992, 0x2993, 0x2994, 0x2995, 0x2996, 0x2997, 0x2998, 0x29d8, 0x29d9, 0x29da, 0x29db,
    0x29fc, 0x29fd, 0x2e02, 0x2e03, 0x2e04, 0x2e05, 0x2e09, 0x2e0a, 0x2e0c, 0x2e0d, 0x2e1c, 0x2e1d,
    0x2e20, 0x2e21, 0x2e22, 0x2e23, 0x2e24, 0x2e25, 0x2e26, 0x2e27, 0x2e28, 0x2e29, 0x2e42, 0x2e55,
    0x2e56, 0x2e57, 0x2e58, 0x2e59, 0x2e5a, 0x2e5b, 0x2e5c, 0x3008, 0x3009, 0x300a, 0x300b, 0x300c,
    0x300d, 0x300e, 0x300f, 0x3010, 0x3011, 0x3014, 0x3015, 0x3016, 0x3017, 0x3018, 0x3019, 0x301a,
    0x301b, 0x301d, 0x301e, 0x301f, 0xfd3e, 0xfd3f, 0xfe17, 0xfe18, 0xfe35, 0xfe36, 0xfe37, 0xfe38,
    0xfe39, 0xfe3a, 0xfe3b, 0xfe3c, 0xfe3d, 0xfe3e, 0xfe3f, 0xfe40, 0xfe41, 0xfe42, 0xfe43, 0xfe44,
    0xfe47, 0xfe48, 0xfe59, 0xfe5a, 0xfe5b, 0xfe5c, 0xfe5d, 0xfe5e, 0xff08, 0xff09, 0xff3b, 0xff3d,
    0xff5b, 0xff5d, 0xff5f, 0xff60, 0xff62, 0xff63,
];
