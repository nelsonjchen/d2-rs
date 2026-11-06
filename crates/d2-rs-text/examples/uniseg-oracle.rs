// SPDX-License-Identifier: MPL-2.0
use std::io::{self, Read};
fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let requests: Vec<String> = serde_json::from_str(&input).unwrap();
    let rows: Vec<_> = requests
        .iter()
        .map(|text| {
            serde_json::json!({"input":text,
        "line_boundaries":d2_rs_text::uniseg::line_boundaries(text),
        "grapheme_boundaries":d2_rs_text::uniseg::grapheme_boundaries(text),
        "string_width":d2_rs_text::uniseg::string_width(text)})
        })
        .collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
