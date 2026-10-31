// SPDX-License-Identifier: MPL-2.0
//! Original master import-cache cancellation witnesses and ownership contract.
#[allow(dead_code)]
#[path = "../examples/master-ast-clone-scoreboard.rs"]
mod adapter;

use d2_rs_core::{
    LayoutError,
    ast::{Child, Node as _, Shared},
    ir::clone_ast_with_context,
};

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_master_ast_clone_calls() {
    let corpus: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-ast-clone.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 65);
    for case in cases {
        assert_eq!(
            adapter::evaluate(case["request"].clone()),
            case["got"],
            "{}",
            case["name"]
        );
    }
}

#[test]
fn canceled_working_copies_preserve_source_and_context_cause() {
    let parsed = d2_rs_language::parse(
        "cache.d2",
        b"# comment\nvars: {x: world}\na: label {items: [1; true; null; 'hi'; \"hello ${x}\"; {child: v}; ...@library.items]; b -> c: edge}\n",
        None,
    );
    assert!(parsed.error.is_none());
    let source = Child::Map(Shared::new(parsed.map));
    let original = serde_json::to_value(&source).unwrap();
    // Every boundary either returns a complete tree or the original cause.
    // Repeated failures must leave the immutable import template reusable.
    let mut canceled = 0;
    let mut completed = 0;
    for cancel_at in 1..=256 {
        let mut calls = 0;
        match clone_ast_with_context(&source, &mut || {
            calls += 1;
            (calls >= cancel_at).then_some(LayoutError::DeadlineExceeded)
        }) {
            Err(LayoutError::DeadlineExceeded) => canceled += 1,
            Ok(cloned) => {
                completed += 1;
                // Editing every cloned string and numeric allocation cannot
                // mutate the parsed template or any previous working copy.
                let mut stack = vec![cloned];
                while let Some(node) = stack.pop() {
                    stack.extend(node.children());
                    match node {
                        Child::Unquoted(value) => value.write().value = None,
                        Child::Number(value) => {
                            if let Some(number) = &value.read().value {
                                *number.write() = Default::default();
                            }
                        }
                        _ => {}
                    }
                }
            }
            other => panic!("unexpected clone result: {other:?}"),
        }
        assert_eq!(serde_json::to_value(&source).unwrap(), original);
    }
    assert!(canceled > 0 && completed > 0);
}
