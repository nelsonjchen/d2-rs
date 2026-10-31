// SPDX-License-Identifier: MPL-2.0
//! Replay original pinned renderer calls without compilation or layout.
use base64::{Engine, engine::general_purpose::STANDARD};
use d2_rs_render::{SvgRenderOptions, target::TargetDiagram};
use serde_json::{Value, json};

fn evaluate(request: &Value) -> Value {
    let operation = request["operation"].as_str().unwrap();
    let result: Result<Value, String> = match operation {
        "wrap" => {
            let target =
                TargetDiagram::decode_json(&serde_json::to_vec(&request["diagram"]).unwrap())
                    .unwrap()
                    .unwrap();
            let frames: Vec<Vec<u8>> = request["frames"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| STANDARD.decode(v.as_str().unwrap()).unwrap())
                .collect();
            let mut options = SvgRenderOptions::default();
            options.pad = request["opts"]["Pad"].as_i64();
            target
                .wrap_svg_frames(&frames, &options, request["interval"].as_i64().unwrap())
                .map(|bytes| json!(STANDARD.encode(bytes)))
        }
        "aspect" => d2_rs_scene::aspect_ratio_matrix(
            serde_json::from_value(request["source"].clone()).unwrap(),
            serde_json::from_value(request["destination"].clone()).unwrap(),
            serde_json::from_value(request["aspect"].clone()).unwrap(),
        )
        .map(|matrix| json!(matrix)),
        "validate" => d2_rs_math::validate_input(request["input"].as_str().unwrap())
            .map(|()| Value::Null)
            .map_err(|error| error.to_string()),
        "render" => d2_rs_math::render_exact(request["input"].as_str().unwrap())
            .map(|svg| json!(svg))
            .map_err(|error| error.to_string()),
        "measure" => d2_rs_math::measure(request["input"].as_str().unwrap())
            .map(|size| json!([size.width as i64, size.height as i64]))
            .map_err(|error| error.to_string()),
        _ => panic!("unsupported operation {operation}"),
    };
    match result {
        Ok(output) => json!({"output": output, "error": ""}),
        Err(error) => {
            // Match each Go signature's failure return values as well as its error.
            let output = match operation {
                "render" => json!(""),
                "measure" => json!([0, 0]),
                "aspect" => json!(d2_rs_scene::Matrix::default()),
                _ => Value::Null,
            };
            json!({"output": output, "error": error})
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("requests.json");
    let requests: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    println!(
        "{}",
        json!(requests.iter().map(evaluate).collect::<Vec<_>>())
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_pinned_renderer_calls() {
        let cases: Vec<Value> =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/renderer-contract-api.json"))
                .unwrap();
        assert_eq!(cases.len(), 15);
        for case in cases {
            let actual = evaluate(&case["request"]);
            if case["request"]["operation"] == "aspect" {
                assert_eq!(actual["error"], case["got"]["error"], "{}", case["name"]);
                // Go writes integral float64 fields as JSON integers; compare
                // the native Matrix contract, preserving every f64 value.
                let actual: d2_rs_scene::Matrix =
                    serde_json::from_value(actual["output"].clone()).unwrap();
                let expected: d2_rs_scene::Matrix =
                    serde_json::from_value(case["got"]["output"].clone()).unwrap();
                assert_eq!(actual, expected, "{}", case["name"]);
            } else {
                assert_eq!(actual, case["got"], "{}", case["name"]);
            }
        }
    }
}
