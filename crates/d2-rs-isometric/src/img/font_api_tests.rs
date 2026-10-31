// SPDX-License-Identifier: MPL-2.0
//! Original configured-font requests. This module and transport compile only in tests.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::*;
use base64::Engine;
use d2_rs_scenebuild::{FallbackFont, FallbackRequest, FallbackResolver, FontFallbackOptions};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
fn asset(v: &Value) -> Vec<u8> {
    let v = &v["FontAsset"];
    if let Some(path) = v["DataFile"].as_str() {
        std::fs::read(path).unwrap()
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(v["Data"].as_str().unwrap())
            .unwrap()
    }
}
fn register(v: &Value) {
    use d2_rs_text::d2fonts::*;
    for (name, faces) in v.as_object().unwrap() {
        if FONT_FAMILIES.iter().any(|f| f.as_str() == name) {
            continue;
        }
        let data: Vec<_> = ["regular", "italic", "bold", "semibold"]
            .iter()
            .map(|s| asset(&faces[*s]))
            .collect();
        add_font_family(
            name,
            Some(&data[0]),
            Some(&data[1]),
            Some(&data[2]),
            Some(&data[3]),
        )
        .unwrap();
    }
}
#[derive(Debug)]
struct Resolver {
    font: FallbackFont,
    requested: AtomicBool,
    requests: Mutex<Vec<Value>>,
}
impl FallbackResolver for Resolver {
    fn resolve_fallbacks(&self, r: &FallbackRequest) -> Result<Vec<FallbackFont>, String> {
        self.requested.store(true, Ordering::SeqCst);
        self.requests.lock().unwrap().push(json!({"Runes":r.runes.iter().map(|c| *c as u32).collect::<Vec<_>>(),"Family":r.family,"Style":r.style,"Weight":r.weight}));
        Ok(vec![self.font.clone()])
    }
}
fn replay(case: &Value) -> Value {
    let mut texts = HashMap::<u64, TextPainter>::new();
    let mut outputs = vec![];
    for op in case["operations"].as_array().unwrap() {
        let v = &op["request"];
        register(&v["registry"]);
        let style = serde_json::from_value(v["style"].clone()).unwrap();
        let got = if v["op"] == "text" {
            let p = texts
                .entry(v["id"].as_u64().unwrap())
                .or_insert_with(|| TextPainter::new(v["count"].as_i64().unwrap(), None).unwrap());
            p.primary = v["primary"].as_str().unwrap().into();
            p.mono = v["mono"].as_str().unwrap().into();
            let fallback = &v["fallback"];
            let resolver = (!fallback.is_null()).then(|| {
                Arc::new(Resolver {
                    font: FallbackFont {
                        name: fallback["font"]["Name"].as_str().unwrap().into(),
                        mime_type: fallback["font"]["MIMEType"].as_str().unwrap().into(),
                        data: base64::engine::general_purpose::STANDARD
                            .decode(fallback["font"]["Data"].as_str().unwrap())
                            .unwrap()
                            .into(),
                        face_index: fallback["font"]["FaceIndex"].as_u64().unwrap() as u16,
                    },
                    requested: AtomicBool::new(fallback["requested"].as_bool().unwrap()),
                    requests: Mutex::new(vec![]),
                })
            });
            p.fallback_fonts = resolver.as_ref().map(|r| FontFallbackOptions {
                resolver_impl: Some(r.clone()),
                max_assets: fallback["options"]["MaxAssets"].as_i64().unwrap(),
                max_bytes: fallback["options"]["MaxBytes"].as_i64().unwrap(),
                ..Default::default()
            });
            let (frame, layout, error) = match p.texture(v["value"].as_str().unwrap(), &style) {
                Ok((t, l)) => (t, json!(l), String::new()),
                Err(e) => (
                    None,
                    json!({"Lines":null,"FontSize":0,"LineHeight":0,"Truncated":false}),
                    e,
                ),
            };
            let mut faces: Vec<_> = p
                .faces
                .keys()
                .map(|(f, b, i)| {
                    format!(
                        "{f}/{}",
                        if *b {
                            "bold"
                        } else if *i {
                            "italic"
                        } else {
                            "regular"
                        }
                    )
                })
                .collect();
            faces.sort();
            json!({"error":error,"layout":layout,"frame":frame.map(|t|json!({"width":t.width,"height":t.height,"stride":t.width*4,"pixels":base64::engine::general_purpose::STANDARD.encode(&*t.pixels.borrow())})),"faces":faces,"requests":resolver.as_ref().map(|r|r.requests.lock().unwrap().clone()).unwrap_or_default(),"requested":resolver.is_some_and(|r|r.requested.load(Ordering::SeqCst))})
        } else {
            assert_eq!(v["op"], "rich");
            let shape = serde_json::from_value(v["shape"].clone()).unwrap();
            match super::super::rich_labels::rich_label_document(
                &shape,
                &style,
                None,
                None,
                None,
                v["families"][0].as_str().unwrap(),
                v["families"][1].as_str().unwrap(),
            ) {
                Ok(d) => json!({"error":"","document":d}),
                Err(e) => json!({"error":e,"document":null}),
            }
        };
        outputs.push(got);
    }
    json!(outputs)
}
fn hydrate(v: &mut Value, assets: &Value) {
    match v {
        Value::Object(o) => {
            if let Some(key) = o.remove("DataRef") {
                o.insert("Data".into(), assets[key.as_str().unwrap()].clone());
            }
            for v in o.values_mut() {
                hydrate(v, assets);
            }
        }
        Value::Array(a) => {
            for v in a {
                hydrate(v, assets);
            }
        }
        _ => {}
    }
}
fn compact(v: &mut Value) {
    use sha2::{Digest, Sha256};
    match v {
        Value::Object(o) => {
            for (k, v) in o.iter_mut() {
                if matches!(k.as_str(), "pixels" | "Data") && v.is_string() {
                    let raw = base64::engine::general_purpose::STANDARD
                        .decode(v.as_str().unwrap())
                        .unwrap();
                    *v = json!({"length":raw.len(),"sha256":format!("{:x}",Sha256::digest(raw))});
                } else {
                    compact(v);
                }
            }
        }
        Value::Array(a) => {
            for v in a {
                compact(v);
            }
        }
        _ => {}
    }
}
fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a == b || a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|b| equal(v, b)))
        }
        _ => a == b,
    }
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let Ok(input) = std::env::var("D2RS_GEOMETRY_REQUEST") else {
        let mut fixture: Value =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/font-api.json")).unwrap();
        let assets = fixture["assets"].take();
        hydrate(&mut fixture["cases"], &assets);
        for case in fixture["cases"].as_array().unwrap() {
            let mut got = replay(case);
            compact(&mut got);
            let expected: Vec<_> = case["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o["got"].clone())
                .collect();
            assert!(equal(&got, &json!(expected)), "{}", case["name"]);
        }
        return;
    };
    let output = std::env::var("D2RS_GEOMETRY_OUTPUT").unwrap();
    assert!(!std::path::Path::new(&output).exists());
    let request: Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    let actual: Vec<_> = request["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(replay)
        .collect();
    std::fs::write(output, serde_json::to_vec(&actual).unwrap()).unwrap();
}
