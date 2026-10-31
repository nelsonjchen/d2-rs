// SPDX-License-Identifier: MPL-2.0
//! Every live pinned TestDelete and TestSet formatted-source/error result,
//! from the requests recorded by recipe mathtext-txt-editor-graphs.
#![cfg(feature = "oracle-fixtures")]
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Case {
    name: String,
    request: Request,
    error: String,
}
#[derive(Deserialize)]
struct Request {
    input: String,
    path: String,
    key: String,
    value: Option<String>,
    tag: Option<String>,
    files: Option<HashMap<String, String>>,
    board_path: Option<Vec<String>>,
    expected_text: String,
}

fn check(operation: &str, count: usize) {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/editor-graphs-v090.json"
    ))
    .unwrap();
    let all = fixture["cases"].as_array().unwrap();
    let delete_set = all
        .iter()
        .filter(|case| matches!(case["request"]["operation"].as_str(), Some("delete" | "set")))
        .count();
    assert_eq!(delete_set, 224);
    let cases: Vec<Case> = all
        .iter()
        .filter(|case| case["request"]["operation"] == operation)
        .map(|case| serde_json::from_value(case.clone()).unwrap())
        .collect();
    assert_eq!(cases.len(), count);
    for case in cases {
        let request = case.request;
        let options = d2_rs_editor::MoveOptions {
            path: request.path,
            files: request.files.unwrap_or_default(),
            board_path: request.board_path.unwrap_or_default(),
        };
        let result = if operation == "delete" {
            d2_rs_editor::delete_object_with_options(&request.input, &request.key, &options)
                .map(|result| result.source)
        } else {
            d2_rs_editor::set_object_with_options(
                &request.input,
                &request.key,
                request.tag.as_deref(),
                request.value.as_deref(),
                &options,
            )
            .map(|result| result.source)
        };
        if case.error.is_empty() {
            assert_eq!(
                result.unwrap_or_else(|error| panic!("{}: {error}", case.name)),
                request.expected_text,
                "{}",
                case.name
            );
        } else {
            assert_eq!(result.unwrap_err().to_string(), case.error, "{}", case.name);
        }
    }
}

#[test]
fn upstream_delete_text_and_errors() {
    check("delete", 122);
}

#[test]
fn upstream_set_text_and_errors() {
    check("set", 102);
}
