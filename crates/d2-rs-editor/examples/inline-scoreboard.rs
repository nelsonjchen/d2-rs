// SPDX-License-Identifier: MPL-2.0
//! Native runner for scripts/inline-scoreboard.py. `completion` requests print
//! Go's CompletionItem JSON shape; `escape` requests rebuild the AST string
//! node that d2format/escape_test.go formats and print the formatted text.

use d2_rs_core::ast;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: inline-scoreboard <request.json>")?;
    let request: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let output = match request["kind"].as_str() {
        Some(kind) => serde_json::Value::String(escape(kind, &request)?),
        None => completion(&request)?,
    };
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

fn completion(
    request: &serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let text = request["text"].as_str().ok_or("missing text")?;
    let line = request["line"].as_u64().ok_or("missing line")? as usize;
    let column = request["column"].as_u64().ok_or("missing column")? as usize;
    let encoding = if request["utf16"] == true {
        d2_rs_editor::PositionEncoding::Utf16
    } else {
        d2_rs_editor::PositionEncoding::Utf8
    };
    let items = d2_rs_editor::completion_items(text, line, column, encoding)
        .into_iter()
        .map(|item| {
            serde_json::json!({
                "Label": item.label,
                "Kind": item.kind as u8,
                "Detail": item.detail,
                "InsertText": item.insert_text,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::Value::Array(items))
}

/// The nodes built by escape_test.go: a SingleQuotedString, a flat double
/// quoted or unquoted string (optionally as a one-component KeyPath, via
/// MakeValueBox(...).StringBox()), or a BlockString.
fn escape(kind: &str, request: &serde_json::Value) -> Result<String, Box<dyn std::error::Error>> {
    let text = |field: &str| request[field].as_str().unwrap_or("").to_owned();
    let node = match kind {
        "single" => ast::Child::SingleQuoted(ast::Shared::new(ast::SingleQuotedString {
            value: text("str").into(),
            ..Default::default()
        })),
        "block" => ast::Child::Block(ast::Shared::new(ast::BlockString {
            quote: text("quote"),
            tag: text("tag"),
            value: text("value").into(),
            ..Default::default()
        })),
        "double" | "unquoted" => {
            let mut string_box = ast::StringBox::default();
            let child = if kind == "double" {
                let value = ast::Shared::new(ast::DoubleQuotedString::flat(text("str")));
                string_box.double_quoted_string = Some(value.clone());
                ast::Child::DoubleQuoted(value)
            } else {
                let value = ast::Shared::new(ast::UnquotedString::flat(text("str")));
                string_box.unquoted_string = Some(value.clone());
                ast::Child::Unquoted(value)
            };
            if request["in_key"] == true {
                ast::Child::KeyPath(ast::Shared::new(ast::KeyPath {
                    range: Default::default(),
                    path: Some(vec![Some(ast::Shared::new(string_box))].into()),
                }))
            } else {
                child
            }
        }
        _ => return Err(format!("unknown escape kind {kind}").into()),
    };
    Ok(String::from_utf8(d2_rs_language::format::format(&node))?)
}
