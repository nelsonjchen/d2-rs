// SPDX-License-Identifier: MPL-2.0
//! Complete live pinned TestRename text/error contract, including boards.
#[cfg(feature = "oracle-fixtures")]
#[derive(serde::Deserialize)]
struct Case {
    name: String,
    request: Request,
    error: String,
}
#[cfg(feature = "oracle-fixtures")]
#[derive(serde::Deserialize)]
struct Request {
    input: String,
    path: String,
    key: String,
    new_key: String,
    files: Option<std::collections::HashMap<String, String>>,
    board_path: Option<Vec<String>>,
    expected_text: String,
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn upstream_rename_text_and_errors() {
    // The live TestRename requests recorded by recipe mathtext-txt-editor-graphs.
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/editor-graphs-v090.json"
    ))
    .unwrap();
    let cases: Vec<Case> = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["request"]["operation"] == "rename")
        .map(|case| serde_json::from_value(case.clone()).unwrap())
        .collect();
    assert_eq!(cases.len(), 25);
    for case in cases {
        let request = case.request;
        let options = d2_rs_editor::MoveOptions {
            path: request.path,
            files: request.files.unwrap_or_default(),
            board_path: request.board_path.unwrap_or_default(),
        };
        let result = d2_rs_editor::rename_object_with_options(
            &request.input,
            &request.key,
            &request.new_key,
            &options,
        );
        if case.error.is_empty() {
            assert_eq!(
                result
                    .unwrap_or_else(|e| panic!("{}: {e}", case.name))
                    .source,
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
fn rename_returns_the_generated_literal_name() {
    let renamed = d2_rs_editor::rename_object("a\nb\n", "a", "b").unwrap();
    assert_eq!(renamed.key, "b 2");
    assert_eq!(renamed.source, "b 2\nb\n");
    let renamed = d2_rs_editor::rename_object("a\n", "a", "a.b").unwrap();
    assert_eq!(renamed.key, "a.b");
    assert_eq!(renamed.source, "\"a.b\"\n");
}
