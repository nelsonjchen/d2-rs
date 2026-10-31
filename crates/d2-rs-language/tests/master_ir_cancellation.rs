// SPDX-License-Identifier: MPL-2.0
//! Pinned master TestCompileCancellationDuringIRConstruction and context API contracts.
#[allow(dead_code)]
#[path = "../examples/master-ir-scoreboard.rs"]
mod adapter;

use d2_rs_core::{CancellationToken, LayoutError, ast::Shared, ir::Node};
use d2_rs_language::{compiler, parse};
use std::{cell::Cell, rc::Rc};

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_master_ir_construction_cancellation() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-ir-cancellation.json"
    ))
    .unwrap();
    let case = &fixture["case"];
    assert_eq!(adapter::evaluate(case["request"].clone()), case["got"]);

    let parsed = parse(
        "canceled.d2",
        case["request"]["input"].as_str().unwrap().as_bytes(),
        None,
    );
    assert!(parsed.error.is_none());
    let calls = Rc::new(Cell::new(0));
    let counter = calls.clone();
    let (c, root) = compiler::compile_ir_with_context(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
        compiler::CompileLimits::default(),
        Some(Box::new(move || {
            counter.set(counter.get() + 1);
            (counter.get() >= 16).then_some(LayoutError::Cancelled)
        })),
    )
    .unwrap();
    assert!(matches!(c.context_error, Some(LayoutError::Cancelled)));
    assert_eq!(
        calls.get(),
        16,
        "canceled contexts must not be checked again"
    );
    let Node::Map(map) = c.arena.node(root) else {
        panic!("root map")
    };
    let fields = map.fields.as_ref().map_or(0, Vec::len);
    assert!(
        fields > 0 && fields < 128,
        "must stop during construction: {fields}"
    );
    assert_eq!(c.errors.len(), 1);
    assert_eq!(c.errors[0].to_string(), "context canceled");
}

#[test]
fn canceled_context_precedes_invalid_limits_and_import_io() {
    let parsed = parse("test.d2", b"...@unopened", None);
    assert!(parsed.error.is_none());
    let token = CancellationToken::default();
    token.cancel();
    let (c, root) = compiler::compile_ir_with_context(
        Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: Some(Box::new(|_| {
                panic!("canceled compilation opened an import")
            })),
            ..Default::default()
        },
        compiler::CompileLimits {
            max_variable_expansion: -1,
            ..Default::default()
        },
        Some(Box::new(move || token.error())),
    )
    .unwrap();
    assert!(matches!(c.context_error, Some(LayoutError::Cancelled)));
    assert_eq!(c.errors[0].to_string(), "context canceled");
    let Node::Map(map) = c.arena.node(root) else {
        panic!("root map")
    };
    assert!(map.fields.as_ref().is_none_or(Vec::is_empty));
}

#[test]
fn deadline_cause_supersedes_source_diagnostics() {
    let parsed = parse("test.d2", b"a: {\n...@missing\n}\nb\nc\nd", None);
    assert!(parsed.error.is_none());
    let cause = Rc::new(Cell::new(false));
    let failed_import = cause.clone();
    let (c, _) = compiler::compile_ir_with_context(
        Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: Some(Box::new(move |_| {
                failed_import.set(true);
                Err("file does not exist".into())
            })),
            ..Default::default()
        },
        compiler::CompileLimits::default(),
        Some(Box::new(move || {
            cause.get().then_some(LayoutError::DeadlineExceeded)
        })),
    )
    .unwrap();
    assert!(matches!(
        c.context_error,
        Some(LayoutError::DeadlineExceeded)
    ));
    assert_eq!(c.errors.len(), 1);
    assert_eq!(c.errors[0].to_string(), "context deadline exceeded");
}
