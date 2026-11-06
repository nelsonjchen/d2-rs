// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "oracle-fixtures")]
#[path = "../examples/ir-tree-oracle.rs"]
#[allow(dead_code)]
mod oracle;
#[test]
fn pinned_go_ir_tree_navigation() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/ir-tree.json"
    ))
    .unwrap();
    let requests = fixture["requests"].as_array().unwrap();
    let expected = fixture["reference"].as_array().unwrap();
    assert_eq!(requests.len(), expected.len());
    for (index, (request, expected)) in requests.iter().zip(expected).enumerate() {
        assert_eq!(
            &oracle::evaluate(serde_json::from_value(request.clone()).unwrap()),
            expected,
            "IR tree case {index}: {request}"
        );
    }
}
