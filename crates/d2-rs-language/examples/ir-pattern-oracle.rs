// SPDX-License-Identifier: MPL-2.0
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
pub struct Request {
    hex: String,
    parts: Vec<String>,
}
fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
        .collect()
}
pub fn evaluate(r: Request) -> Value {
    std::panic::catch_unwind(|| {
        let pattern: Vec<_> = r
            .parts
            .iter()
            .map(|h| d2_rs_core::ast::ByteString::from(bytes(h)))
            .collect();
        json!({"match":d2_rs_core::ir::match_pattern(&bytes(&r.hex),&pattern)})
    })
    .unwrap_or_else(|_| json!({"panic":true}))
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        json!(requests.into_iter().map(evaluate).collect::<Vec<_>>())
    );
}
