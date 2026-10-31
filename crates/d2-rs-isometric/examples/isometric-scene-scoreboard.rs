// SPDX-License-Identifier: MPL-2.0
//! Native scene construction and typed mutations from original source tests.
use d2_rs_isometric::{
    scene::{RenderOpts, Scene, build_scene},
    target::Diagram,
};
use serde_json::{Value, json};
use std::collections::HashMap;
fn replay(case: &Value) -> Value {
    let mut sessions = HashMap::<u64, (Diagram, Scene)>::new();
    let mut rows = Vec::new();
    for op in case["operations"].as_array().unwrap() {
        let id = op["id"].as_u64().unwrap();
        rows.push(match op["op"].as_str().unwrap() {
            "validate" => {
                if op["diagram"].is_null() {
                    return json!({"unsupported":"safe native BuildScene takes a non-null diagram reference"});
                }
                let mut d: Diagram = serde_json::from_value(op["diagram"].clone()).unwrap();
                match op["mutation"].as_str().unwrap() {
                    "duplicate node" => {
                        let node = d.shapes()[0].clone();
                        d.shapes.as_mut().unwrap().push(node);
                    }
                    "nonfinite route" => {
                        d.connections.as_mut().unwrap()[0].route.as_mut().unwrap()[0]
                            .as_mut().unwrap().x = f64::INFINITY;
                    }
                    "nil route point" => {
                        d.connections.as_mut().unwrap()[0].route.as_mut().unwrap()[0] = None;
                    }
                    "nonfinite ratio" => d.shapes.as_mut().unwrap()[0].content_aspect_ratio = Some(f64::NAN),
                    "negative size" => d.shapes.as_mut().unwrap()[0].width = -1,
                    "missing endpoint" => {
                        let edge = &mut d.connections.as_mut().unwrap()[0];
                        edge.dst = "absent".into();
                        edge.route = None;
                    }
                    "duplicate edge" => {
                        let edge = d.connections()[0].clone();
                        d.connections.as_mut().unwrap().push(edge);
                    }
                    "text budget" => d.shapes.as_mut().unwrap()[0].text.label = "x".repeat((8 << 20) + 1),
                    "node budget" => d.shapes = Some(vec![Default::default(); 2001]),
                    "invalid UTF8" => return json!({"unsupported":"safe native String cannot represent the original invalid UTF-8 mutation"}),
                    "ID size" => d.shapes.as_mut().unwrap()[0].id = "x".repeat(8193),
                    "ID depth" => d.shapes.as_mut().unwrap()[0].id = "x.".repeat(128) + "x",
                    other => panic!("unknown validation mutation {other}"),
                }
                match build_scene(&d, None) {
                    Ok(_) => json!({"scene_nil":false,"error":""}),
                    Err(error) => json!({"scene_nil":true,"error":error}),
                }
            }
            "build" => {
                let mut d: Diagram = serde_json::from_value(op["diagram"].clone()).unwrap();
                // Restore private Userinfo from the separately captured source URL.
                // The expected scene never supplies native input data.
                for (key, authored) in op["icon_urls"].as_object().unwrap() {
                    let (kind, index) = key.split_once(':').unwrap();
                    let index: usize = index.parse().unwrap();
                    let icon = match kind {
                        "shape" => d.shapes.as_mut().unwrap()[index].icon.as_mut().unwrap(),
                        "edge" => d.connections.as_mut().unwrap()[index].icon.as_mut().unwrap(),
                        _ => panic!("unknown icon source"),
                    };
                    let authored = authored.as_str().unwrap();
                    let prefix = if icon.scheme.is_empty() { "//".to_owned() } else { format!("{}://", icon.scheme) };
                    if let Some(rest) = authored.strip_prefix(&prefix) {
                        let authority = rest.split(['/', '?', '#']).next().unwrap();
                        if let Some((encoded, _)) = authority.rsplit_once('@') {
                            icon.user = d2_rs_isometric::target::Userinfo::from_encoded(encoded);
                        }
                    }
                    if icon.string() != authored {
                        return json!({"unsupported":"native target URL cannot retain authored Userinfo from the Go JSON boundary"});
                    }
                }
                let o = (!op["options"].is_null()).then(|| RenderOpts {
                    theme_id: op["options"]["ThemeID"].as_i64(),
                    theme_overrides: op["options"]
                        .get("ThemeOverrides")
                        .filter(|v| !v.is_null())
                        .map(|v| serde_json::from_value(v.clone()).unwrap()),
                });
                match build_scene(&d, o.as_ref()) {
                    Ok(scene) => {
                        let got = json!({"scene":scene,"source":d,"error":""});
                        sessions.insert(id, (d, scene));
                        got
                    }
                    Err(e) => json!({"scene":null,"source":d,"error":e}),
                }
            }
            "mutate" => {
                let (d, s) = sessions.get_mut(&id).unwrap();
                match op["mutation"].as_str().unwrap() {
                    "column" => {
                        s.nodes[0].metadata.original.columns.as_mut().unwrap()[0]
                            .constraint
                            .as_mut()
                            .unwrap()[0] = "changed".into()
                    }
                    "field" => {
                        s.nodes[1].metadata.original.fields.as_mut().unwrap()[0].name =
                            "changed".into()
                    }
                    "route444" => {
                        s.edges[0].metadata.original.route.as_mut().unwrap()[0]
                            .as_mut()
                            .unwrap()
                            .x = 444.
                    }
                    "route998" => {
                        s.edges[0].metadata.original.route.as_mut().unwrap()[0]
                            .as_mut()
                            .unwrap()
                            .x = 998.
                    }
                    "source-label" => {
                        s.edges[0].source_label.as_mut().unwrap().label = "changed".into()
                    }
                    "point" => s.edges[0].points.as_mut().unwrap()[0].x = 999.,
                    other => panic!("unknown mutation {other}"),
                };
                json!({"source":d,"error":""})
            }
            other => panic!("unknown op {other}"),
        });
    }
    json!(rows)
}
fn main() {
    let request: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let output: Vec<_> = request["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(replay)
        .collect();
    println!("{}", json!(output));
}
