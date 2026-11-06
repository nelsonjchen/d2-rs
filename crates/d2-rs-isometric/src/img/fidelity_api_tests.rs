// SPDX-License-Identifier: MPL-2.0
//! Original input-only replay; this entire module compiles only in tests.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::*;
use d2_rs_fontface::{
    COLRv1ColorLine, COLRv1ColorStop, COLRv1Composite, COLRv1CompositeMode, COLRv1LinearGradient,
    COLRv1Paint as C, COLRv1Solid,
};
use serde_json::{Value, json};
use std::cell::RefCell;
fn paint(v: &Value) -> C {
    let color = |v: &Value| {
        let c: Nrgba = serde_json::from_value(v.clone()).unwrap();
        d2_rs_fontface::Nrgba {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
    };
    if !v["solid"].is_null() {
        return C::Solid(COLRv1Solid {
            color: color(&v["solid"]["Color"]),
        });
    }
    if !v["linear"].is_null() {
        let p = &v["linear"];
        assert_eq!(p["ColorLine"].as_object().unwrap().len(), 1);
        return C::LinearGradient(COLRv1LinearGradient {
            x0: p["X0"].as_f64().unwrap(),
            y0: p["Y0"].as_f64().unwrap(),
            x1: p["X1"].as_f64().unwrap(),
            y1: p["Y1"].as_f64().unwrap(),
            x2: p["X2"].as_f64().unwrap(),
            y2: p["Y2"].as_f64().unwrap(),
            color_line: COLRv1ColorLine {
                stops: p["ColorLine"]["Stops"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| COLRv1ColorStop {
                        offset: s["Offset"].as_f64().unwrap(),
                        color: color(&s["Color"]),
                    })
                    .collect(),
            },
        });
    }
    let p = &v["composite"];
    assert_eq!(p["mode"], 0);
    C::Composite(COLRv1Composite {
        source: std::boxed::Box::new(paint(&p["source"])),
        backdrop: std::boxed::Box::new(paint(&p["backdrop"])),
        mode: COLRv1CompositeMode::SrcIn,
    })
}
fn context(v: &Value) -> super::super::context::RenderContext {
    let token = d2_rs_core::CancellationToken::default();
    if v["canceled"].as_bool().unwrap() {
        token.cancel();
    }
    super::super::context::RenderContext::from_token(&token)
}
fn replay(v: &Value) -> Value {
    if v["op"] == "shadow" || v["op"] == "validate" {
        return super::super::raster::api_tests::replay_fidelity(v);
    }
    let doc: Document = if v["document"].is_null() {
        Document::default()
    } else {
        serde_json::from_value(v["document"].clone()).unwrap()
    };
    let prefix = v["prefix"].as_str().unwrap();
    if v["op"] == "svg" {
        let surface = VectorSurface {
            document: Rc::new(RefCell::new(doc)),
            ..Default::default()
        };
        return match native_surface_svg_with_context(&surface, prefix, &context(v)) {
            Ok(s) => json!({"error":"","svg":s}),
            Err(e) => json!({"error":e,"svg":""}),
        };
    }
    assert_eq!(v["op"], "paint");
    let before = &v["writer"];
    assert_eq!(before["error"], "");
    let mut w = SurfaceSvg {
        doc: &doc,
        context: context(v),
        prefix,
        defs: before["defs"].as_str().unwrap().into(),
        next: before["next"].as_u64().unwrap() as usize,
        nodes: before["nodes"].as_u64().unwrap() as usize,
        commands: before["commands"].as_u64().unwrap() as usize,
        err: None,
        fonts: HashMap::new(),
        glyphs: HashMap::new(),
        transform: serde_json::from_value(before["transform"].clone()).unwrap(),
        linear_gradients: before["linear"].as_bool().unwrap(),
    };
    // These original paint plans have no glyph/font branch. The native method
    // takes a nonoptional face; a bundled face supplies that unused argument.
    let face = d2_rs_fontface::parse_face(
        d2_rs_text::d2fonts::font_face(
            d2_rs_text::d2fonts::FontFamily::SourceSansPro
                .font(d2_rs_text::d2fonts::FontStyle::Regular),
        ),
        0,
    )
    .unwrap();
    let body = w.original_test_color_paint(
        &face,
        &paint(&v["paint"]),
        serde_json::from_value(v["box"].clone()).unwrap(),
        serde_json::from_value(v["matrix"].clone()).unwrap(),
        v["depth"].as_u64().unwrap() as usize,
    );
    json!({"body":body,"writer":{"defs":w.defs,"next":w.next,"nodes":w.nodes,"commands":w.commands,"transform":w.transform,"linear":w.linear_gradients,"error":w.err.unwrap_or_default()}})
}
fn compact(mut got: Value) -> Value {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    if let Some(s) = got.get_mut("shadow").and_then(Value::as_object_mut) {
        for key in ["depth", "opacity"] {
            let raw = base64::engine::general_purpose::STANDARD
                .decode(s[key].as_str().unwrap())
                .unwrap();
            s.insert(
                key.into(),
                json!({"length":raw.len(),"sha256":format!("{:x}",Sha256::digest(raw))}),
            );
        }
    }
    got
}
// JSON has one numeric domain: Go emits integral floats as integers. Compare
// their exact values without an epsilon; all integer fields here fit in 53 bits.
fn equal_json(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a == b || a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal_json(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, a)| b.get(k).is_some_and(|b| equal_json(a, b)))
        }
        _ => a == b,
    }
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let Ok(path) = std::env::var("D2RS_GEOMETRY_REQUEST") else {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/fidelity-api.json"
        ))
        .unwrap();
        for c in fixture["cases"].as_array().unwrap() {
            for op in c["operations"].as_array().unwrap() {
                assert!(
                    equal_json(&compact(replay(&op["request"])), &op["got"]),
                    "{}",
                    c["name"]
                );
            }
        }
        return;
    };
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let out: std::vec::Vec<_> = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| replay(&o["request"]))
                .collect::<std::vec::Vec<_>>()
        })
        .collect();
    std::fs::write(
        std::env::var("D2RS_GEOMETRY_OUTPUT").unwrap(),
        serde_json::to_vec(&out).unwrap(),
    )
    .unwrap();
}
