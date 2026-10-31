// SPDX-License-Identifier: MPL-2.0
//! Public master Markdown substitution API, observed in original Go tests.
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn evaluate(request: Value) -> Value {
    let mut calls = request["context_policy"]["initial_calls"]
        .as_u64()
        .unwrap_or(0);
    let cancel_at = request["context_policy"]["cancel_at"].as_u64();
    let deadline = request["context_policy"]["remaining_ns"]
        .as_i64()
        .map(|ns| std::time::Instant::now() + std::time::Duration::from_nanos(ns.max(0) as u64));
    let kind = request["context_policy"]["kind"].as_str().unwrap();
    if !matches!(kind, "background" | "cancel_after_checks" | "deadline") {
        return json!({"unsupported":"unknown context policy"});
    }
    let mut check = || {
        calls += 1;
        if cancel_at.is_some_and(|at| calls >= at) {
            Some(d2_rs_core::LayoutError::Cancelled)
        } else if deadline.is_some_and(|at| std::time::Instant::now() >= at) {
            Some(d2_rs_core::LayoutError::DeadlineExceeded)
        } else {
            None
        }
    };
    let variables: BTreeMap<String, String> =
        serde_json::from_value(request["variables"].clone()).unwrap();
    match d2_rs_text::markdown::substitute::replace_substitutions_bounded_with_context(
        request["source"].as_str().unwrap().as_bytes(),
        &variables,
        request["limit"].as_i64().unwrap(),
        &mut check,
    ) {
        Ok((result, used)) => json!({"result":result,"used":used,"error":"","error_kind":""}),
        Err(error) => {
            use d2_rs_text::markdown::substitute::SubstitutionError;
            let kind = match &error {
                SubstitutionError::Context(d2_rs_core::LayoutError::Cancelled) => "canceled",
                SubstitutionError::Context(d2_rs_core::LayoutError::DeadlineExceeded) => "deadline",
                _ => "",
            };
            json!({"result":"","used":0,"error":error.to_string(),"error_kind":kind})
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).ok_or("request path")?,
    )?)?;
    println!("{}", evaluate(request));
    Ok(())
}
