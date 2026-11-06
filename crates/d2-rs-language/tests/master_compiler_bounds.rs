// SPDX-License-Identifier: MPL-2.0
//! Requests/results captured from original master compiler test assertions.
#![cfg(feature = "oracle-fixtures")]
#[allow(dead_code)]
#[path = "../examples/master-language-scoreboard.rs"]
mod adapter;

fn same_json(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a == b || a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_json(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|v| same_json(value, v)))
        }
        _ => left == right,
    }
}

#[test]
fn original_master_compiler_bounds_links_and_config_contracts() {
    let corpus: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-compiler-bounds.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 34);
    for case in cases {
        let actual = adapter::evaluate(case["request"].clone());
        assert!(
            same_json(&case["got"], &actual),
            "{}\nexpected: {}\nactual: {}",
            case["name"],
            case["got"],
            actual
        );
    }
}
