// SPDX-License-Identifier: MPL-2.0
//! Actual calls from three previously failing upstream mesh boards.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_isometric::img::{
    text::{LabelTextStyle, TextPainter},
    vector_surface::{native_vector_context, native_vector_for_texture},
    vector_surface_svg::native_surface_svg,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[cfg(feature = "oracle-fixtures")]
#[test]
fn plain_label_layout_pixels_and_retained_svg_match_go_boards() {
    check_fixtures(d2_rs_testdata::oracle_fixture!("tests/fixtures/plain-labels.json"));
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn fallback_layout_color_glyphs_and_surface_match_go_emoji_board() {
    check_fixtures(d2_rs_testdata::oracle_fixture!("tests/fixtures/fallback-labels.json"));
}
fn check_fixtures(source: &str) {
    let fixtures: Value = serde_json::from_str(source).unwrap();
    for fixture in fixtures.as_array().unwrap() {
        let mut painters = std::collections::HashMap::new();
        for call in fixture["calls"].as_array().unwrap() {
            let id = call["painter"].as_u64().unwrap();
            let p = painters.entry(id).or_insert_with(|| {
                let mut p =
                    TextPainter::new(call["count"].as_i64().unwrap(), native_vector_context())
                        .unwrap();
                p.primary = call["primary"].as_str().unwrap().into();
                p.mono = call["mono"].as_str().unwrap().into();
                p.output_density = call["density"].as_f64().unwrap();
                p
            });
            let style: LabelTextStyle = serde_json::from_value(call["style"].clone()).unwrap();
            let (texture, layout) = p.texture(call["value"].as_str().unwrap(), &style).unwrap();
            let t = texture.unwrap();
            let surface = native_vector_for_texture(&p.vectors, &t).unwrap();
            let actual = json!({"width":t.width,"height":t.height,"pixels":format!("{:x}",Sha256::digest(&*t.pixels.borrow())),"layout":layout,"surface":native_surface_svg(&surface,"text").unwrap()});
            assert_eq!(actual, call["want"], "{}", fixture["board"]);
        }
    }
}
#[test]
fn plain_label_admission_errors_do_not_consume_label_allocation() {
    assert!(TextPainter::new(-1, None).is_err());
    assert!(TextPainter::new(19001, None).is_err());
    let mut painter = TextPainter::new(1, None).unwrap();
    let mut style = LabelTextStyle {
        width: 1.0,
        depth: 0.2,
        pixel_scale: 0.01,
        font_size: 16.0,
        opacity: 1.0,
        ..Default::default()
    };
    style.width = f64::NAN;
    assert_eq!(
        painter.texture("a", &style).unwrap_err(),
        "isometric text has an invalid physical print area or pixel scale"
    );
    style.width = 1.0;
    assert!(painter.texture("a", &style).is_ok());
    assert_eq!(
        painter.texture("a", &style).unwrap_err(),
        "isometric text label allocation exceeds its declared count"
    );
}
