// SPDX-License-Identifier: MPL-2.0
//! Original calls captured from the pinned Go mesh builder. Complete retained
//! surface bytes and premultiplied pixels both matter to subsequent meshes.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_isometric::img::{
    rich_labels::{RichLabelPainter, rich_source_size},
    text::LabelTextStyle,
    vector_surface::{native_vector_context, native_vector_for_texture},
    vector_surface_svg::native_surface_svg,
};
use d2_rs_isometric::target::{ClassField, Shape};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[cfg(feature = "oracle-fixtures")]
#[test]
fn rich_label_pixels_viewport_and_surface_match_actual_go_boards() {
    let fixtures: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/rich-labels.json")).unwrap();
    for fixture in fixtures.as_array().unwrap() {
        let mut painters = std::collections::HashMap::new();
        for call in fixture["calls"].as_array().unwrap() {
            let id = call["painter"].as_u64().unwrap();
            let p = painters.entry(id).or_insert_with(|| {
                let mut p =
                    RichLabelPainter::new(call["count"].as_i64().unwrap(), native_vector_context())
                        .unwrap();
                p.primary = call["primary"].as_str().unwrap().into();
                p.mono = call["mono"].as_str().unwrap().into();
                p.theme_id = call["themeID"].as_i64();
                p.theme_overrides = serde_json::from_value(call["overrides"].clone()).unwrap();
                p.output_density = call["density"].as_f64().unwrap();
                p
            });
            let style: LabelTextStyle = serde_json::from_value(call["style"].clone()).unwrap();
            let shape: Shape = serde_json::from_value(call["value"].clone()).unwrap();
            let texture = p.texture(&shape, &style).unwrap().unwrap();
            let surface = native_vector_for_texture(&p.vectors, &texture).unwrap();
            let actual = json!({"width":texture.width,"height":texture.height,
                "pixels":format!("{:x}", Sha256::digest(&*texture.pixels.borrow())),
                "surface":native_surface_svg(&surface,"text").unwrap()});
            let mut expected = call["want"].clone();
            // ViewBox is a typed float64 model. encoding/json writes integral
            // floats as integers; compare their exact model values, as the
            // scoreboard does, and keep every SVG byte and pixel hash strict.
            let view_box: d2_rs_scene::Box = serde_json::from_value(
                expected.as_object_mut().unwrap().remove("viewBox").unwrap(),
            )
            .unwrap();
            assert_eq!(p.last_document.as_ref().unwrap().view_box, view_box);
            assert_eq!(actual, expected, "{}", fixture["board"]);
        }
    }
}

#[test]
fn rich_source_admission_counts_all_fields_and_constraints() {
    let mut shape = Shape::default();
    shape.text.label = "a".repeat(65536);
    assert_eq!(rich_source_size(&shape).unwrap(), (65536, 0));
    shape.fields = Some(vec![ClassField {
        name: "b".into(),
        ..Default::default()
    }]);
    assert_eq!(
        rich_source_size(&shape).unwrap_err(),
        "isometric rich label exceeds source byte budget or contains invalid UTF-8"
    );
    shape.text.label.clear();
    shape.fields = Some(vec![ClassField::default(); 513]);
    assert_eq!(
        rich_source_size(&shape).unwrap_err(),
        "isometric rich label exceeds 512 structured rows"
    );
}

#[test]
fn rich_invalid_style_does_not_consume_allocation() {
    assert!(RichLabelPainter::new(-1, None).is_err());
    assert!(RichLabelPainter::new(19001, None).is_err());
    let mut p = RichLabelPainter::new(1, None).unwrap();
    let mut shape = Shape {
        type_: "code".into(),
        ..Default::default()
    };
    shape.text.label = "a".into();
    let mut style = LabelTextStyle {
        width: 1.0,
        depth: 0.2,
        pixel_scale: 0.01,
        font_size: 16.0,
        opacity: 1.0,
        ..Default::default()
    };
    style.opacity = f64::NAN;
    assert_eq!(
        p.texture(&shape, &style).unwrap_err(),
        "isometric rich label has invalid surface dimensions or style"
    );
    assert_eq!(p.used, 0);
    style.opacity = 1.0;
    assert!(p.texture(&shape, &style).is_ok());
    assert_eq!(
        p.texture(&shape, &style).unwrap_err(),
        "isometric rich label allocation exceeds its declared count"
    );
}
