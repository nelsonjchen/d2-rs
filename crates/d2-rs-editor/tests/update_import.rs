// SPDX-License-Identifier: MPL-2.0
//! Current pinned UpdateImport tests compare text, not historical graph files.
#![cfg(feature = "oracle-fixtures")]
use d2_rs_editor::update_import;
use serde_json::Value;

#[test]
fn pinned_update_import_text_contracts() {
    let fixture: Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/update-import-v090.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 19);
    for case in cases {
        let r = &case["request"];
        let actual = update_import(
            r["input"].as_str().unwrap(),
            r["old_path"].as_str().unwrap(),
            r["new_path"].as_str(),
        )
        .unwrap();
        assert_eq!(
            actual,
            r["expected_text"].as_str().unwrap(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn pinned_update_import_boundaries() {
    let cases: Vec<Value> =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/update-import-boundaries-v090.json")).unwrap();
    assert_eq!(cases.len(), 6);
    for case in cases {
        let actual = update_import(
            case["input"].as_str().unwrap(),
            case["old_path"].as_str().unwrap(),
            case["new_path"].as_str(),
        );
        if case["error"] == "" {
            assert_eq!(
                actual.unwrap(),
                case["text"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        } else {
            assert_eq!(
                actual.unwrap_err().to_string(),
                case["error"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}
