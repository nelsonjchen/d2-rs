// SPDX-License-Identifier: MPL-2.0
//! Input-only original-test replay; compiled only in unit-test executables.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{framing::ProjectedExtent, mesh::*, source_panels::NativePanel};
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::vec::Vec;

fn bytes(v: &Value) -> Vec<u8> {
    STANDARD.decode(v.as_str().unwrap()).unwrap()
}
fn colors(v: &Value) -> Vec<[u8; 4]> {
    serde_json::from_value(v.clone()).unwrap()
}
fn context(v: &Value) -> RenderContext {
    let token = CancellationToken::default();
    if v["canceled"].as_bool().unwrap() {
        token.cancel();
    }
    RenderContext::from_token(&token)
}
fn image(rect: [usize; 4], pixels: &[u8], colors: &Value) -> Value {
    json!({"rect":rect,"stride":rect[2]-rect[0],"pixels":STANDARD.encode(pixels),"colors":colors})
}
fn extent(v: &Value) -> ProjectedExtent {
    ProjectedExtent {
        min_x: v["minX"].as_f64().unwrap(),
        min_y: v["minY"].as_f64().unwrap(),
        min_z: v["minZ"].as_f64().unwrap(),
        max_x: v["maxX"].as_f64().unwrap(),
        max_y: v["maxY"].as_f64().unwrap(),
        max_z: v["maxZ"].as_f64().unwrap(),
        valid: v["valid"].as_bool().unwrap(),
    }
}
fn extent_value(e: ProjectedExtent) -> Value {
    json!({"minX":e.min_x,"minY":e.min_y,"minZ":e.min_z,"maxX":e.max_x,"maxY":e.max_y,"maxZ":e.max_z,"valid":e.valid})
}
fn replay(v: &Value) -> Value {
    match v["op"].as_str().unwrap() {
        "palette" => match palette_lookup(&context(v), &colors(&v["colors"])) {
            Ok(p) => json!({"error":"","lookup":STANDARD.encode(p)}),
            Err(e) => json!({"error":e.to_string(),"lookup":Value::Null}),
        },
        "index" => {
            let (w, h) = (
                v["width"].as_u64().unwrap() as usize,
                v["height"].as_u64().unwrap() as usize,
            );
            // The three original inputs are opaque RGBA, so their stored
            // channels are also the straight channels index_frame consumes.
            let raw = NrgbaImage {
                rect: d2_rs_raster::Rect::new(0, 0, w as i64, h as i64),
                pix: bytes(&v["pixels"]),
                stride: v["stride"].as_u64().unwrap() as usize,
            };
            assert!(raw.pix.chunks_exact(4).all(|p| p[3] == 255));
            match index_frame(&context(v), &raw, &bytes(&v["lookup"])) {
                Ok(p) => json!({"error":"","image":image([0,0,w,h],&p,&v["colors"])}),
                Err(e) => json!({"error":e.to_string(),"image":Value::Null}),
            }
        }
        "delta" => {
            let a = &v["after"];
            let r: [usize; 4] = serde_json::from_value(a["rect"].clone()).unwrap();
            let (x, y, w, h, p) = frame_delta(
                &bytes(&v["before"]["pixels"]),
                &bytes(&a["pixels"]),
                r[2],
                r[3],
            );
            image([x, y, x + w, y + h], &p, &a["colors"])
        }
        "delay" => json!(cycle_frame_delay(
            v["index"].as_u64().unwrap() as usize,
            FRAME_COUNT,
            833
        )),
        "write" => {
            let context = context(v);
            let mut w = BoundedWriter {
                context: &context,
                bytes: bytes(&v["destination"]),
                remaining: v["remaining"].as_u64().unwrap() as usize,
            };
            let (count, error) = match w.write(&bytes(&v["bytes"])) {
                Ok(n) => (n, String::new()),
                Err(e) => (0, e.to_string()),
            };
            json!({"count":count,"error":error,"remaining":w.remaining,"destination":STANDARD.encode(w.bytes)})
        }
        "loop" => {
            let document: d2_rs_scene::Document =
                serde_json::from_value(v["document"].clone()).unwrap();
            let original = NativeScene {
                panels: vec![NativePanel {
                    document,
                    animated: v["animated"].as_bool().unwrap(),
                    ..Default::default()
                }],
                ..Default::default()
            };
            let out = super::super::gif_loop::loop_scene(
                &context(v),
                &original,
                v["loop"].as_i64().unwrap(),
            )
            .unwrap();
            let a = &original.panels[0].document;
            let b = &out.panels[0].document;
            json!({"error":"","source":a,"document":b,"distinct_document":!std::ptr::eq(a,b),"distinct_root":!std::ptr::eq(a.root.as_deref().unwrap(),b.root.as_deref().unwrap()),"distinct_mask":!std::ptr::eq(a.root.as_ref().unwrap().mask.as_ref().unwrap().root.as_deref().unwrap(),b.root.as_ref().unwrap().mask.as_ref().unwrap().root.as_deref().unwrap())})
        }
        "ground" => {
            let mut e = extent(&v["extent"]);
            let ts = v["triangles"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| {
                    let mut tri = Triangle {
                        material: Some(material(Material {
                            color: serde_json::from_value(t["color"].clone()).unwrap(),
                            ..Default::default()
                        })),
                        cast_shadow: t["cast"].as_bool().unwrap(),
                        shadow_opacity: t["opacity"].as_f64(),
                        ..Default::default()
                    };
                    for (dst, src) in tri.v.iter_mut().zip(t["vertices"].as_array().unwrap()) {
                        dst.position = nv(
                            src["x"].as_f64().unwrap(),
                            src["y"].as_f64().unwrap(),
                            src["z"].as_f64().unwrap(),
                        );
                    }
                    tri
                })
                .collect::<Vec<_>>();
            e.ground_shadows(&ts, &[], v["scale"].as_f64().unwrap(), None);
            extent_value(e)
        }
        _ => panic!("unknown original operation"),
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let Ok(input) = std::env::var("D2RS_GEOMETRY_REQUEST") else {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/output-api.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            for op in case["operations"].as_array().unwrap() {
                same(&replay(&op["request"]), &op["got"]);
            }
        }
        return;
    };
    let output = std::env::var("D2RS_GEOMETRY_OUTPUT").unwrap();
    assert!(!std::path::Path::new(&output).exists());
    let request: Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    let values = request["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| replay(&o["request"]))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    std::fs::write(output, serde_json::to_vec(&values).unwrap()).unwrap();
}

fn same(a: &Value, b: &Value) {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => assert_eq!(a.as_f64(), b.as_f64()),
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                same(a, b)
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len());
            for (key, a) in a {
                same(a, b.get(key).unwrap())
            }
        }
        _ => assert_eq!(a, b),
    }
}
