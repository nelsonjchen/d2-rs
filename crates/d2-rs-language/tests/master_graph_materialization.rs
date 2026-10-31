// SPDX-License-Identifier: MPL-2.0
#[path = "../examples/master-graph-scoreboard.rs"]
#[allow(dead_code)]
mod runner;
use d2_rs_core::{LayoutError, ast};
use d2_rs_language::{compiler, parse};
use serde_json::Value;
use std::{cell::Cell, rc::Rc};

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_graph_materialization_calls() {
    let fixture: Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/master-graph-materialization.json"))
            .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        assert_eq!(
            runner::evaluate(case["request"].clone()),
            case["got"],
            "{}",
            case["name"]
        );
    }
}

fn compile(
    source: &str,
    limit: i64,
) -> (
    compiler::Compiler,
    d2_rs_core::ir::NodeId,
    ast::Shared<ast::Map>,
) {
    let parsed = parse("graph-context.d2", source.as_bytes(), None);
    assert!(parsed.error.is_none());
    let ast = ast::Shared::new(parsed.map);
    let (c, root) = compiler::compile_ir_with_limits(
        ast.clone(),
        compiler::CompileOptions::default(),
        compiler::CompileLimits {
            max_variable_expansion: limit,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(c.errors.is_empty());
    (c, root, ast)
}

#[test]
fn deadline_precedes_attribute_errors_and_never_publishes_a_graph() {
    let (mut c, root, ast) = compile("a.shape: image", 0);
    assert_eq!(
        compiler::source_graph_json_with_context(
            &mut c,
            root,
            ast.clone(),
            Some(Box::new(|| Some(LayoutError::DeadlineExceeded)))
        )
        .unwrap_err(),
        compiler::GraphCompileError::DeadlineExceeded
    );
    assert!(matches!(
        compiler::source_graph_json_with_context(&mut c, root, ast, None),
        Err(compiler::GraphCompileError::InvalidAttributes(_))
            | Err(compiler::GraphCompileError::ImageMissingIcon(_))
    ));
}

#[test]
fn repeated_graph_calls_share_the_original_ir_copy_budget() {
    for context_api in [false, true] {
        let (c, root, ast) = compile("layers: {one: {a}}", 120);
        let mut passed = 0;
        loop {
            let result = if context_api {
                compiler::source_graph_json_with_context(&c, root, ast.clone(), None)
            } else {
                compiler::source_graph_json(&c, root, ast.clone())
            };
            match result {
                Ok(_) => passed += 1,
                Err(compiler::GraphCompileError::VariableExpansionLimit(120)) => break,
                Err(error) => panic!("unexpected error: {error}"),
            }
            assert!(passed < 20, "graph calls reset the compilation budget");
        }
        assert!(passed > 0);
    }
}

#[test]
fn canceled_copy_does_not_rewrite_the_callers_ir() {
    let (mut c, root, ast) = compile("layers: {one: {a}}", 0);
    let fields = match c.arena.node(root) {
        d2_rs_core::ir::Node::Map(m) => m.fields.clone(),
        _ => unreachable!(),
    };
    let calls = Rc::new(Cell::new(0));
    let checks = calls.clone();
    let result = compiler::source_graph_json_with_context(
        &mut c,
        root,
        ast,
        Some(Box::new(move || {
            checks.set(checks.get() + 1);
            (checks.get() >= 4).then_some(LayoutError::Cancelled)
        })),
    );
    assert_eq!(result.unwrap_err(), compiler::GraphCompileError::Cancelled);
    assert_eq!(calls.get(), 4);
    assert_eq!(
        match c.arena.node(root) {
            d2_rs_core::ir::Node::Map(m) => m.fields.clone(),
            _ => unreachable!(),
        },
        fields
    );
}
