// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::ast::*;
use d2_rs_language::format::{FormatOptions, format_key_path_with_options, key_path_with_options};
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
pub struct Request {
    parts: Vec<String>,
    specials: Option<String>,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn evaluate(request: Request) -> serde_json::Value {
    let values: Vec<_> = request
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let bytes: Vec<_> = part
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
                .collect();
            let value = match String::from_utf8(bytes) {
                Err(error) => Child::Block(Shared::new(BlockString {
                    value: error.into_bytes().into(),
                    ..Default::default()
                })),
                Ok(value) => match index % 3 {
                    0 => Child::Unquoted(Shared::new(UnquotedString::flat(value))),
                    1 => Child::DoubleQuoted(Shared::new(DoubleQuotedString::flat(value))),
                    _ => Child::SingleQuoted(Shared::new(SingleQuotedString {
                        value: value.into(),
                        ..Default::default()
                    })),
                },
            };
            ValueBox::from_child(Some(value)).string_box()
        })
        .collect();
    let path = KeyPath {
        path: Some(
            values
                .iter()
                .cloned()
                .map(|v| Some(Shared::new(v)))
                .collect::<Vec<_>>()
                .into(),
        ),
        ..Default::default()
    };
    let options = FormatOptions {
        unquoted_key_specials: request.specials.as_deref().unwrap_or(UNQUOTED_KEY_SPECIALS),
        ..Default::default()
    };
    json!({"hex":hex(&format_key_path_with_options(&values,options)),"parts":key_path_with_options(&path,options).map(|parts|parts.iter().map(|p|hex(p)).collect::<Vec<_>>())})
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    println!(
        "{}",
        serde_json::to_string(&requests.into_iter().map(evaluate).collect::<Vec<_>>()).unwrap()
    );
}
