// SPDX-License-Identifier: MPL-2.0
//! Original public SVG calls and source-backed deadline boards, without partial output.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_core::{CancellationToken, LayoutError};
use d2_rs_isometric::{
    img::{
        context::{RenderContext, RenderError},
        geometry_core::MeshGeometry,
        mesh::{Material, Vertex, material},
        render::{Options, render_svg_with_context},
        svg::{NativeScene, native_scene_svg_with_context},
        svg_order::{OrderUnit, order_paint_units_with_context},
        svg_visibility::{SVG_DEFAULT_VISIBILITY_LIMITS, svg_visible_faces_with_context},
        text::{LabelTextStyle, TextPainter},
    },
    target::Diagram,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_svg_calls_and_deadline_boards() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/svg-api.json"
    ))
    .unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 11);
    for (index, call) in calls.iter().enumerate() {
        let r = &call["request"];
        let diagram: Option<Diagram> = serde_json::from_value(r["diagram"].clone()).unwrap();
        let before = serde_json::to_value(&diagram).unwrap();
        let o = &r["options"];
        let options = Options {
            width: o["Width"].as_i64().unwrap_or(0),
            height: o["Height"].as_i64().unwrap_or(0),
            fit_content: o["FitContent"].as_bool().unwrap_or(false),
            timeout_ns: o["Timeout"].as_i64().unwrap_or(0),
            ..Default::default()
        };
        let token = CancellationToken::default();
        if r["context"] == "context canceled" {
            token.cancel();
        }
        let result = render_svg_with_context(Some(&token), diagram.as_ref(), Some(&options));
        if let Some(expected) = call["want"]["sha256"].as_str() {
            let svg = result.unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(svg.as_bytes())),
                expected,
                "original call {index}"
            );
            assert_eq!(svg.len() as u64, call["want"]["bytes"].as_u64().unwrap());
        } else {
            let error = result.unwrap_err();
            assert_eq!(
                error.to_string(),
                call["want"]["error"].as_str().unwrap(),
                "original call {index}"
            );
            if options.timeout_ns == 1 {
                assert!(matches!(
                    error,
                    RenderError::Context(LayoutError::DeadlineExceeded)
                ));
                assert!(
                    !token.is_cancelled(),
                    "renderer deadline escaped into its caller"
                );
            }
        }
        assert_eq!(serde_json::to_value(&diagram).unwrap(), before);
    }
}

#[test]
fn svg_admission_and_typed_caller_causes() {
    let token = CancellationToken::default();
    token.cancel();
    let error = render_svg_with_context(Some(&token), None, None).unwrap_err();
    assert!(matches!(
        error,
        RenderError::Context(LayoutError::Cancelled)
    ));
    let error = render_svg_with_context(
        Some(&token),
        None,
        Some(&Options {
            width: 1,
            height: 1,
            ..Default::default()
        }),
    )
    .unwrap_err();
    assert!(
        matches!(error, RenderError::Message(_)),
        "admission precedes context"
    );
    let token = CancellationToken::default();
    token.expire();
    let error = render_svg_with_context(Some(&token), None, None).unwrap_err();
    assert!(matches!(
        error,
        RenderError::Context(LayoutError::DeadlineExceeded)
    ));
    assert!(
        std::error::Error::source(&error)
            .unwrap()
            .is::<LayoutError>()
    );
}

#[test]
fn canceled_geometry_and_cached_text_do_not_publish() {
    let token = CancellationToken::default();
    let context = RenderContext::from_token(&token);
    let mut text = TextPainter::new(2, None).unwrap();
    text.context = context.clone();
    let style = LabelTextStyle {
        width: 1.0,
        depth: 0.3,
        pixel_scale: 0.01,
        font_size: 16.0,
        opacity: 1.0,
        color: d2_rs_scene::Nrgba::new(0, 0, 0, 255),
        ..Default::default()
    };
    assert!(text.texture("cached", &style).unwrap().0.is_some());
    token.cancel();
    assert_eq!(
        text.texture("cached", &style).unwrap_err(),
        "context canceled"
    );
    let mut geometry = MeshGeometry::default();
    geometry.context = context;
    let m = material(Material {
        color: d2_rs_scene::Nrgba::new(0, 0, 0, 255),
        ..Default::default()
    });
    geometry.triangle(
        Vertex::default(),
        Vertex::default(),
        Vertex::default(),
        &m,
        false,
    );
    assert!(geometry.triangles.is_empty());
    assert_eq!(geometry.err.as_deref(), Some("context canceled"));
}

#[test]
fn svg_visibility_order_and_serialization_observe_the_same_context() {
    let token = CancellationToken::default();
    let context = RenderContext::from_token(&token);
    let mut scene = NativeScene {
        triangles: vec![],
        camera: Default::default(),
        width: 64,
        height: 64,
        background: Default::default(),
        background_paint: Some(Default::default()),
        links: vec![],
        ..Default::default()
    };
    // An actual writer callback cancels while serializing a retained surface.
    let cancel = token.clone();
    let error = native_scene_svg_with_context(
        &mut scene,
        &move |_, _| {
            cancel.cancel();
            Ok("<path/>".into())
        },
        &context,
    )
    .unwrap_err();
    assert_eq!(error, "context canceled");
    assert_eq!(
        svg_visible_faces_with_context(&[], SVG_DEFAULT_VISIBILITY_LIMITS, &context).unwrap_err(),
        "context canceled"
    );
    let units = vec![OrderUnit::default()];
    assert_eq!(
        order_paint_units_with_context(&units, SVG_DEFAULT_VISIBILITY_LIMITS, &context)
            .unwrap_err(),
        "context canceled"
    );
}
