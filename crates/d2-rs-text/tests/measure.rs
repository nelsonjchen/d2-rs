// SPDX-License-Identifier: MPL-2.0
use d2_rs_text::measure::{
    TrueTypeFont, bundled_source_sans_pro_bold, bundled_source_sans_pro_regular,
    bundled_source_sans_pro_semibold,
};

fn be16(value: u16) -> [u8; 2] {
    value.to_be_bytes()
}

fn be32(value: u32) -> [u8; 4] {
    value.to_be_bytes()
}

fn synthetic_font() -> Vec<u8> {
    let mut tables = Vec::<([u8; 4], Vec<u8>)>::new();
    let mut head = vec![0; 20];
    head[18..20].copy_from_slice(&be16(1000));
    tables.push((*b"head", head));
    let mut hhea = vec![0; 36];
    hhea[4..6].copy_from_slice(&be16(800));
    hhea[6..8].copy_from_slice(&be16((-200i16) as u16));
    hhea[34..36].copy_from_slice(&be16(2));
    tables.push((*b"hhea", hhea));
    let mut maxp = vec![0; 6];
    maxp[4..6].copy_from_slice(&be16(2));
    tables.push((*b"maxp", maxp));
    let mut hmtx = Vec::new();
    hmtx.extend_from_slice(&be16(250));
    hmtx.extend_from_slice(&be16(0));
    hmtx.extend_from_slice(&be16(500));
    hmtx.extend_from_slice(&be16(0));
    tables.push((*b"hmtx", hmtx));
    let mut cmap = vec![0, 0, 0, 1];
    cmap.extend_from_slice(&be16(3));
    cmap.extend_from_slice(&be16(1));
    cmap.extend_from_slice(&be32(12));
    cmap.extend_from_slice(&be16(4));
    cmap.extend_from_slice(&be16(32));
    cmap.extend_from_slice(&be16(0));
    cmap.extend_from_slice(&be16(4));
    cmap.extend_from_slice(&be16(4));
    cmap.extend_from_slice(&be16(1));
    cmap.extend_from_slice(&be16(0));
    cmap.extend_from_slice(&be16(0x0041));
    cmap.extend_from_slice(&be16(0xffff));
    cmap.extend_from_slice(&be16(0));
    cmap.extend_from_slice(&be16(0x0041));
    cmap.extend_from_slice(&be16(0xffff));
    cmap.extend_from_slice(&be16((-64i16) as u16));
    cmap.extend_from_slice(&be16(1));
    cmap.extend_from_slice(&be16(0));
    cmap.extend_from_slice(&be16(0));
    tables.push((*b"cmap", cmap));

    let directory_len = 12 + tables.len() * 16;
    let mut output = vec![0; directory_len];
    output[0..4].copy_from_slice(b"\0\x01\0\0");
    output[4..6].copy_from_slice(&be16(tables.len() as u16));
    let mut offset = directory_len;
    for (index, (tag, bytes)) in tables.into_iter().enumerate() {
        let entry = 12 + index * 16;
        output[entry..entry + 4].copy_from_slice(&tag);
        output[entry + 8..entry + 12].copy_from_slice(&be32(offset as u32));
        output[entry + 12..entry + 16].copy_from_slice(&be32(bytes.len() as u32));
        output.extend_from_slice(&bytes);
        offset += bytes.len();
    }
    output
}

#[test]
fn parses_format_four_and_measures_tabs_and_lines() {
    let font = TrueTypeFont::parse(&synthetic_font()).unwrap();
    let metrics = font.measure("AA\nA\tA", 10.0);
    assert_eq!(metrics.lines, 2);
    assert_eq!(metrics.width, 15.0);
    assert_eq!(metrics.height, 20.0);
}

#[test]
fn rejects_truncated_fonts_without_panicking() {
    assert!(TrueTypeFont::parse(&[]).is_err());
}

#[test]
fn bundled_source_sans_pro_matches_d2_default_face() {
    let font = bundled_source_sans_pro_regular().unwrap();
    let metrics = font.measure("a", 16.0);
    assert_eq!(metrics.lines, 1);
    assert_eq!(metrics.width, 7.0);
    assert_eq!(metrics.height, 20.125);
}

#[test]
fn bundled_source_sans_pro_bold_matches_d2_default_node_labels() {
    let font = bundled_source_sans_pro_bold().unwrap();
    let metrics = font.measure("a", 16.0);
    assert_eq!(metrics.lines, 1);
    assert_eq!(metrics.width, 8.0);
    assert_eq!(metrics.height, 20.125);
}

#[test]
fn bundled_font_uses_d2_atlas_replacement_for_unlisted_symbols() {
    let regular = bundled_source_sans_pro_regular().unwrap();
    let bold = bundled_source_sans_pro_bold().unwrap();
    // The v0.9.0 atlas contains ASCII, Latin-1, and geometric-shape runes.
    // Other symbols are rendered through U+FFFD even when the TTF cmap has a
    // native glyph, which is part of D2's legacy measurement contract.
    assert_eq!(regular.measure("→", 14.0).width, 7.0);
    assert_eq!(bold.measure("→", 14.0).width, 8.0);
    assert_eq!(regular.measure("⬤", 14.0).width, 7.0);
    assert_eq!(bold.measure("⬤", 14.0).width, 8.0);
}

#[test]
fn markdown_bounds_with_dot_preserves_origin_and_trailing_ink() {
    let semibold = bundled_source_sans_pro_semibold().unwrap();
    // D2 v0.9.0's boundsWithDot ruler keeps the origin in the union. The
    // semibold H2 witness therefore includes both the leading side bearing
    // and the final glyph's ink beyond its advance.
    assert_eq!(
        semibold.measure_with_dot("Raster rendering", 24.0).width,
        173.453125
    );
}

#[test]
fn markdown_advance_matches_go_measure_string_rounding() {
    let bold = bundled_source_sans_pro_bold().unwrap();
    let regular = bundled_source_sans_pro_regular().unwrap();
    // These are the 26.6 fixed-point advances reported by the pinned
    // x/image face in D2 v0.9.0, including the legacy kern table rounding.
    assert_eq!(bold.measure_advance("bold italic", 16.0), 70.453125);
    assert_eq!(bold.measure_advance("both", 16.0), 33.3125);
    assert_eq!(regular.measure_advance(" and ", 16.0), 32.09375);
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn bounds_with_dot_matches_original_go_cursor_and_ink_union() {
    let controls: serde_json::Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/bounds-dot-source-v090.json")).unwrap();
    assert_eq!(controls.as_array().unwrap().len(), 192);
    for row in controls.as_array().unwrap() {
        let family = row["family"].as_str().unwrap();
        let style = row["style"].as_str().unwrap();
        let asset = match (family, style) {
            ("SourceSansPro", "regular") => {
                include_bytes!("../assets/SourceSansPro-Regular.ttf").as_slice()
            }
            ("SourceSansPro", "semibold") => {
                include_bytes!("../assets/SourceSansPro-Semibold.ttf").as_slice()
            }
            ("SourceSansPro", "bold") => {
                include_bytes!("../assets/SourceSansPro-Bold.ttf").as_slice()
            }
            ("SourceSansPro", "italic") => {
                include_bytes!("../assets/SourceSansPro-Italic.ttf").as_slice()
            }
            ("SourceCodePro", "regular") => {
                include_bytes!("../assets/SourceCodePro-Regular.ttf").as_slice()
            }
            ("SourceCodePro", "semibold") => {
                include_bytes!("../assets/SourceCodePro-Semibold.ttf").as_slice()
            }
            ("SourceCodePro", "bold") => {
                include_bytes!("../assets/SourceCodePro-Bold.ttf").as_slice()
            }
            ("SourceCodePro", "italic") => {
                include_bytes!("../assets/SourceCodePro-Italic.ttf").as_slice()
            }
            ("HandDrawn", "regular" | "italic") => {
                include_bytes!("../assets/FuzzyBubbles-Regular.ttf").as_slice()
            }
            ("HandDrawn", "semibold" | "bold") => {
                include_bytes!("../assets/FuzzyBubbles-Bold.ttf").as_slice()
            }
            _ => panic!("unknown Source font {row}"),
        };
        let font = TrueTypeFont::parse(asset).unwrap();
        let actual =
            font.measure_with_dot(row["text"].as_str().unwrap(), row["size"].as_f64().unwrap());
        assert_eq!(
            actual.width.to_bits(),
            row["width"].as_f64().unwrap().to_bits(),
            "width {row}"
        );
        assert_eq!(
            actual.height.to_bits(),
            row["height"].as_f64().unwrap().to_bits(),
            "height {row}"
        );
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn code_bounds_match_original_control_pen_and_zero_origin() {
    let controls: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/markdown-code-bounds-source-v090.json"
    ))
    .unwrap();
    assert_eq!(controls.as_array().unwrap().len(), 3960);
    for row in controls.as_array().unwrap() {
        let family = row["family"].as_str().unwrap();
        let style = row["style"].as_str().unwrap();
        let asset = match (family, style) {
            ("SourceSansPro", "regular") => {
                include_bytes!("../assets/SourceSansPro-Regular.ttf").as_slice()
            }
            ("SourceSansPro", "semibold") => {
                include_bytes!("../assets/SourceSansPro-Semibold.ttf").as_slice()
            }
            ("SourceSansPro", "bold") => {
                include_bytes!("../assets/SourceSansPro-Bold.ttf").as_slice()
            }
            ("SourceSansPro", "italic") => {
                include_bytes!("../assets/SourceSansPro-Italic.ttf").as_slice()
            }
            ("SourceCodePro", "regular") => {
                include_bytes!("../assets/SourceCodePro-Regular.ttf").as_slice()
            }
            ("SourceCodePro", "semibold") => {
                include_bytes!("../assets/SourceCodePro-Semibold.ttf").as_slice()
            }
            ("SourceCodePro", "bold") => {
                include_bytes!("../assets/SourceCodePro-Bold.ttf").as_slice()
            }
            ("SourceCodePro", "italic") => {
                include_bytes!("../assets/SourceCodePro-Italic.ttf").as_slice()
            }
            ("HandDrawn", "regular" | "italic") => {
                include_bytes!("../assets/FuzzyBubbles-Regular.ttf").as_slice()
            }
            ("HandDrawn", "semibold" | "bold") => {
                include_bytes!("../assets/FuzzyBubbles-Bold.ttf").as_slice()
            }
            _ => panic!("unknown Source font {row}"),
        };
        let font = TrueTypeFont::parse(asset).unwrap();
        let actual = font.measure_markdown_code(
            row["text"].as_str().unwrap(),
            row["size"].as_f64().unwrap(),
            row["factor"].as_f64().unwrap(),
        );
        assert_eq!(
            actual.width.to_bits(),
            row["width"].as_f64().unwrap().to_bits(),
            "width {row}"
        );
        assert_eq!(
            actual.height.to_bits(),
            row["height"].as_f64().unwrap().to_bits(),
            "height {row}"
        );
    }
}
