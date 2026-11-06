// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// Copyright 2021 The go-text authors (script subdivision, delimiter data and
// fixed GPOS anchor arithmetic); Rust translation Copyright 2026 Nelson Chen.
//! Source fontface/shaping.go: grapheme-affine face selection and fixed glyphs.
//! Frame-wide resolver/shaping budgets and cancellation are still separate work.

use super::font_fallback::source_cmap;
use rustybuzz::{Direction, Face, UnicodeBuffer};
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

pub(super) struct Font<'a> {
    pub data: &'a [u8],
    pub index: u32,
}

#[derive(Debug)]
pub(super) struct Glyph {
    pub id: u32,
    pub face: usize,
    pub x: f64,
    pub y: f64,
    #[cfg(test)]
    pub advance: f64,
    #[cfg(test)]
    pub source: char,
    pub source_index: usize,
    pub text: String,
    pub empty: bool,
}

pub(super) struct Shaped {
    pub glyphs: Vec<Glyph>,
    pub advance: f64,
}

pub(super) fn ignorable(c: char) -> bool {
    matches!(u32::from(c), 0xad | 0x34f | 0x61c | 0x17b4..=0x17b5 |
        0x180b..=0x180e | 0x200b..=0x200f | 0x202a..=0x202e |
        0x2060..=0x206f | 0xfe00..=0xfe0f | 0xfeff | 0xfff0..=0xfff8 |
        0x1d173..=0x1d17a | 0xe0000..=0xe0fff)
}

fn supports(face: &Face<'_>, c: char) -> bool {
    source_cmap::glyph_index(face, c).is_some_and(|id| face.glyph_index(c) == Some(id))
}

#[derive(Clone, Copy)]
struct Run {
    start: usize,
    end: usize,
    face: usize,
    rtl: bool,
    script: Script,
}

/// GPOS attaches marks by rounding the difference of two scaled float32
/// anchors, then resolving the attachment chain against fixed advances.
/// Scaling Rustybuzz's already-combined design offset loses that boundary.
fn fixed_mark_attachment(
    face: &Face<'_>,
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

/// Source's default LTR paragraph, script subdivision and occurrence-specific
/// face boundaries. Retain logical runs here; reverse consecutive RTL runs
/// after shaping, as orderShapedRunsLTR does.
fn runs(text: &str, assignment: &[usize]) -> Vec<Run> {
    let chars = text.char_indices().collect::<Vec<_>>();
    if chars.is_empty() {
        return Vec::new();
    }
    // go-text uses x/text's implicit paragraph level and its treatment of
    // formatting controls. Share the fontface port rather than UAX #9's
    // different run boundaries from unicode-bidi. ShapeTextTransient bypasses
    // bidi segmentation for ASCII, including independent code tokens.
    let bidi = if text.is_ascii() {
        vec![(0, chars.len(), false)]
    } else {
        let runes = chars.iter().map(|&(_, value)| value).collect::<Vec<_>>();
        d2_rs_fontface::bidi_runs(&runes).unwrap_or_else(|| vec![(0, runes.len(), false)])
    };
    let mut result = Vec::new();
    let mut delimiter_stack = Vec::<(usize, Script)>::new();
    for (at, end, rtl) in bidi {
        let mut start = at;
        let mut script = Script::Common;
        let mut semantic = Vec::new();
        for i in at..end {
            let mut next = chars[i].1.script();
            if matches!(next, Script::Common | Script::Inherited) {
                if let Ok(index) = PAIRED_DELIMITERS.binary_search(&u32::from(chars[i].1)) {
                    if index % 2 == 0 {
                        delimiter_stack.push((index, script));
                    } else {
                        let matching = delimiter_stack
                            .iter()
                            .rposition(|(open, _)| *open == index - 1);
                        if let Some(j) = matching {
                            next = delimiter_stack[j].1;
                        }
                        delimiter_stack.truncate(matching.unwrap_or(0));
                    }
                }
            }
            if matches!(next, Script::Common | Script::Inherited) || next == script {
                continue;
            }
            if script == Script::Common {
                for (_, value) in &mut delimiter_stack {
                    *value = next;
                }
                script = next;
            } else {
                semantic.push((start, i, script));
                start = i;
                script = next;
            }
        }
        semantic.push((start, end, script));
        for (start, end, script) in semantic {
            let mut first = start;
            for i in start + 1..=end {
                if i == end || assignment[i] != assignment[first] {
                    result.push(Run {
                        start: chars[first].0,
                        end: chars.get(i).map_or(text.len(), |c| c.0),
                        face: assignment[first],
                        rtl,
                        script,
                    });
                    first = i;
                }
            }
        }
    }
    let mut at = 0;
    while at < result.len() {
        if !result[at].rtl {
            at += 1;
            continue;
        }
        let start = at;
        while at < result.len() && result[at].rtl {
            at += 1;
        }
        result[start..at].reverse();
    }
    result
}

pub(super) fn shape(text: &str, size: f64, fonts: &[Font<'_>]) -> Option<Shaped> {
    if !size.is_finite() || size <= 0.0 || fonts.is_empty() {
        return None;
    }
    let faces = fonts
        .iter()
        .map(|font| Face::from_slice(font.data, font.index))
        .collect::<Option<Vec<_>>>()?;
    let mut assignment = Vec::with_capacity(text.chars().count());
    let mut replacements = Vec::with_capacity(assignment.capacity());
    for cluster in text.graphemes(true) {
        let selected = faces
            .iter()
            .position(|face| cluster.chars().all(|c| ignorable(c) || supports(face, c)));
        let (selected, replacement) = match selected {
            Some(face) => (face, None),
            None => ['\u{fffd}', '\u{25a1}', '\u{2610}', '?']
                .iter()
                .find_map(|&c| {
                    faces.iter().enumerate().find_map(|(i, face)| {
                        supports(face, c).then(|| (i, face.glyph_index(c).unwrap().0))
                    })
                })
                .map(|(face, id)| (face, Some(id)))?,
        };
        for _ in cluster.chars() {
            assignment.push(selected);
            replacements.push(replacement);
        }
    }
    let mut glyphs = Vec::new();
    let mut pen = 0.0;
    let scale = ((size * 64.0).round() / 64.0).ceil() * 64.0;
    for run in runs(text, &assignment) {
        let face = &faces[run.face];
        let units = f64::from(face.units_per_em());
        let source = &text[run.start..run.end];
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(source);
        buffer.guess_segment_properties();
        buffer.set_direction(if run.rtl {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        });
        buffer.set_script(rustybuzz::Script::from_iso15924_tag(
            ttf_parser::Tag::from_bytes(run.script.short_name().as_bytes().try_into().ok()?),
        )?);
        buffer.set_language("en".parse().ok()?);
        let output = rustybuzz::shape(face, &[], buffer);
        let mut clusters = output
            .glyph_infos()
            .iter()
            .map(|g| g.cluster as usize)
            .collect::<Vec<_>>();
        clusters.push(source.len());
        clusters.sort_unstable();
        clusters.dedup();
        let mut design_pen = 0.0;
        let mut previous = Vec::new();
        for (info, position) in output.glyph_infos().iter().zip(output.glyph_positions()) {
            let byte = run.start + info.cluster as usize;
            let source_index = text[..byte].chars().count();
            let _value = text[byte..].chars().next()?;
            let replacement = replacements[source_index].filter(|_| info.glyph_id == 0);
            let id = replacement.map_or(info.glyph_id, u32::from);
            let nominal =
                f64::from(face.glyph_hor_advance(ttf_parser::GlyphId(u16::try_from(id).ok()?))?);
            let is_mark = face
                .tables()
                .gdef
                .and_then(|gdef| gdef.glyph_class(ttf_parser::GlyphId(id as u16)))
                == Some(ttf_parser::gdef::GlyphClass::Mark);
            let hidden_space = face
                .glyph_index(' ')
                .is_some_and(|glyph| u32::from(glyph.0) == id)
                && source.chars().any(ignorable);
            let advance = if position.x_advance == 0 && (is_mark || hidden_space) {
                0.0
            } else if replacement.is_some() {
                (nominal * (size * 64.0).round() / units).round() / 64.0
            } else {
                (nominal * scale / units).round() / 64.0
                    + ((f64::from(position.x_advance) - nominal) * scale / units).trunc() / 64.0
            };
            let next = *clusters
                .iter()
                .find(|&&offset| offset > info.cluster as usize)?;
            let design = (
                design_pen + f64::from(position.x_offset),
                -f64::from(position.y_offset),
            );
            let fixed_attachment = (replacement.is_none() && position.x_advance == 0)
                .then(|| fixed_mark_attachment(face, id as u16, design, &previous, scale))
                .flatten();
            let x = fixed_attachment.map_or_else(
                || {
                    pen + if replacement.is_some() {
                        0.0
                    } else {
                        (f64::from(position.x_offset) * scale / units).trunc() / 64.0
                    }
                },
                |p| p.0,
            );
            let y = fixed_attachment.map_or_else(
                || {
                    if replacement.is_some() {
                        0.0
                    } else {
                        -(f64::from(position.y_offset) * scale / units).trunc() / 64.0
                    }
                },
                |p| p.1,
            );
            glyphs.push(Glyph {
                id,
                face: run.face,
                x,
                y,
                #[cfg(test)]
                advance,
                #[cfg(test)]
                source: _value,
                source_index,
                text: source[info.cluster as usize..next].to_owned(),
                // Rustybuzz emits the font's real spacing glyph for hidden
                // controls; Source retains it as a non-Empty, non-ink glyph.
                empty: false,
            });
            previous.push((id as u16, design.0, design.1, x, y));
            design_pen += f64::from(position.x_advance);
            pen += advance;
        }
    }
    Some(Shaped {
        glyphs,
        advance: pen,
    })
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

#[cfg(test)]
mod bundled_contract_tests {
    #[test]
    fn bidi_override_positions_match_fontface() {
        use super::*;
        use d2_rs_fontface::{ShapeFace, ShapeLimits, ShapingWorkspace, parse_face};
        let data = d2_rs_text::assets::SOURCE_SANS_PRO_REGULAR_TTF;
        let face = parse_face(data, 0).unwrap();
        for text in [
            "\u{202e}RTL override\u{202c}",
            "\u{202e}RTL override\u{202c} 5",
        ] {
            let expected = ShapingWorkspace::new()
                .shape_text_transient(
                    &(),
                    text,
                    32 * 64,
                    &[ShapeFace {
                        id: "primary",
                        face: &face,
                    }],
                    ShapeLimits {
                        runes: 1000,
                        faces: 10,
                        coverage_checks: 10000,
                        runs: 1000,
                        glyphs: 1000,
                    },
                )
                .unwrap();
            let actual = shape(text, 32.0, &[Font { data, index: 0 }]).unwrap();
            assert_eq!(actual.advance, expected.advance);
            assert_eq!(actual.glyphs.len(), expected.glyphs.len());
            for (a, e) in actual.glyphs.iter().zip(&expected.glyphs) {
                assert_eq!(
                    (
                        a.id,
                        a.x,
                        a.y,
                        a.advance,
                        a.source_index,
                        a.source as i32,
                        a.empty
                    ),
                    (
                        e.id,
                        e.position_x,
                        e.position_y,
                        e.advance,
                        e.source_index as usize,
                        e.source,
                        e.empty
                    )
                );
            }
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn bundled_graphemes_bidi_and_placeholders_match_pristine_shaping() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/font-grapheme-shaping-source-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let primary: &[u8] = match row["primary"].as_str().unwrap() {
                "FuzzyBubbles-Regular.ttf" => d2_rs_text::assets::FUZZY_BUBBLES_REGULAR_TTF,
                "SourceCodePro-Regular.ttf" => d2_rs_text::assets::SOURCE_CODE_PRO_REGULAR_TTF,
                _ => panic!("unknown Source fixture face"),
            };
            let fallback = d2_rs_text::assets::SOURCE_SANS_PRO_REGULAR_TTF;
            let shaped = shape(
                row["text"].as_str().unwrap(),
                row["size"].as_f64().unwrap(),
                &[
                    Font {
                        data: primary,
                        index: 0,
                    },
                    Font {
                        data: fallback,
                        index: 0,
                    },
                ],
            )
            .unwrap();
            let expected = &row["shaped"];
            assert_eq!(row["error"], "");
            assert_eq!(
                shaped.advance,
                expected["Advance"].as_f64().unwrap(),
                "{}",
                row
            );
            assert_eq!(
                shaped.glyphs.len(),
                expected["Glyphs"].as_array().unwrap().len(),
                "{}",
                row
            );
            for (g, e) in shaped
                .glyphs
                .iter()
                .zip(expected["Glyphs"].as_array().unwrap())
            {
                assert_eq!(
                    (
                        g.id,
                        g.face,
                        g.x,
                        g.y,
                        g.advance,
                        u32::from(g.source),
                        g.source_index,
                        g.empty
                    ),
                    (
                        e["ID"].as_u64().unwrap() as u32,
                        e["Face"].as_u64().unwrap() as usize,
                        e["PositionX"].as_f64().unwrap(),
                        e["PositionY"].as_f64().unwrap(),
                        e["Advance"].as_f64().unwrap(),
                        e["Source"].as_u64().unwrap() as u32,
                        e["SourceIndex"].as_u64().unwrap() as usize,
                        e["Empty"].as_bool().unwrap()
                    ),
                    "{}",
                    row
                );
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod color_contract_tests {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn color_graphemes_and_primary_spaces_match_pristine_shaping() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/font-color-shaping-source-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let primary = d2_rs_text::assets::SOURCE_SANS_PRO_REGULAR_TTF;
            let mut database = std::sync::Arc::new(resvg::usvg::fontdb::Database::new());
            let id = crate::bundled_emoji::Resolver::default()
                .select('😀', &[], &mut database)
                .unwrap();
            let fallback = database
                .with_face_data(id, |bytes, _| bytes.to_vec())
                .unwrap();
            let shaped = shape(
                row["text"].as_str().unwrap(),
                row["size"].as_f64().unwrap(),
                &[
                    Font {
                        data: primary,
                        index: 0,
                    },
                    Font {
                        data: &fallback,
                        index: 0,
                    },
                ],
            )
            .unwrap();
            let expected = &row["shaped"];
            assert_eq!(row["error"], "");
            assert_eq!(
                shaped.advance,
                expected["Advance"].as_f64().unwrap(),
                "{}",
                row
            );
            assert_eq!(
                shaped.glyphs.len(),
                expected["Glyphs"].as_array().unwrap().len(),
                "{}",
                row
            );
            for (g, e) in shaped
                .glyphs
                .iter()
                .zip(expected["Glyphs"].as_array().unwrap())
            {
                assert_eq!(
                    (
                        g.id,
                        g.face,
                        g.x,
                        g.y,
                        g.advance,
                        u32::from(g.source),
                        g.source_index,
                        g.empty
                    ),
                    (
                        e["ID"].as_u64().unwrap() as u32,
                        e["Face"].as_u64().unwrap() as usize,
                        e["PositionX"].as_f64().unwrap(),
                        e["PositionY"].as_f64().unwrap(),
                        e["Advance"].as_f64().unwrap(),
                        e["Source"].as_u64().unwrap() as u32,
                        e["SourceIndex"].as_u64().unwrap() as usize,
                        e["Empty"].as_bool().unwrap()
                    ),
                    "{}",
                    row
                );
            }
        }
    }
}

#[cfg(test)]
mod original_ascii_shaping_contract {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn ascii_font_runs_match_original_go() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/ascii-shaping-source-v090.json"
        ))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let data: &[u8] = match row["font"].as_str().unwrap() {
                "Regular" => d2_rs_text::assets::SOURCE_SANS_PRO_REGULAR_TTF,
                "Bold" => d2_rs_text::assets::SOURCE_SANS_PRO_BOLD_TTF,
                "Italic" => d2_rs_text::assets::SOURCE_SANS_PRO_ITALIC_TTF,
                _ => unreachable!(),
            };
            let shaped = super::shape(
                row["text"].as_str().unwrap(),
                row["size"].as_f64().unwrap(),
                &[super::Font { data, index: 0 }],
            )
            .unwrap();
            let expected = &row["shaped"];
            if shaped.advance != expected["Advance"].as_f64().unwrap() {
                differences.push(format!(
                    "{} {} {} total {} vs {}",
                    row["font"], row["size"], row["text"], shaped.advance, expected["Advance"]
                ));
            }
            assert_eq!(
                shaped.glyphs.len(),
                expected["Glyphs"].as_array().unwrap().len()
            );
            for (i, (g, e)) in shaped
                .glyphs
                .iter()
                .zip(expected["Glyphs"].as_array().unwrap())
                .enumerate()
            {
                if (g.id, g.x, g.y, g.advance)
                    != (
                        e["ID"].as_u64().unwrap() as u32,
                        e["PositionX"].as_f64().unwrap(),
                        e["PositionY"].as_f64().unwrap(),
                        e["Advance"].as_f64().unwrap(),
                    )
                {
                    differences.push(format!(
                        "{} {} {} glyph {} {:?} vs {}",
                        row["font"],
                        row["size"],
                        row["text"],
                        i,
                        (g.id, g.x, g.y, g.advance),
                        e
                    ));
                }
            }
        }
        assert!(
            differences.is_empty(),
            "{} differences: {:?}",
            differences.len(),
            differences.iter().take(8).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod original_control_shaping_contract {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn ascii_controls_match_original_go_glyphs() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/control-shaping-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 90);
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let data: &[u8] = match row["primary"].as_str().unwrap() {
                "SourceSansPro-Regular.ttf" => d2_rs_text::assets::SOURCE_SANS_PRO_REGULAR_TTF,
                "SourceSansPro-Bold.ttf" => d2_rs_text::assets::SOURCE_SANS_PRO_BOLD_TTF,
                "SourceSansPro-Italic.ttf" => d2_rs_text::assets::SOURCE_SANS_PRO_ITALIC_TTF,
                _ => unreachable!(),
            };
            let actual = super::shape(
                row["text"].as_str().unwrap(),
                row["size"].as_f64().unwrap(),
                &[super::Font { data, index: 0 }],
            )
            .unwrap();
            let expected = &row["shaped"];
            let glyphs = actual.glyphs.iter().map(|g| serde_json::json!({"ID":g.id,"Face":g.face,"PositionX":g.x,"PositionY":g.y,"Advance":g.advance,"Source":u32::from(g.source),"SourceIndex":g.source_index,"Empty":g.empty})).collect::<Vec<_>>();
            let expected_glyphs = expected["Glyphs"].as_array().unwrap().iter().map(|g| serde_json::json!({"ID":g["ID"],"Face":g["Face"],"PositionX":g["PositionX"].as_f64().unwrap(),"PositionY":g["PositionY"].as_f64().unwrap(),"Advance":g["Advance"].as_f64().unwrap(),"Source":g["Source"],"SourceIndex":g["SourceIndex"],"Empty":g["Empty"]})).collect::<Vec<_>>();
            if actual.advance != expected["Advance"].as_f64().unwrap() || glyphs != expected_glyphs
            {
                differences.push(format!(
                    "{} size {} text {}: {:?} / {} vs {:?} / {}",
                    row["primary"],
                    row["size"],
                    row["text"],
                    glyphs,
                    actual.advance,
                    expected_glyphs,
                    expected["Advance"]
                ));
            }
        }
        assert!(
            differences.is_empty(),
            "{} control-shaping differences: {:?}",
            differences.len(),
            differences.iter().take(4).collect::<Vec<_>>()
        );
    }
}
