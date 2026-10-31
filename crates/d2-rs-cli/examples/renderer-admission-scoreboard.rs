// SPDX-License-Identifier: MPL-2.0
//! Native public API replay for original master raw-target admission calls.
use base64::{Engine, engine::general_purpose::STANDARD};
use d2_rs_render::{SvgRenderOptions, target::TargetDiagram};
use serde_json::{Value, json};

fn evaluate(request: &Value) -> Value {
    let operation = request["operation"].as_str().unwrap();
    let result = if operation == "latex_validate" {
        let bytes = STANDARD.decode(request["bytes"].as_str().unwrap()).unwrap();
        d2_rs_math::validate_input(std::str::from_utf8(&bytes).expect("Rust string boundary"))
            .map(|_| Value::Null)
            .map_err(|e| e.to_string())
    } else {
        let graph = request["graph"].as_array().unwrap();
        let mut nodes: Vec<Option<Value>> = (0..graph.len()).map(|_| None).collect();
        for i in (0..graph.len()).rev() {
            let mut value = graph[i]["diagram"].clone();
            for name in ["layers", "scenarios", "steps"] {
                value[name] = match graph[i]["edges"][name].as_array() {
                    None => Value::Null,
                    Some(children) => Value::Array(
                        children
                            .iter()
                            .map(|id| {
                                id.as_u64()
                                    .map_or(Value::Null, |id| nodes[id as usize].take().unwrap())
                            })
                            .collect(),
                    ),
                };
            }
            assert!(graph[i]["floats"].is_null());
            nodes[i] = Some(value);
        }
        let d = TargetDiagram::decode_json(&serde_json::to_vec(&nodes[0].take().unwrap()).unwrap())
            .unwrap()
            .unwrap();
        let mut opts = SvgRenderOptions::default();
        opts.isometric = request["opts"]["Isometric"].as_bool();
        opts.theme_overrides = request["opts"]["ThemeOverrides"]
            .as_object()
            .map(|_| serde_json::from_value(request["opts"]["ThemeOverrides"].clone()).unwrap());
        match operation {
            "svg" => d2_rs_isometric::svg::render(Some(&d), &opts)
                .map(|bytes| json!(STANDARD.encode(bytes))),
            "isometric" => d2_rs_isometric::svg::render_isometric(None, Some(&d), &opts, None)
                .map(|bytes| json!(STANDARD.encode(bytes))),
            "multiboard" => {
                d2_rs_isometric::svg::render_multiboard(Some(&d), &opts).map(|frames| {
                    json!(
                        frames
                            .iter()
                            .map(|bytes| STANDARD.encode(bytes))
                            .collect::<Vec<_>>()
                    )
                })
            }
            _ => panic!("unexpected public operation {operation}"),
        }
    };
    match result {
        Ok(output) => json!({"output":output,"error":""}),
        Err(error) => json!({"output":null,"error":error}),
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
