// SPDX-License-Identifier: MPL-2.0
//! Focused contracts of the fontface port. Whole-corpus parity against the
//! pinned Go package is scripts/fontface-scoreboard.py; these tests pin the
//! subtle pieces it relies on and the error paths it does not reach.

#[cfg(feature = "oracle-fixtures")]
use d2_rs_fontface::COLRv1Paint;
use d2_rs_fontface::sfnt::{self, Hinting, SegmentOp};
use d2_rs_fontface::{
    Error, ParsedFace, ShapeFace, ShapeLimits, ShapingWorkspace,
    bundled_noto_color_emoji_covers_rune, fixed, is_default_ignorable_rune, parse_face,
    parse_face_collection_with_limit, register_bundled_face, registered_bundled_face,
    registered_bundled_face_backing_digest, registered_bundled_noto_color_emoji,
    registered_bundled_noto_color_emoji_backing_digest,
};
#[cfg(feature = "oracle-fixtures")]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::sync::{Arc, OnceLock};

const REGULAR: &[u8] = include_bytes!("../../d2-rs-text/assets/SourceSansPro-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../d2-rs-text/assets/SourceSansPro-Bold.ttf");
const MONO: &[u8] = include_bytes!("../../d2-rs-text/assets/SourceCodePro-Regular.ttf");

fn noto_bytes() -> &'static [u8] {
    static DATA: OnceLock<Vec<u8>> = OnceLock::new();
    DATA.get_or_init(|| {
        let compressed = include_bytes!("../assets/NotoColorEmoji-COLRv1-v2.051.ttf.br");
        let mut data = Vec::new();
        brotli_decompressor::Decompressor::new(&compressed[..], 4096)
            .read_to_end(&mut data)
            .unwrap();
        data
    })
}

fn noto() -> ParsedFace {
    registered_bundled_noto_color_emoji(noto_bytes(), 0)
        .unwrap()
        .unwrap()
        .clone_read_only()
        .unwrap()
}

fn bundled(data: &[u8]) -> ParsedFace {
    register_bundled_face(data, Sha256::digest(data).into()).unwrap();
    registered_bundled_face(data, 0)
        .unwrap()
        .unwrap()
        .clone_read_only()
        .unwrap()
}

const LIMITS: ShapeLimits = ShapeLimits {
    runes: 100_000,
    faces: 64,
    coverage_checks: 1_000_000,
    runs: 100_000,
    glyphs: 1_000_000,
};

fn shape(
    text: &str,
    size: i32,
    faces: &[(&str, &ParsedFace)],
    limits: ShapeLimits,
) -> Result<d2_rs_fontface::ShapedText, Error> {
    let faces: Vec<ShapeFace<'_>> = faces
        .iter()
        .map(|&(id, face)| ShapeFace { id, face })
        .collect();
    ShapingWorkspace::new().shape_text_transient(&(), text, size, &faces, limits)
}

/// The Source v0.9.0 plan fixture's schema (d2-rs-export colr_plan.rs).
#[cfg(feature = "oracle-fixtures")]
fn plan_json(paint: &COLRv1Paint) -> Value {
    let stops = |line: &d2_rs_fontface::COLRv1ColorLine| {
        line.stops
            .iter()
            .map(|s| json!({"offset": s.offset, "color": [s.color.r, s.color.g, s.color.b, s.color.a]}))
            .collect::<Vec<_>>()
    };
    match paint {
        COLRv1Paint::Layers(l) => {
            json!({"kind": "layers", "paints": l.paints.iter().map(plan_json).collect::<Vec<_>>()})
        }
        COLRv1Paint::Solid(s) => {
            json!({"kind": "solid", "color": [s.color.r, s.color.g, s.color.b, s.color.a]})
        }
        COLRv1Paint::LinearGradient(g) => json!({"kind": "linear", "stops": stops(&g.color_line),
            "points": [g.x0, g.y0, g.x1, g.y1, g.x2, g.y2]}),
        COLRv1Paint::RadialGradient(g) => json!({"kind": "radial", "stops": stops(&g.color_line),
            "circles": [g.x0, g.y0, g.radius0, g.x1, g.y1, g.radius1]}),
        COLRv1Paint::Glyph(g) => {
            json!({"kind": "glyph", "glyph_id": g.glyph_id, "paint": plan_json(&g.paint)})
        }
        COLRv1Paint::Transform(t) => {
            let m = t.matrix;
            json!({"kind": "transform", "matrix": [m.xx, m.yx, m.xy, m.yy, m.dx, m.dy], "paint": plan_json(&t.paint)})
        }
        COLRv1Paint::Composite(c) => json!({"kind": "composite", "source": plan_json(&c.source),
            "backdrop": plan_json(&c.backdrop), "mode": c.mode as u8}),
    }
}

/// Numbers compare by value (the fixture writes integral floats as ints).
#[cfg(feature = "oracle-fixtures")]
fn normalized(value: &Value) -> Value {
    match value {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(items) => Value::Array(items.iter().map(normalized).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), normalized(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn every_bundled_emoji_colrv1_plan_matches_the_pinned_source_fixture() {
    let face = noto();
    // The artifact belongs to d2-rs-export (recipe renderexport-colrv1-plans).
    let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::load_str(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-export"),
        "tests/fixtures/colrv1-plans-source-v090.json",
    ))
    .unwrap();
    assert_eq!(rows.len(), face.outline().num_glyphs() + 2);
    let mut found = 0;
    for row in &rows {
        let glyph = row["glyph_id"].as_u64().unwrap() as u32;
        let actual = face.compile_bundled_noto_color_emoji_colrv1_plan(glyph);
        let expected_error = row["error"].as_str().unwrap();
        if !expected_error.is_empty() {
            assert_eq!(
                actual.unwrap_err().to_string(),
                expected_error,
                "glyph {glyph}"
            );
            continue;
        }
        let actual = actual.unwrap();
        assert_eq!(
            actual.is_some(),
            row["found"].as_bool().unwrap(),
            "glyph {glyph}"
        );
        let Some(plan) = actual else { continue };
        found += 1;
        let expected = &row["plan"];
        assert_eq!(plan.glyph_id, glyph);
        assert_eq!(
            normalized(&plan_json(&plan.root)),
            normalized(&expected["root"]),
            "glyph {glyph}"
        );
        let clip = plan
            .clip
            .map(|c| json!([c.x_min, c.y_min, c.x_max, c.y_max]));
        assert_eq!(
            normalized(&json!(clip)),
            normalized(&expected["clip"]),
            "glyph {glyph}"
        );
        let usage = &expected["usage"];
        assert_eq!(
            [
                plan.usage.paint_nodes,
                plan.usage.max_depth,
                plan.usage.max_layers,
                plan.usage.max_gradient_stops
            ],
            [
                usage["paint_nodes"].as_u64().unwrap() as usize,
                usage["max_depth"].as_u64().unwrap() as usize,
                usage["max_layers"].as_u64().unwrap() as usize,
                usage["max_gradient_stops"].as_u64().unwrap() as usize,
            ],
            "glyph {glyph}"
        );
    }
    assert!(found > 3_000, "only {found} color glyphs compiled");
}

#[test]
fn colrv1_compilation_requires_the_authenticated_bundled_face() {
    let face = bundled(REGULAR);
    assert!(!face.is_bundled_noto_color_emoji());
    assert_eq!(
        face.compile_bundled_noto_color_emoji_colrv1_plan(5)
            .unwrap_err(),
        Error::COLRv1UntrustedFont
    );
    // A same-sized unrelated resource is not claimed.
    let mut tampered = noto_bytes().to_vec();
    tampered[100] ^= 1;
    assert!(
        registered_bundled_noto_color_emoji(&tampered, 0)
            .unwrap()
            .is_none()
    );
    assert!(
        registered_bundled_noto_color_emoji(REGULAR, 0)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        registered_bundled_noto_color_emoji(noto_bytes(), 1)
            .unwrap_err()
            .to_string(),
        "load font face 1: collection has 1 faces"
    );
    // Byte-identical copies authenticate; only the private backing matches
    // the identity-only digest lookup.
    let source = registered_bundled_noto_color_emoji(noto_bytes(), 0)
        .unwrap()
        .unwrap();
    assert!(source.is_bundled_noto_color_emoji());
    assert!(registered_bundled_noto_color_emoji_backing_digest(noto_bytes()).is_none());
    // ParseFace on the exact bytes keeps provenance.
    assert!(
        parse_face(noto_bytes(), 0)
            .unwrap()
            .is_bundled_noto_color_emoji()
    );
}

#[test]
fn emoji_coverage_bounds_and_layers_follow_the_trusted_plan() {
    let face = noto();
    for value in [
        '\u{2705}',
        '\u{1f600}',
        '\u{2764}',
        'A',
        '\u{3042}',
        '\u{10ffff}',
    ] {
        assert_eq!(
            bundled_noto_color_emoji_covers_rune(value),
            face.outline().glyph_index(value).unwrap() != 0,
            "{value:?}"
        );
    }
    assert!(face.supports_renderable_rune('\u{1f600}').unwrap());
    assert!(!face.supports_renderable_rune('A').unwrap());
    let glyph = u32::from(face.outline().glyph_index('\u{1f600}').unwrap());
    let plan = face
        .compile_bundled_noto_color_emoji_colrv1_plan(glyph)
        .unwrap()
        .unwrap();
    let clip = plan.clip.unwrap();
    let (bounds, ink) = face.glyph_render_bounds(glyph, fixed::i(64)).unwrap();
    assert!(ink);
    // Y-up clip reflected into sfnt's Y-down 26.6 space at 64 ppem.
    let scale = f64::from(fixed::i(64)) / f64::from(face.outline().units_per_em());
    assert_eq!(bounds.min.x, (clip.x_min * scale).round() as i32);
    assert_eq!(bounds.min.y, (-clip.y_max * scale).round() as i32);
    assert_eq!(bounds.max.x, (clip.x_max * scale).round() as i32);
    assert_eq!(bounds.max.y, (-clip.y_min * scale).round() as i32);
    // COLRv1 paints are not COLRv0 layers; their base outline is empty, so
    // the digest-neutral path reports the unsupported paint.
    assert_eq!(
        face.colr0_glyph_layers(glyph).unwrap_err().to_string(),
        format!("color glyph {glyph} uses unsupported COLRv1 paint without an outline fallback")
    );
    assert_eq!(face.shaping().glyph_data_kind(glyph), "color");
}

#[test]
fn bundled_registry_authenticates_and_shares_private_backing() {
    assert_eq!(
        register_bundled_face(&[], [1; 32]).unwrap_err().to_string(),
        "invalid bundled font registration"
    );
    let digest: [u8; 32] = Sha256::digest(BOLD).into();
    register_bundled_face(BOLD, digest).unwrap();
    register_bundled_face(BOLD, digest).unwrap();
    assert_eq!(
        register_bundled_face(&BOLD[..10], digest)
            .unwrap_err()
            .to_string(),
        format!(
            "duplicate bundled font digest has size 10, want {}",
            BOLD.len()
        )
    );
    let mut tampered = BOLD.to_vec();
    tampered[200] ^= 1;
    assert!(registered_bundled_face(&tampered, 0).unwrap().is_none());
    let source = registered_bundled_face(BOLD, 0).unwrap().unwrap();
    assert!(Arc::ptr_eq(
        &source,
        &registered_bundled_face(BOLD, 0).unwrap().unwrap()
    ));
    assert_eq!(
        registered_bundled_face(BOLD, 3).unwrap_err().to_string(),
        "load font face 3: collection has 1 faces"
    );
    // The caller's slice is not the registry's private copy.
    assert!(registered_bundled_face_backing_digest(BOLD).is_none());
    // Clones get distinct shaping identities over shared parsed tables.
    let a = source.clone_read_only().unwrap();
    let b = source.clone_read_only().unwrap();
    assert_ne!(a.shaping().id(), b.shaping().id());
    assert!(Arc::ptr_eq(a.shaping().font(), b.shaping().font()));
    assert!(a.supports_renderable_rune('A').unwrap());
    assert!(!a.supports_renderable_rune('\u{4e16}').unwrap());
}

#[test]
fn shaping_limits_and_inputs_report_source_errors() {
    let regular = bundled(REGULAR);
    let mono = bundled(MONO);
    let error = |text: &str, size: i32, faces: &[(&str, &ParsedFace)], limits: ShapeLimits| {
        shape(text, size, faces, limits).unwrap_err().to_string()
    };
    assert_eq!(
        error("a", 0, &[("r", &regular)], LIMITS),
        "d2fonts: shaping size must be positive"
    );
    assert_eq!(
        error(
            "a",
            64,
            &[("r", &regular)],
            ShapeLimits {
                glyphs: 0,
                ..LIMITS
            }
        ),
        "d2fonts: every shaping limit must be positive"
    );
    assert_eq!(
        error("a", 64, &[], LIMITS),
        "d2fonts: shaping requires at least one font face"
    );
    assert_eq!(
        error(
            "a",
            64,
            &[("r", &regular), ("m", &mono)],
            ShapeLimits { faces: 1, ..LIMITS }
        ),
        "d2fonts: font face count 2 exceeds limit 1"
    );
    assert_eq!(
        error("a", 64, &[("r", &regular), ("again", &regular)], LIMITS),
        "d2fonts: font face 1 (\"again\") duplicates an earlier shaping face"
    );
    assert_eq!(
        error(
            "abcd",
            64,
            &[("r", &regular)],
            ShapeLimits { runes: 3, ..LIMITS }
        ),
        "d2fonts: text rune count exceeds per-run limit 3"
    );
    assert_eq!(
        error(
            "abcd",
            64,
            &[("r", &regular)],
            ShapeLimits {
                coverage_checks: 2,
                ..LIMITS
            }
        ),
        "d2fonts: text font coverage checks exceed limit 2"
    );
    assert_eq!(
        error(
            "ab \u{5d0}\u{5d1}",
            64,
            &[("r", &regular)],
            ShapeLimits { runs: 1, ..LIMITS }
        ),
        "d2fonts: text shaping run count exceeds limit 1"
    );
    assert_eq!(
        error(
            "abcd",
            64,
            &[("r", &regular)],
            ShapeLimits {
                glyphs: 3,
                ..LIMITS
            }
        ),
        "d2fonts: shaped glyph count exceeds limit 3"
    );
    let empty = shape("", 64, &[("r", &regular)], LIMITS).unwrap();
    assert_eq!((empty.glyphs.len(), empty.runes, empty.runs), (0, 0, 0));
    let canceled = ShapingWorkspace::new().shape_text_transient(
        &|| Err(Error::Canceled("context canceled".into())),
        "a",
        64,
        &[ShapeFace {
            id: "r",
            face: &regular,
        }],
        LIMITS,
    );
    assert_eq!(canceled.unwrap_err().to_string(), "context canceled");
}

#[test]
fn missing_clusters_use_the_first_drawable_placeholder() {
    let regular = bundled(REGULAR);
    let shaped = shape("a\u{4e16}b", fixed::i(16), &[("r", &regular)], LIMITS).unwrap();
    // Source Sans Pro omits U+FFFD and U+25A1; U+2610 is the first placeholder.
    let placeholder = u32::from(regular.outline().glyph_index('\u{2610}').unwrap());
    let glyph = shaped.glyphs[1];
    assert_eq!(
        (glyph.id, glyph.source, glyph.source_index),
        (placeholder, 0x4e16, 1)
    );
    assert_eq!(glyph.position_y, 0.0);
    let advance = regular
        .outline()
        .glyph_advance(placeholder as u16, fixed::i(16), Hinting::None)
        .unwrap();
    assert_eq!(glyph.advance, f64::from(advance) / 64.0);
    assert_eq!(
        shaped.glyphs[2].position_x,
        glyph.position_x + glyph.advance
    );
    // Coverage counts one check per distinct (face, rune) per call:
    // a, U+4E16, then the placeholders U+FFFD, U+25A1, U+2610, then b.
    assert_eq!(shaped.coverage_checks, 6);
}

#[test]
fn fallback_runs_keep_grapheme_affinity_and_face_indexes() {
    let regular = bundled(REGULAR);
    let emoji = noto();
    let shaped = shape(
        "ok \u{1f44d}\u{1f3fd} \u{1f1fa}\u{1f1f8}",
        fixed::i(16),
        &[("r", &regular), ("e", &emoji)],
        LIMITS,
    )
    .unwrap();
    let faces: Vec<usize> = shaped.glyphs.iter().map(|g| g.face).collect();
    assert_eq!(faces[..3], [0, 0, 0]);
    assert!(faces[3..].iter().filter(|&&f| f == 1).count() >= 2);
    assert!(
        shaped
            .glyphs
            .iter()
            .filter(|g| g.face == 1)
            .all(|g| g.has_ink || g.source == 0x20)
    );
    let pen: f64 = shaped.glyphs.iter().map(|g| g.advance).sum();
    assert_eq!(shaped.advance, pen);
}

#[test]
fn outlines_follow_sfnt_scaling_and_contours() {
    let face = bundled(REGULAR);
    let outline = face.outline();
    let h = outline.glyph_index('H').unwrap();
    let segments = outline.load_glyph(h, fixed::i(64)).unwrap();
    assert_eq!(segments.first().unwrap().op, SegmentOp::MoveTo);
    // Each contour closes with an explicit segment back to its start.
    let start = segments[0].args[0];
    let close = segments
        .iter()
        .position(|s| s.op == SegmentOp::MoveTo && s.args[0] != start)
        .unwrap_or(segments.len());
    assert_eq!(segments[close - 1].args[0], start);
    let (bounds, advance) = outline
        .glyph_bounds(h, fixed::i(64), Hinting::None)
        .unwrap();
    assert_eq!(bounds, sfnt::segments_bounds(&segments));
    assert_eq!(
        advance,
        outline
            .glyph_advance(h, fixed::i(64), Hinting::None)
            .unwrap()
    );
    assert_eq!(
        outline.load_glyph(u16::MAX, fixed::i(64)).unwrap_err(),
        sfnt::Error::NotFound
    );
    let space = outline.glyph_index(' ').unwrap();
    assert!(outline.load_glyph(space, fixed::i(64)).unwrap().is_empty());
    let (_, ink) = face
        .glyph_render_bounds(u32::from(space), fixed::i(64))
        .unwrap();
    assert!(!ink);
    let m = outline.metrics(fixed::i(64), Hinting::Full);
    assert_eq!(m.ascent & 63, 0);
}

#[test]
fn collection_parsing_rejects_invalid_and_oversized_inputs() {
    assert_eq!(
        parse_face_collection_with_limit(&[], 4)
            .err()
            .unwrap()
            .to_string(),
        "empty font data"
    );
    assert_eq!(
        parse_face_collection_with_limit(REGULAR, 0)
            .err()
            .unwrap()
            .to_string(),
        "font face limit must be positive"
    );
    let mut otto = REGULAR.to_vec();
    otto[..4].copy_from_slice(b"OTTO");
    assert_eq!(
        parse_face(&otto, 0).unwrap_err().to_string(),
        // x/image/font/sfnt/sfnt.go parseMaxp requires six bytes for OTTO.
        "load outline font face 0: sfnt: invalid maxp table"
    );
    // A two-face TTC header over the same font.
    let mut ttc = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
    let offset = 20u32;
    ttc.extend(offset.to_be_bytes());
    ttc.extend(offset.to_be_bytes());
    let mut font = REGULAR.to_vec();
    let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
    for index in 0..tables {
        let at = 12 + index * 16 + 8;
        let value = u32::from_be_bytes(font[at..at + 4].try_into().unwrap()) + offset;
        font[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    ttc.extend(font);
    assert_eq!(
        parse_face_collection_with_limit(&ttc, 1).err().unwrap(),
        Error::FaceCountLimit { count: 2, limit: 1 }
    );
    let collection = parse_face_collection_with_limit(&ttc, 4).unwrap();
    assert_eq!(collection.num_faces(), 2);
    assert_eq!(
        collection.face(1).unwrap().outline().num_glyphs(),
        bundled(REGULAR).outline().num_glyphs()
    );
    assert_eq!(
        collection.face(2).unwrap_err().to_string(),
        "load font face 2: collection has 2 faces"
    );
}

#[test]
fn default_ignorables_follow_the_harfbuzz_ranges() {
    for value in [
        '\u{ad}',
        '\u{34f}',
        '\u{200b}',
        '\u{200f}',
        '\u{2060}',
        '\u{fe0f}',
        '\u{feff}',
        '\u{e0001}',
    ] {
        assert!(is_default_ignorable_rune(value), "{value:?}");
    }
    // Hangul fillers, Mongolian FVS4 and prepended concatenation marks stay visible.
    for value in [
        '\u{115f}',
        '\u{3164}',
        '\u{180f}',
        '\u{600}',
        '\u{6dd}',
        'a',
        '\u{1d17b}',
    ] {
        assert!(!is_default_ignorable_rune(value), "{value:?}");
    }
}
