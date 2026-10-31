// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::ast::*;
use d2_rs_language::{ParseOptions, format::format, parse};
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
pub struct Request {
    hex: String,
    mode: String,
    utf16: bool,
    path: String,
    #[serde(default)]
    in_key: bool,
    #[serde(default)]
    options: String,
    #[serde(default)]
    specials: Option<String>,
}
pub fn evaluate(r: Request) -> serde_json::Value {
    let bytes: Vec<_> = r
        .hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
        .collect();
    if r.mode != "format" {
        let value = match r.mode.as_str() {
            "format_single" => Child::SingleQuoted(Shared::new(SingleQuotedString {
                value: bytes.into(),
                ..Default::default()
            })),
            "format_double" => Child::DoubleQuoted(Shared::new(DoubleQuotedString::flat(
                String::from_utf8(bytes).unwrap(),
            ))),
            "format_unquoted" => Child::Unquoted(Shared::new(UnquotedString::flat(
                String::from_utf8(bytes).unwrap(),
            ))),
            "format_block" => Child::Block(Shared::new(BlockString {
                value: bytes.into(),
                quote: r.options,
                tag: "md".into(),
                ..Default::default()
            })),
            _ => panic!("unknown mode"),
        };
        let node = if r.in_key {
            Child::KeyPath(Shared::new(KeyPath {
                path: Some(
                    vec![Some(Shared::new(
                        ValueBox::from_child(Some(value.clone())).string_box(),
                    ))]
                    .into(),
                ),
                ..Default::default()
            }))
        } else {
            value.clone()
        };
        let mut options = d2_rs_language::format::FormatOptions::default();
        if let Some(specials) = r.specials.as_deref() {
            if r.in_key {
                options.unquoted_key_specials = specials;
            } else {
                options.unquoted_value_specials = specials;
            }
        }
        let bytes = d2_rs_language::format::format_with_options(&node, options);
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        return json!({"hex":hex,"value":ValueBox::from_child(Some(value))});
    }
    let output = parse(
        r.path,
        &bytes,
        Some(ParseOptions {
            utf16_positions: r.utf16,
            errors: None,
        }),
    );
    let map = Shared::new(output.map);
    let bytes = format(&Child::Map(map.clone()));
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    json!({"hex":hex,"map":map,"error":output.error.map_or_else(String::new,|e|e.to_string())})
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    let rows: Vec<_> = requests.into_iter().map(evaluate).collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
