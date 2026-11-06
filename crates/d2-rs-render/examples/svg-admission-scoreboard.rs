// SPDX-License-Identifier: MPL-2.0
//! Replay public raw-target rendering calls from the pinned Go tests.
use base64::{Engine, engine::general_purpose::STANDARD};
use d2_rs_render::{RenderOptions, target::TargetDiagram};
use serde_json::{Value, json};

fn evaluate(v: &Value) -> Value {
    let opts = &v["opts"];
    let mut options = RenderOptions::default();
    options.pad = opts["Pad"].as_i64().unwrap_or(100) as f64;
    options.theme_id = opts["ThemeID"].as_i64();
    options.dark_theme_id = opts["DarkThemeID"].as_i64();
    for (key, map) in [
        ("ThemeOverrides", &mut options.theme_overrides),
        ("DarkThemeOverrides", &mut options.dark_theme_overrides),
    ] {
        for (name, value) in opts[key].as_object().into_iter().flatten() {
            if let Some(value) = value.as_str() {
                map.insert(name.to_lowercase(), value.into());
            }
        }
    }
    let target = TargetDiagram::decode_json(&serde_json::to_vec(&v["diagram"]).unwrap())
        .unwrap()
        .unwrap();
    match target.render_d2svg(
        &options,
        opts["Sketch"] == true,
        false,
        0,
        v["multi"] == true,
    ) {
        Ok(output) => json!({"output":STANDARD.encode(output),"error":""}),
        Err(error) => json!({"output":null,"error":error}),
    }
}
fn main() {
    let path = std::env::args().nth(1).expect("request.json");
    let request: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    println!("{}", evaluate(&request));
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_pinned_raw_target_calls() {
        check(
            d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-admission-api.json"),
            60,
        );
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_pinned_structural_calls() {
        check(
            d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-structural-api.json"),
            31,
        );
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_pinned_label_calls() {
        check(
            d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-labels-api.json"),
            10,
        );
    }

    fn check(fixture: &str, count: usize) {
        let cases: Vec<Value> = serde_json::from_str(fixture).unwrap();
        assert_eq!(cases.len(), count);
        for case in cases {
            let got = evaluate(&case["request"]);
            assert_eq!(got["error"], case["error"], "{}", case["name"]);
            let digest = got["output"]
                .as_str()
                .map(|s| format!("{:x}", Sha256::digest(STANDARD.decode(s).unwrap())));
            assert_eq!(json!(digest), case["output_sha256"], "{}", case["name"]);
        }
    }
}
