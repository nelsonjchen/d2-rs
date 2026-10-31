// SPDX-License-Identifier: MPL-2.0
//! Whole-image calls from four original upstream paint/icon tests and a
//! container-gradient admission witness on regression/cylinder_grid_label.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use d2_rs_isometric::{
    img::{
        page::render_page,
        render::{Options, render_png_with_context, render_svg_with_context},
    },
    target::Diagram,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[cfg(feature = "oracle-fixtures")]
#[test]
fn whole_image_paint_and_container_admission_calls() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/paint-api.json")).unwrap();
    let calls = fixture["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 34);
    for (index, call) in calls.iter().enumerate() {
        let request = &call["request"];
        let diagram: Diagram = serde_json::from_value(request["diagram"].clone()).unwrap();
        let before = serde_json::to_value(&diagram).unwrap();
        let o = &request["options"];
        let options = Options {
            format: o["Format"].as_str().unwrap_or("").into(),
            width: o["Width"].as_i64().unwrap(),
            height: o["Height"].as_i64().unwrap(),
            fit_content: o["FitContent"].as_bool().unwrap(),
            ..Default::default()
        };
        let mut annotations = None;
        let result = if request["api"] == "page" {
            render_page(Some(&diagram), Some(&options))
                .map(|p| {
                    annotations = Some(json!({"Links":p.links,"Width":p.width,"Height":p.height}));
                    p.png
                })
                .map_err(|e| e.to_string())
        } else if options.format == "svg" {
            render_svg_with_context(None, Some(&diagram), Some(&options))
                .map(String::into_bytes)
                .map_err(|e| e.to_string())
        } else {
            render_png_with_context(None, Some(&diagram), Some(&options)).map_err(|e| e.to_string())
        };
        if call["want"]["error"] != "" {
            assert_eq!(result.unwrap_err(), call["want"]["error"], "call {index}");
        } else {
            let data = result.unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(&data)),
                call["want"]["sha256"],
                "call {index}"
            );
            assert_eq!(data.len() as u64, call["want"]["bytes"].as_u64().unwrap());
            if let Some(annotations) = annotations {
                assert_eq!(annotations, call["want"]["page"], "call {index}");
            }
        }
        assert_eq!(
            serde_json::to_value(&diagram).unwrap(),
            before,
            "source mutated at {index}"
        );
    }
}
