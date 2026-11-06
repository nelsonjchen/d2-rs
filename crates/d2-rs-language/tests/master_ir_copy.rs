// SPDX-License-Identifier: MPL-2.0
#[path = "../examples/master-ir-copy-scoreboard.rs"]
#[allow(dead_code)]
mod runner;
use d2_rs_core::{LayoutError, ast, ir};
use d2_rs_language::{compiler, parse};
use std::{cell::Cell, rc::Rc};

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_copy_sessions_and_node_units() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-ir-copy.json"
    ))
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
fn compile(source: &str, limit: i64) -> (compiler::Compiler, ir::NodeId) {
    let parsed = parse("copy-context.d2", source.as_bytes(), None);
    assert!(parsed.error.is_none());
    let (c, root) = compiler::compile_ir_with_limits(
        ast::Shared::new(parsed.map),
        compiler::CompileOptions::default(),
        compiler::CompileLimits {
            max_variable_expansion: limit,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(c.errors.is_empty());
    (c, root)
}
#[test]
fn cancellation_preserves_ir_and_only_consumes_completed_node_charges() {
    let (c, root) = compile("x", 10);
    let calls = Rc::new(Cell::new(0));
    let checks = calls.clone();
    let fields = match c.arena.node(root) {
        ir::Node::Map(m) => m.fields.clone(),
        _ => unreachable!(),
    };
    let error = c
        .reserve_variable_expansion_copy(
            root,
            Some(Box::new(move || {
                checks.set(checks.get() + 1);
                (checks.get() >= 2).then_some(LayoutError::Cancelled)
            })),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        compiler::VariableExpansionError::Context(LayoutError::Cancelled)
    ));
    assert_eq!(calls.get(), 2);
    assert_eq!(c.variable_expansion_usage(), (10, 2));
    assert_eq!(
        match c.arena.node(root) {
            ir::Node::Map(m) => m.fields.clone(),
            _ => unreachable!(),
        },
        fields
    );
    // Public reservations do not latch the context into the compiler. A fresh
    // operation can continue against its remaining compilation-wide budget.
    c.reserve_variable_expansion_copy(root, None).unwrap();
    assert_eq!(c.variable_expansion_usage(), (10, 9));
}
#[test]
fn deadline_is_typed_and_precedes_even_an_exhausted_budget() {
    let (c, root) = compile("x", 1);
    let error = c
        .reserve_variable_expansion_copy(
            root,
            Some(Box::new(|| Some(LayoutError::DeadlineExceeded))),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        compiler::VariableExpansionError::Context(LayoutError::DeadlineExceeded)
    ));
    assert_eq!(error.to_string(), "context deadline exceeded");
    assert_eq!(c.variable_expansion_usage(), (1, 0));
    assert!(std::error::Error::source(&error).is_some());
    assert!(matches!(
        c.reserve_variable_expansion_copy(root, None),
        Err(compiler::VariableExpansionError::Limit(1))
    ));
}
#[test]
fn nested_map_copy_uses_the_compilation_budget_and_charges_shared_occurrences() {
    let (mut c, root) = compile("a: {leaf}\nb: {leaf}", 0);
    let a = c
        .arena
        .get_field(root, &[ast::raw_string_box("a", true)])
        .unwrap();
    let b = c
        .arena
        .get_field(root, &[ast::raw_string_box("b", true)])
        .unwrap();
    let child = c.arena.map(a).unwrap();
    if let ir::Node::Field(field) = c.arena.node_mut(b) {
        field.composite = Some(child)
    }
    let before = c.variable_expansion_usage().1;
    c.reserve_variable_expansion_copy(child, None).unwrap();
    let child_cost = c.variable_expansion_usage().1 - before;
    let before = c.variable_expansion_usage().1;
    c.reserve_variable_expansion_copy(root, None).unwrap();
    let root_cost = c.variable_expansion_node_units(root)
        + c.variable_expansion_node_units(a)
        + c.variable_expansion_node_units(b);
    assert_eq!(
        c.variable_expansion_usage().1 - before,
        root_cost + 2 * child_cost
    );
}
