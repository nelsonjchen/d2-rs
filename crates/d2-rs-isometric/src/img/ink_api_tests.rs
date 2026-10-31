// SPDX-License-Identifier: MPL-2.0
//! Admission replay only: expected values never enter production execution.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{context::RenderContext, geometry_api_tests};
use super::*;
use d2_rs_core::CancellationToken;
use serde_json::{Value, json};

fn vector(v: &Value) -> Vec {
    Vec {
        x: v["x"].as_f64().unwrap(),
        y: v["y"].as_f64().unwrap(),
        z: v["z"].as_f64().unwrap(),
    }
}
fn replay(request: &Value) -> Value {
    let token = CancellationToken::new();
    if request["canceled"].as_bool().unwrap() {
        token.cancel();
    }
    let context = RenderContext::from_token(&token);
    if request["op"] == "fit" {
        use super::super::mesh::{Texture, TextureFormat};
        use base64::Engine;
        let t = &request["texture"];
        let texture = if t.is_null() {
            None
        } else {
            let rect = &t["rect"];
            assert_eq!(rect["Min"], json!({"X": 0, "Y": 0}));
            Some(Rc::new(Texture::with_storage(
                rect["Max"]["X"].as_u64().unwrap() as usize,
                rect["Max"]["Y"].as_u64().unwrap() as usize,
                t["stride"].as_i64().unwrap(),
                TextureFormat::Rgba,
                base64::engine::general_purpose::STANDARD
                    .decode(t["pixels"].as_str().unwrap())
                    .unwrap(),
            )))
        };
        let mut builder = MeshBuilder::default();
        builder.context = context;
        builder.err = request["error"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let fit = builder.solid_ink_fit(texture.as_ref(), request["hex"].as_bool().unwrap());
        return json!({"fit": fit, "error": builder.err.as_deref().unwrap_or("")});
    }
    let node = geometry_api_tests::node(&request["node"]);
    let mut triangles: std::vec::Vec<Triangle> = request["triangles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let vertices = v["vertices"].as_array().unwrap();
            let m = &v["material"];
            let material = if m.is_null() {
                None
            } else {
                assert_eq!(m["texture"], false, "this admission fixture has no texture");
                Some(super::super::mesh::material(super::super::mesh::Material {
                    color: serde_json::from_value(m["color"].clone()).unwrap(),
                    ..Default::default()
                }))
            };
            Triangle {
                v: std::array::from_fn(|i| Vertex {
                    position: vector(&vertices[i]["Position"]),
                    normal: vector(&vertices[i]["Normal"]),
                    u: vertices[i]["U"].as_f64().unwrap(),
                    v: vertices[i]["V"].as_f64().unwrap(),
                }),
                material,
                cast_shadow: v["shadow"].as_bool().unwrap(),
                depth_bias: v["bias"].as_f64().unwrap(),
                no_depth_write: v["noDepth"].as_bool().unwrap(),
                ..Default::default()
            }
        })
        .collect();
    let zero_count = request["zeroCount"].as_u64().unwrap() as usize;
    if zero_count > 0 {
        assert!(triangles.is_empty());
        triangles.resize_with(zero_count, Triangle::default);
    }
    let radius = request["radius"]
        .as_array()
        .and_then(|v| v.first())
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    match classic_ink_segments_with_context(&context, &node, &triangles, radius) {
        Ok(segments) => json!({"count":segments.len(),"error":""}),
        Err(error) => json!({"count":0,"error":error}),
    }
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|_| d2_rs_testdata::oracle_fixture!("tests/fixtures/ink-api.json").into());
    let cases: Value = serde_json::from_str(&source).unwrap();
    let out: std::vec::Vec<_> = cases["cases"]
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
    if let Ok(path) = std::env::var("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(path, serde_json::to_vec(&out).unwrap()).unwrap();
    } else {
        let expected: std::vec::Vec<_> = cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                c["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| o["got"].clone())
                    .collect::<std::vec::Vec<_>>()
            })
            .collect();
        geometry_api_tests::assert_same(&json!(expected), &json!(out));
    }
}

#[test]
fn source_admission_order_and_builder_context() {
    let token = CancellationToken::new();
    token.cancel();
    let context = RenderContext::from_token(&token);
    let node = Node {
        stroke_width: 4,
        opacity: 1.0,
        stroke: "black".into(),
        fill: "white".into(),
        ..Default::default()
    };
    // Go polls its loops, rather than adding a new eager admission check.
    assert!(
        classic_ink_segments_with_context(&context, &node, &[], 0.0)
            .unwrap()
            .is_empty()
    );
    let oversized = vec![Triangle::default(); MAX_CLASSIC_INK_TRIANGLES + 1];
    assert_eq!(
        classic_ink_segments_with_context(&context, &node, &oversized, 0.0)
            .err()
            .as_deref(),
        Some("isometric outline exceeds 100000 input triangles")
    );
    assert_eq!(
        classic_ink_segments_with_context(&context, &node, &[Triangle::default()], 0.0)
            .err()
            .as_deref(),
        Some("context canceled")
    );
    let mut builder = MeshBuilder::default();
    builder.context = context;
    builder.triangles.push(Triangle::default());
    builder.classic_ink_edges(&node, 0);
    assert_eq!(builder.err.as_deref(), Some("context canceled"));
    assert_eq!(builder.triangles.len(), 1);
    builder.err = Some("prior failure".into());
    builder.classic_ink_edges(&node, 0);
    assert_eq!(builder.err.as_deref(), Some("prior failure"));
}

#[test]
fn solid_fit_checks_rows_and_retains_nil_admission() {
    use super::super::mesh::Texture;
    let token = CancellationToken::new();
    token.cancel();
    let mut builder = MeshBuilder::default();
    builder.context = RenderContext::from_token(&token);
    builder.err = Some("prior failure".into());
    assert_eq!(builder.solid_ink_fit(None, false), 1.0);
    assert_eq!(builder.err.as_deref(), Some("prior failure"));
    let empty = Rc::new(Texture::new(0, 0, vec![]));
    assert_eq!(builder.solid_ink_fit(Some(&empty), false), 1.0);
    assert_eq!(builder.err.as_deref(), Some("prior failure"));
    let texture = Rc::new(Texture::new(32, 32, vec![0; 32 * 32 * 4]));
    assert_eq!(builder.solid_ink_fit(Some(&texture), true), 1.0);
    // This Go helper assigns its context error directly, before pixel reads.
    assert_eq!(builder.err.as_deref(), Some("context canceled"));
}
