// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "oracle-fixtures")]
#[path = "../examples/import-selection-oracle.rs"]
#[allow(dead_code)]
mod oracle;
#[test]
fn import_selection_matches_pinned_go() {
    // Go formats missing-key diagnostics from raw AST pointers, so their text
    // changes between runs. The generator drops `errors` from those rows and
    // marks them `unstable_errors`; the rest of the selection still compares.
    let cases: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/import-selection.json"
    ))
    .unwrap();
    for case in cases {
        let mut actual = oracle::evaluate(serde_json::from_value(case["request"].clone()).unwrap());
        if case["unstable_errors"] == true {
            let errors = actual.as_object_mut().unwrap().remove("errors");
            assert!(
                errors.is_some_and(|errors| !errors.as_array().unwrap().is_empty()),
                "{}: expected a missing-key diagnostic",
                case["request"]
            );
        }
        assert_eq!(actual, case["reference"], "{}", case["request"]);
    }
}
