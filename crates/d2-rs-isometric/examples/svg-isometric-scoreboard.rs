// SPDX-License-Identifier: MPL-2.0
//! Replay original d2svg document calls; no source compilation or layout.
use base64::{Engine, engine::general_purpose::STANDARD};
use d2_rs_isometric::{img::render::Options, svg};
use d2_rs_render::{SvgRenderOptions, target::TargetDiagram};
use serde_json::{Value, json};

fn evaluate(request: &Value, baseline: bool) -> Value {
    let o = &request["opts"];
    let theme =
        |key: &str| (!o[key].is_null()).then(|| serde_json::from_value(o[key].clone()).unwrap());
    let options = SvgRenderOptions {
        pad: o["Pad"].as_i64(),
        sketch: o["Sketch"].as_bool(),
        isometric: o["Isometric"].as_bool(),
        center: o["Center"].as_bool(),
        theme_id: o["ThemeID"].as_i64(),
        dark_theme_id: o["DarkThemeID"].as_i64(),
        theme_overrides: theme("ThemeOverrides"),
        dark_theme_overrides: theme("DarkThemeOverrides"),
        scale: request["scaleBits"].as_u64().map(f64::from_bits),
        master_id: o["MasterID"].as_str().unwrap_or_default().into(),
        no_xml_tag: o["NoXMLTag"].as_bool(),
        salt: o["Salt"].as_str().map(str::to_owned),
        omit_version: o["OmitVersion"].as_bool(),
        ..Default::default()
    };
    let target =
        TargetDiagram::decode_json(&serde_json::to_vec(&request["diagram"]).unwrap()).unwrap();
    // The negative baseline measures the pre-existing flat target primitive.
    // It lacks isometric dispatch; it is not claimed as a prior RenderIsometric API.
    if baseline {
        return match target.as_ref().unwrap().render_flat_svg(&options) {
            Ok(output) => json!({"output":STANDARD.encode(output),"error":""}),
            Err(error) => json!({"output":null,"error":error}),
        };
    }
    if request["api"] == "RenderMultiboard" {
        return match svg::render_multiboard(target.as_ref(), &options) {
            Ok(boards) => {
                json!({"output":boards.iter().map(|b| STANDARD.encode(b)).collect::<Vec<_>>(),"error":""})
            }
            Err(error) => json!({"output":null,"error":error}),
        };
    }
    let native = (!request["native"].is_null()).then(|| Options {
        width: request["native"]["Width"].as_i64().unwrap_or(0),
        height: request["native"]["Height"].as_i64().unwrap_or(0),
        fit_content: request["native"]["FitContent"].as_bool().unwrap_or(false),
        ..Default::default()
    });
    let result = if request["api"] == "RenderIsometric" {
        svg::render_isometric(None, target.as_ref(), &options, native.as_ref())
    } else {
        svg::render(target.as_ref(), &options)
    };
    match result {
        Ok(output) => json!({"output":STANDARD.encode(output),"error":""}),
        Err(error) => json!({"output":null,"error":error}),
    }
}
fn main() {
    let path = std::env::args().nth(1).expect("request.json");
    let request: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    println!(
        "{}",
        evaluate(&request, std::env::args().any(|v| v == "--baseline"))
    );
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_svg_isometric_calls() {
        let cases: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-isometric-api.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 13);
        for case in cases {
            assert_eq!(
                evaluate(&case["request"], false),
                case["got"],
                "{}",
                case["name"]
            );
        }
    }
}
