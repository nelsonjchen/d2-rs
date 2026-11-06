// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of the original panel and source-clearance calls.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{
    geometry_api_tests::node, native_solids::solid_api_tests as solid,
    surface_icons::SurfaceIconPainter, text::TextPainter,
};
use super::*;
use serde_json::{Value, json};
use std::collections::HashMap;

fn panel_state(b: &MeshBuilder) -> Value {
    assert!(b.links.is_empty());
    json!({"mesh":solid::mesh(&b.triangles),"error":b.err.as_deref().unwrap_or_default(),
        "pixels":b.face_pixels,"panels":b.panels.iter().map(|p|json!({"document":p.document,
            "width":p.width,"height":p.height,"first":p.first,"last":p.last,"animated":p.animated})).collect::<Vec<_>>()})
}
fn replay(
    v: &Value,
    panels: &mut HashMap<u64, MeshBuilder>,
    texts: &mut HashMap<u64, TextPainter>,
    icons: &mut HashMap<u64, SurfaceIconPainter>,
) -> Value {
    let op = v["op"].as_str().unwrap();
    if op == "panel" {
        let b = panels.entry(v["id"].as_u64().unwrap()).or_default();
        b.output_density = v["density"].as_f64().unwrap();
        let doc: Document = serde_json::from_value(v["document"].clone()).unwrap();
        let area = super::super::geometry::LabelSurface {
            center: nv(
                v["area"]["center"]["x"].as_f64().unwrap(),
                v["area"]["center"]["y"].as_f64().unwrap(),
                v["area"]["center"]["z"].as_f64().unwrap(),
            ),
            width: v["area"]["width"].as_f64().unwrap(),
            depth: v["area"]["depth"].as_f64().unwrap(),
            angle: v["area"]["angle"].as_f64().unwrap(),
            align: v["area"]["align"].as_str().unwrap().into(),
        };
        let error = b
            .source_panel(doc.clone(), area, v["planned"].as_u64().unwrap() as usize)
            .err()
            .unwrap_or_default();
        return json!({"error":error,"state":panel_state(b),"source":doc});
    }
    let mut b = solid::builder(&v["state"]);
    if !v["text"].is_null() {
        let id = v["text"]["id"].as_u64().unwrap();
        b.text = texts.remove(&id).unwrap_or_else(|| {
            TextPainter::new(v["text"]["count"].as_i64().unwrap(), None).unwrap()
        });
    }
    if !v["icons"].is_null() {
        let id = v["icons"]["id"].as_u64().unwrap();
        b.icons = icons.remove(&id).unwrap_or_else(|| {
            SurfaceIconPainter::new(v["icons"]["count"].as_i64().unwrap(), None, None).unwrap()
        });
    }
    let n = node(&v["node"]);
    let tint = v["tint"].as_str().unwrap();
    if op == "hierarchy" {
        b.hierarchy_node(&n, tint);
    } else {
        b.node(
            &n,
            tint,
            v["relief"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_f64),
        );
    }
    let output = json!({"state":solid::state(&b),"node":n});
    if !v["text"].is_null() {
        let p = std::mem::take(&mut b.text);
        texts.insert(v["text"]["id"].as_u64().unwrap(), p);
    }
    if !v["icons"].is_null() {
        let p = std::mem::take(&mut b.icons);
        icons.insert(v["icons"]["id"].as_u64().unwrap(), p);
    }
    output
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|_| {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/panel-api.json").into()
        });
    let cases: Value = serde_json::from_str(&source).unwrap();
    let output: Vec<_> = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let mut panels = HashMap::new();
            let mut texts = HashMap::new();
            let mut icons = HashMap::new();
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| replay(&o["request"], &mut panels, &mut texts, &mut icons))
                .collect::<Vec<_>>()
        })
        .collect();
    if let Ok(p) = std::env::var("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(p, serde_json::to_vec(&output).unwrap()).unwrap();
    } else {
        let expected: Vec<_> = cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                c["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| o["got"].clone())
                    .collect::<Vec<_>>()
            })
            .collect();
        super::super::geometry_api_tests::assert_same(&json!(expected), &json!(output));
    }
}
