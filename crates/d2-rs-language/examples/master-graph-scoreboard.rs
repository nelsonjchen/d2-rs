// SPDX-License-Identifier: MPL-2.0
//! Replay original graph-stage calls including post-Compile composite aliases.
use d2_rs_core::{ast, ir};
use d2_rs_language::{compiler, parse};
use serde_json::{Value, json};
use std::{cell::Cell, rc::Rc};

fn field(arena: &ir::Arena, root: ir::NodeId, path: &Value) -> ir::NodeId {
    let path = path
        .as_array()
        .unwrap()
        .iter()
        .map(|v| ast::raw_string_box(v.as_str().unwrap(), true))
        .collect::<Vec<_>>();
    arena.get_field(root, &path).expect("captured field path")
}

pub fn evaluate(request: Value) -> Value {
    let parsed = parse(
        request["path"].as_str().unwrap(),
        request["input"].as_str().unwrap().as_bytes(),
        None,
    );
    assert!(parsed.error.is_none());
    let ast = ast::Shared::new(parsed.map);
    let (mut c, root) = compiler::compile_ir_with_limits(
        ast.clone(),
        compiler::CompileOptions::default(),
        compiler::CompileLimits {
            max_variable_expansion: request["max_variable_expansion"].as_i64().unwrap(),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(c.errors.is_empty());
    for alias in request["composite_aliases"].as_array().unwrap() {
        let dst = field(&c.arena, root, &alias["field"]);
        let src = if alias["target"].as_array().unwrap().is_empty() {
            root
        } else {
            let f = field(&c.arena, root, &alias["target"]);
            c.arena.map(f).unwrap()
        };
        if let ir::Node::Field(f) = c.arena.node_mut(dst) {
            f.composite = Some(src);
        }
    }
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
        _ => return json!({"unsupported":"graph context policy"}),
    };
    let result = compiler::source_graph_json_with_context(&c, root, ast, context);
    let checks = (request["context_policy"]["kind"] != "background").then_some(calls.get());
    match result {
        Ok(graph) => json!({"graph":graph,"error":"","error_kind":"","context_checks":checks}),
        Err(error) => {
            let kind = match error {
                compiler::GraphCompileError::Cancelled => "canceled",
                compiler::GraphCompileError::DeadlineExceeded => "deadline",
                _ => "",
            };
            json!({"graph":null,"error":error.to_string(),"error_kind":kind,"context_checks":checks})
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
