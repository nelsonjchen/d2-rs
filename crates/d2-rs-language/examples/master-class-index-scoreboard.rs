// SPDX-License-Identifier: MPL-2.0
//! Replay complete original class lookup sessions on native IR.
#[path = "support/ir_json.rs"]
mod projection;
use d2_rs_core::ast;
use d2_rs_language::{compiler, parse};
use serde_json::{Value, json};
use std::{cell::Cell, rc::Rc};

pub fn evaluate(request: Value) -> Value {
    let parsed = parse(
        request["path"].as_str().unwrap(),
        request["input"].as_str().unwrap().as_bytes(),
        None,
    );
    assert!(parsed.error.is_none());
    let (compiler, root) = compiler::compile_ir(ast::Shared::new(parsed.map)).unwrap();
    assert!(compiler.errors.is_empty());
    let calls = Rc::new(Cell::new(0));
    let context: Option<compiler::CompileContext> = match request["context_policy"]["kind"].as_str()
    {
        Some("background") => None,
        Some("cancel_after_checks") => {
            calls.set(request["context_policy"]["initial_calls"].as_u64().unwrap());
            let cancel_at = request["context_policy"]["cancel_at"].as_u64().unwrap();
            let checks = calls.clone();
            Some(Box::new(move || {
                checks.set(checks.get() + 1);
                (checks.get() >= cancel_at).then_some(d2_rs_core::LayoutError::Cancelled)
            }))
        }
        _ => return json!({"unsupported":"context policy"}),
    };
    let mut lookup = compiler::ClassLookup::new(&compiler.arena, context);
    let results: Vec<_> = request["queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|query| {
            lookup
                .get(root, query.as_str().unwrap().as_bytes())
                .ok()
                .flatten()
                .map_or(Value::Null, |map| projection::ir_json(&compiler.arena, map))
        })
        .collect();
    let error = lookup.error();
    let kind = match error {
        Some(d2_rs_core::LayoutError::Cancelled) => "canceled",
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => "deadline",
        _ => "",
    };
    json!({"results":results,"indexed_roots":lookup.indexed_root_count(),
        "error":error.map(ToString::to_string).unwrap_or_default(),"error_kind":kind,
        "context_checks":(request["context_policy"]["kind"] != "background").then_some(calls.get())})
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).ok_or("request path")?,
    )?)?;
    println!("{}", evaluate(request));
    Ok(())
}
