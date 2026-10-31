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
    assert_eq!((faults, successes), (97, 51));
    assert!(
        render("x + y").is_ok(),
        "faults must not poison the next parse"
    );
}
