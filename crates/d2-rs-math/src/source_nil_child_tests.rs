// SPDX-License-Identifier: MPL-2.0
//! Original mathjax-go nil-child rendering and parser-error precedence.
use super::*;

#[test]
fn nil_children_fault_only_after_successful_top_level_parsing() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/source-nil-child-v090.json"
    ))
    .unwrap();
    let (mut faults, mut successes) = (0, 0);
    for case in fixture["cases"].as_array().unwrap() {
        let expression = case["expression"].as_str().unwrap();
        if !case["source_error"].is_null() {
            assert_eq!(case["source_error"]["code"], 500);
            assert_eq!(
                render(expression),
                Err(MathError::SourceNilChild),
                "{}",
                case["name"]
            );
            assert_eq!(
                measure(expression),
                Err(MathError::SourceNilChild),
                "{}",
                case["name"]
            );
            faults += 1;
        } else {
            let size = measure(expression).unwrap();
            let shape = &case["expected_diagram"]["shapes"][0];
            assert_eq!(
                size.width,
                shape["labelWidth"].as_f64().unwrap(),
                "{} width",
                case["name"]
            );
            assert_eq!(
                size.height,
                shape["labelHeight"].as_f64().unwrap(),
                "{} height",
                case["name"]
            );
            successes += 1;
        }
    }
    // This inventory describes the pinned Go fixture, not Rust behavior.
    // Master now compiles all 148 requests; derive the split rather than
    // transcribing the old v0.1.0 97/51 split (brief Update 1).
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 148);
    let expected_faults = cases
        .iter()
        .filter(|case| !case["source_error"].is_null())
        .count();
    assert_eq!(
        (faults, successes),
        (expected_faults, cases.len() - expected_faults)
    );
    assert!(
        render("x + y").is_ok(),
        "faults must not poison the next parse"
    );
}
