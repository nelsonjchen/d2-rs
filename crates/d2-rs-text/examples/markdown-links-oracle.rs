// SPDX-License-Identifier: MPL-2.0
use std::io::{self, Read};
fn decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let requests: Vec<serde_json::Value> = serde_json::from_str(&input).unwrap();
    let rows:Vec<_> = requests.iter().map(|request| {
        let bytes=decode(request["hex"].as_str().unwrap());
        let raw=d2_rs_text::markdown::html::render_html_bytes(&bytes);
        let html=d2_rs_text::markdown::html::render_markdown_html_bytes(&bytes);
        serde_json::json!({"raw_html_hex":hex(&raw),"html_hex":hex(&html),"error":null,"xml_error":d2_rs_text::xml::validate_markdown_html_bytes(&html).err()})
    }).collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
