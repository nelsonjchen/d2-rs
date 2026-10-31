// SPDX-License-Identifier: MPL-2.0
//! Complete outputs from repeated pristine Go D2 v0.9.0 calls. These cases
//! cover every upstream TestMove fixture (including upstream skips), both
//! descendant settings, reference moves, boards and virtual imports.
//! This finite corpus does not establish the entire Move API contract.
#![cfg(feature = "oracle-fixtures")]
use serde::Deserialize;

#[derive(Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    #[serde(default)]
    source: String,
    #[serde(default)]
    error: String,
    panic: Option<String>,
}
#[derive(Deserialize)]
struct Input {
    name: String,
    source: String,
    old_key: String,
    new_key: String,
    include_descendants: bool,
    path: String,
    #[serde(default)]
    files: std::collections::HashMap<String, String>,
    #[serde(default)]
    board_path: Vec<String>,
}

#[test]
fn reference_moves_match_complete_v090_outputs_and_errors() {
    check_source_cases(d2_rs_testdata::oracle_fixture!("tests/fixtures/move-transplant-v090.json"), 392, 0);
}

#[test]
fn move_boundaries_match_complete_v090_outputs_and_errors() {
    check_source_cases(d2_rs_testdata::oracle_fixture!("tests/fixtures/move-boundary-v090.json"), 62, 0);
}

#[test]
fn edge_moves_match_source_results_and_reject_source_panics() {
    check_source_cases(d2_rs_testdata::oracle_fixture!("tests/fixtures/move-edge-v090.json"), 176, 8);
}

fn check_source_cases(json: &str, expected_cases: usize, expected_source_panics: usize) {
    let corpus: Corpus = serde_json::from_str(json).unwrap();
    assert_eq!(
        corpus.cases.len(),
        expected_cases,
        "fixture inventory must stay complete"
    );
    assert_eq!(
        corpus
            .cases
            .iter()
            .filter(|case| case.panic.is_some())
            .count(),
        expected_source_panics
    );
    for case in corpus.cases {
        let input = case.input;
        let result = d2_rs_editor::move_object_with_options(
            &input.source,
            &input.old_key,
            &input.new_key,
            input.include_descendants,
            &d2_rs_editor::MoveOptions {
                path: input.path,
                files: input.files,
                board_path: input.board_path,
            },
        );
        if let Some(panic) = case.panic {
            // Source-panic differences remain in the inventory. A safe native
            // rejection is required, but is not counted as a source match.
            assert!(
                result.is_err(),
                "{}: source panics ({panic}); native must reject safely",
                input.name
            );
        } else if case.error.is_empty() {
            assert_eq!(
                result
                    .unwrap_or_else(|e| panic!("{}: {e}", input.name))
                    .source,
                case.source,
                "{}",
                input.name
            );
        } else {
            assert_eq!(
                result.unwrap_err().to_string(),
                case.error,
                "{}",
                input.name
            );
        }
    }
}
