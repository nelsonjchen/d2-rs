// SPDX-License-Identifier: MPL-2.0
//! Native IR compile boundary: full IR, import inventory and exact diagnostics.
#[path = "support/ir_json.rs"]
mod ir_projection;
use d2_rs_core::ast::Shared;
use d2_rs_language::{compiler, parse};
use serde_json::{Value, json};
use std::{cell::Cell, rc::Rc};

pub(crate) fn evaluate(request: Value) -> Value {
    let canceled_on_open = Rc::new(Cell::new(false));
    let context: Option<compiler::CompileContext> = match request["context_policy"]["kind"].as_str()
    {
        Some("background") => None,
        Some("cancel_on_open") => {
            let canceled = canceled_on_open.clone();
            Some(Box::new(move || {
                canceled.get().then_some(d2_rs_core::LayoutError::Cancelled)
            }))
        }
        Some("cancel_after_checks") => {
            let Some(cancel_at) = request["context_policy"]["cancel_at"].as_u64() else {
                return json!({"unsupported":"invalid context cancellation threshold"});
            };
            let Some(mut calls) = request["context_policy"]["initial_calls"].as_u64() else {
                return json!({"unsupported":"missing initial context check count"});
            };
            Some(Box::new(move || {
                calls += 1;
                (calls >= cancel_at).then_some(d2_rs_core::LayoutError::Cancelled)
            }))
        }
        _ if request["has_context"] == true => return json!({"unsupported":"context policy"}),
        _ => None,
    };
    let parsed = parse(
        request["path"].as_str().unwrap(),
        request["input"].as_str().unwrap().as_bytes(),
        None,
    );
    if let Some(error) = parsed.error {
        return json!({"error":error.to_string(), "ir":null, "imports":null});
    }
    let opener = if request["has_fs"] == true {
        let files = request["files"].clone();
        let cancel_on_open = request["context_policy"]["kind"] == "cancel_on_open";
        Some(Box::new(move |path: &str| {
            files[path]
                .as_str()
                .map(|text| {
                    canceled_on_open.set(cancel_on_open);
                    Box::new(std::io::Cursor::new(text.as_bytes().to_vec()))
                        as Box<dyn d2_rs_language::stream::ByteReader>
                })
                .ok_or_else(|| format!("open {path}: file does not exist"))
        }) as compiler::ImportOpener)
    } else {
        None
    };
    let (c, root) = match compiler::compile_ir_with_context(
        Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: opener,
            ..Default::default()
        },
        compiler::CompileLimits {
            max_edge_expansion: request["max_edge_expansion"].as_i64().unwrap_or_default(),
            max_edge_expansion_work: request["max_edge_expansion_work"]
                .as_i64()
                .unwrap_or_default(),
            max_glob_expansion: request["max_glob_expansion"].as_i64().unwrap_or_default(),
            max_variable_expansion: request["max_variable_expansion"]
                .as_i64()
                .unwrap_or_default(),
            ..Default::default()
        },
        context,
    ) {
        Ok(value) => value,
        Err(error) => return json!({"unsupported":error.to_string()}),
    };
    if !c.errors.is_empty() {
        let mut result = json!({"error":c.errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"), "ir":null, "imports":null});
        if request["typed_error"] == true {
            result["error_kind"] = json!(if matches!(
                c.context_error,
                Some(d2_rs_core::LayoutError::Cancelled)
            ) {
                "canceled"
            } else {
                ""
            });
        }
        return result;
    }
    let mut result = json!({"error":"", "ir":ir_projection::ir_json(&c.arena,root), "imports":(!c.imports.is_empty()).then_some(c.imports)});
    if request["typed_error"] == true {
        result["error_kind"] = json!("");
    }
    result
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).ok_or("request path")?,
    )?)?;
    println!("{}", evaluate(request));
    Ok(())
}
