// SPDX-License-Identifier: MPL-2.0
//! Exact-time public capture replay; no native encoder or source normalization.
use base64::Engine;
use d2_rs_isometric::img::{
    gif_capture::{CallbackError, capture_frames},
    render::Options,
};
use serde_json::{Value, json};
use std::io::BufRead;

fn replay(r: &Value) -> Value {
    let diagram = r
        .get("diagram")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()).unwrap());
    let before = serde_json::to_value(&diagram).unwrap();
    let o = &r["options"];
    let options = Options {
        width: o["Width"].as_i64().unwrap_or(0),
        height: o["Height"].as_i64().unwrap_or(0),
        fit_content: o["FitContent"].as_bool().unwrap_or(false),
        format: o["Format"].as_str().unwrap_or("").into(),
        timeout_ns: o["Timeout"].as_i64().unwrap_or(0),
        ..Default::default()
    };
    let seconds: Vec<f64> = r["seconds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| match v.as_str().unwrap() {
            "+Inf" => f64::INFINITY,
            "-Inf" => f64::NEG_INFINITY,
            value => value.parse().unwrap(),
        })
        .collect();
    let token = d2_rs_core::CancellationToken::default();
    match r["context"].as_str().unwrap_or("") {
        "context canceled" => token.cancel(),
        "context deadline exceeded" => token.expire(),
        "" => (),
        value => panic!("unsupported caller cause {value}"),
    }
    let mut frames: Option<Vec<String>> = None;
    let mut callback = |i: usize, bytes: Vec<u8>| {
        frames
            .get_or_insert_default()
            .push(base64::engine::general_purpose::STANDARD.encode(bytes));
        if r["cancelAfter"].as_i64() == Some(i as i64) {
            token.cancel();
        }
        let error = r["callbackError"].as_str().unwrap_or("");
        if error.is_empty() {
            Ok(())
        } else {
            Err(std::io::Error::other(error).into())
        }
    };
    let emit = r["callback"]
        .as_bool()
        .unwrap()
        .then_some(&mut callback as &mut dyn FnMut(usize, Vec<u8>) -> Result<(), CallbackError>);
    let result = capture_frames(
        Some(&token),
        diagram.as_ref(),
        Some(&options),
        &seconds,
        r["traffic"].as_bool().unwrap(),
        emit,
    );
    assert_eq!(
        serde_json::to_value(&diagram).unwrap(),
        before,
        "capture mutated source"
    );
    json!({"frames":frames,"error":result.err().map(|e| e.to_string()).unwrap_or_default()})
}
fn main() {
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        println!("{}", replay(&request));
    }
}
