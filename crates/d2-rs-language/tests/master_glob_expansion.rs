// SPDX-License-Identifier: MPL-2.0
//! Original master IR Compile calls, including exact boundaries and diagnostics.
#![cfg(feature = "oracle-fixtures")]
#[allow(dead_code)]
#[path = "../examples/master-ir-scoreboard.rs"]
mod adapter;

#[test]
fn original_master_glob_ir_contracts() {
    let corpus: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-glob-expansion.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 13);
    for case in cases {
        let actual = adapter::evaluate(case["request"].clone());
        assert_eq!(actual, case["got"], "{}", case["name"]);
    }
}
