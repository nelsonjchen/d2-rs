// SPDX-License-Identifier: MPL-2.0
//! Original cached import AST clone calls, complete result and typed error.
use d2_rs_core::{
    LayoutError,
    ast::{Child, Shared},
    ir::clone_ast_with_context,
};
use serde_json::{Value, json};

pub(crate) fn evaluate(request: Value) -> Value {
    let parsed = d2_rs_language::parse(
        request["path"].as_str().unwrap(),
        request["input"].as_str().unwrap().as_bytes(),
        None,
    );
    assert!(parsed.error.is_none());
    let mut calls = request["context_policy"]["initial_calls"].as_u64().unwrap();
    let cancel_at = request["context_policy"]["cancel_at"].as_u64().unwrap();
    let mut check = || {
        calls += 1;
        (calls >= cancel_at).then_some(LayoutError::Cancelled)
    };
    match clone_ast_with_context(&Child::Map(Shared::new(parsed.map)), &mut check) {
        Ok(Child::Map(map)) => json!({"map": map, "error":"", "error_kind":""}),
        Err(error) => {
            json!({"map":null, "error":error.to_string(), "error_kind":if matches!(error, LayoutError::Cancelled) {"canceled"} else {""}})
        }
        _ => unreachable!(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).ok_or("request path")?,
    )?)?;
    println!("{}", evaluate(request));
    Ok(())
}
