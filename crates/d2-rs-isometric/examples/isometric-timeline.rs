// SPDX-License-Identifier: MPL-2.0
//! RenderTimeline replay for the original pinned board-order and framing tests.
use base64::Engine;
use d2_rs_isometric::{img::render::Options, target::Diagram};
use serde_json::{Value, json};
use std::io::BufRead;

fn replay(r: &Value) -> Value {
    let diagram: Option<Vec<Option<Diagram>>> =
        serde_json::from_value(r["boards"].clone()).unwrap();
    let before = serde_json::to_value(&diagram).unwrap();
    let o = &r["options"];
    let options = Options {
        width: o["Width"].as_i64().unwrap_or(0),
        height: o["Height"].as_i64().unwrap_or(0),
        format: o["Format"].as_str().unwrap_or("").into(),
        fit_content: o["FitContent"].as_bool().unwrap_or(false),
        timeout_ns: o["Timeout"].as_i64().unwrap_or(0),
        ..Default::default()
    };
    let token = d2_rs_core::CancellationToken::default();
    match r["context"].as_str().unwrap() {
        "context canceled" => token.cancel(),
        "context deadline exceeded" => token.expire(),
        "" => (),
        value => panic!("unsupported context {value}"),
    }
    let result = d2_rs_isometric::img::timeline::render_timeline(
        Some(&token),
        diagram.as_deref().unwrap_or_default(),
        Some(&options),
        r["interval_ns"].as_i64().unwrap(),
    );
    assert_eq!(
        serde_json::to_value(&diagram).unwrap(),
        before,
        "GIF mutated source"
    );
    match result {
        Ok(data) => {
            json!({"data":base64::engine::general_purpose::STANDARD.encode(data), "error":""})
        }
        Err(error) => json!({"data":null, "error":error.to_string()}),
    }
}
fn main() {
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        println!("{}", replay(&request));
    }
}
