// SPDX-License-Identifier: MPL-2.0
//! Complete calls captured from master's original Markdown API tests.
#[allow(dead_code)]
#[path = "../examples/master-markdown-scoreboard.rs"]
mod adapter;
use d2_rs_core::LayoutError;
use d2_rs_text::markdown::substitute::{
    SubstitutionError, replace_substitutions_bounded_with_context as replace,
};
use std::collections::BTreeMap;

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_master_substitution_calls() {
    let corpus: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-markdown-substitutions.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 13);
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
fn negative_limit_precedes_context_and_fast_paths_observe_deadlines() {
    let mut calls = 0;
    let mut check = || {
        calls += 1;
        Some(LayoutError::DeadlineExceeded)
    };
    assert!(matches!(
        replace(b"", &BTreeMap::new(), -1, &mut check),
        Err(SubstitutionError::NegativeLimit)
    ));
    assert!(matches!(
        replace(b"", &BTreeMap::new(), 0, &mut check),
        Err(SubstitutionError::Context(LayoutError::DeadlineExceeded))
    ));
    assert_eq!(calls, 1);
}

#[test]
fn cancellation_discards_work_across_text_nodes_and_preserves_inputs() {
    let source = b"${x} *next ${x}* **more ${x}** `code ${x}`";
    let variables = BTreeMap::from([("x".to_owned(), "value".to_owned())]);
    let original = variables.clone();
    let mut calls = 0;
    let error = replace(source, &variables, 100, &mut || {
        calls += 1;
        (calls >= 20).then_some(LayoutError::Cancelled)
    })
    .unwrap_err();
    assert!(matches!(
        error,
        SubstitutionError::Context(LayoutError::Cancelled)
    ));
    assert_eq!(calls, 20);
    assert_eq!(variables, original);
    assert_eq!(
        replace(source, &variables, 100, &mut || None).unwrap(),
        ("value *next value* **more value** `code ${x}`".into(), 18)
    );
}

#[test]
fn resource_failure_returns_no_partial_replacement_and_remains_reusable() {
    let variables = BTreeMap::from([("x".to_owned(), "é".to_owned())]);
    assert!(matches!(
        replace(b"${x} *${x}*", &variables, 5, &mut || None),
        Err(SubstitutionError::ExpansionLimit)
    ));
    assert_eq!(
        replace(b"${x} *${x}*", &variables, 6, &mut || None).unwrap(),
        ("é *é*".into(), 6)
    );
}
