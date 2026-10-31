// SPDX-License-Identifier: MPL-2.0
//! Original master import budget calls, complete IR/imports/errors.
#![cfg(feature = "oracle-fixtures")]
#[allow(dead_code)]
#[path = "../examples/master-ir-scoreboard.rs"]
mod adapter;

#[test]
fn original_master_import_expansion_contracts() {
    let corpus: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-import-expansion.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        assert_eq!(
            adapter::evaluate(case["request"].clone()),
            case["got"],
            "{}",
            case["name"]
        );
    }
}
