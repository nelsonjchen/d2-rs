// SPDX-License-Identifier: MPL-2.0
//! Complete parser output for master's inline tests.
use d2_rs_language::{ParseContextError, ParseOptions, parse_with_context};
use serde_json::{Value, json};

fn evaluate(request: Value) -> Value {
    let context: Option<d2_rs_language::parser::ParseContext> =
        match request["context_policy"].as_str() {
            Some("background") => None,
            Some("canceled") => Some(Box::new(|| Some(d2_rs_core::LayoutError::Cancelled))),
            _ => return json!({"unsupported":"parser context"}),
        };
    let parsed = parse_with_context(
        request["path"].as_str().unwrap(),
        request["input"].as_str().unwrap().as_bytes(),
        Some(ParseOptions {
            utf16_positions: request["utf16"] == true,
            errors: None,
        }),
        context,
    );
    let (kind, structured) = match &parsed.error {
        Some(ParseContextError::Parse(error)) => ("parse", json!(error)),
        Some(ParseContextError::Context(d2_rs_core::LayoutError::Cancelled)) => {
            ("canceled", Value::Null)
        }
        Some(ParseContextError::Context(_)) => return json!({"unsupported":"parser context cause"}),
        None => ("", Value::Null),
    };
    json!({"map":parsed.map,
        "error":parsed.error.as_ref().map(ToString::to_string).unwrap_or_default(),
        "error_kind":kind,
        "structured":structured})
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).ok_or("request path")?,
    )?)?;
    println!("{}", evaluate(request));
    Ok(())
}
