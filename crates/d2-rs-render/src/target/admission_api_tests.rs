// SPDX-License-Identifier: MPL-2.0
//! Original master render/target calls transported without changing input values.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

fn diagram(graph: &Value) -> TargetDiagram {
    let nodes = graph.as_array().unwrap();
    let mut built: Vec<Option<TargetDiagram>> = (0..nodes.len()).map(|_| None).collect();
    for i in (0..nodes.len()).rev() {
        let mut d: TargetDiagram = serde_json::from_value(nodes[i]["diagram"].clone()).unwrap();
        for patch in nodes[i]["floats"].as_array().into_iter().flatten() {
            let mut connections = d.connections.as_deref().unwrap().to_vec();
            let c = &mut connections[patch["connection"].as_u64().unwrap() as usize];
            let mut route = c.route.as_deref().unwrap().to_vec();
            let p = route[patch["point"].as_u64().unwrap() as usize]
                .as_mut()
                .unwrap();
            let bits = u64::from_str_radix(patch["bits"].as_str().unwrap(), 16).unwrap();
            if patch["axis"] == "x" {
                p.x.0 = f64::from_bits(bits);
            } else {
                p.y.0 = f64::from_bits(bits);
            }
            c.route = Some(route.into());
            d.connections = Some(connections.into());
        }
        for (name, children) in [
            ("layers", &mut d.layers),
            ("scenarios", &mut d.scenarios),
            ("steps", &mut d.steps),
        ] {
            *children = nodes[i]["edges"][name].as_array().map(|ids| {
                ids.iter()
                    .map(|id| {
                        id.as_u64()
                            .map(|id| built[id as usize].take().expect("owned tree child"))
                    })
                    .collect::<Vec<_>>()
                    .into()
            });
        }
        built[i] = Some(d);
    }
    built[0].take().unwrap()
}

fn evaluate(request: &Value) -> Value {
    let operation = request["operation"].as_str().unwrap();
    let result: Result<Value, String> = match operation {
        "target_validate" => diagram(&request["graph"])
            .validate_render_target()
            .map(|_| Value::Null),
        "target_hash" => diagram(&request["graph"]).hash_id(None).map(|s| json!(s)),
        "svg" => diagram(&request["graph"])
            .render_flat_svg(&crate::SvgRenderOptions::default())
            .map(|bytes| json!(STANDARD.encode(bytes))),
        "sketch" | "sketch_typed" => {
            let c: TargetConnection =
                serde_json::from_value(request["connection"].clone()).unwrap();
            let path = request["path"].as_str().unwrap();
            if operation == "sketch" {
                d2svg::sketch::connection(&c, path, request["attributes"].as_str().unwrap())
                    .map(|s| json!(s))
            } else {
                let attrs: Vec<_> = request["attributes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| {
                        if a.is_null() {
                            d2svg::markup::Attribute::default()
                        } else {
                            d2svg::markup::attr(
                                a["name"].as_str().unwrap(),
                                a["value"].as_str().unwrap(),
                            )
                        }
                    })
                    .collect();
                d2svg::sketch::connection_with_attributes(&c, path, &attrs).map(|s| json!(s))
            }
        }
        _ => panic!("unexpected core operation {operation}"),
    };
    match result {
        Ok(output) => json!({"output":output,"error":""}),
        Err(error) => {
            json!({"output": if matches!(operation,"sketch"|"sketch_typed"|"target_hash") { json!("") } else { Value::Null }, "error":error})
        }
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_calls() {
    // HashID's original 1024-deep graph needs a larger stack than Rust's
    // small test-worker default. Validation itself remains iterative.
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            if let Ok(path) = std::env::var("D2RS_RENDER_ADMISSION_INPUT") {
                let requests: Vec<Value> =
                    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                let output: Vec<_> = requests
                    .iter()
                    .map(|r| {
                        std::panic::catch_unwind(|| evaluate(r))
                            .unwrap_or_else(|_| json!({"panic":true}))
                    })
                    .collect();
                std::fs::write(
                    std::env::var("D2RS_RENDER_ADMISSION_OUTPUT").unwrap(),
                    serde_json::to_vec(&output).unwrap(),
                )
                .unwrap();
            } else {
                let cases: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
                    "tests/fixtures/renderer-admission-sketch.json"
                ))
                .unwrap();
                assert_eq!(cases.len(), 6);
                for case in cases {
                    assert_eq!(evaluate(&case["request"]), case["got"], "{}", case["name"]);
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
