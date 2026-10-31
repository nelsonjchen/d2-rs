// SPDX-License-Identifier: MPL-2.0
use super::*;
#[cfg(feature = "oracle-fixtures")]
#[test]
fn native_stacks_displaylines_column_specs_and_double_struck_match_original_go() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/stacks-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 415);
    assert_source_rows(rows);
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn native_backslash_identifier_fonts_scripts_and_delimiters_match_original_go() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/backslash-source-v090.json")).unwrap();
    assert_eq!(rows.len(), 279);
    assert_source_rows(rows);
}
fn assert_source_rows(rows: Vec<serde_json::Value>) {
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let size = measure(expression);
        assert_eq!(
            size.as_ref().ok().map(|s| s.width),
            row["width"].as_f64(),
            "{expression} width"
        );
        assert_eq!(
            size.as_ref().ok().map(|s| s.height),
            row["height"].as_f64(),
            "{expression} height"
        );
        assert_eq!(
            size.err().map(|e| e.to_string()),
            row["measure_error"].as_str().map(str::to_owned),
            "{expression} measure error"
        );
        match render(expression) {
            Ok(svg) => {
                assert!(row["render_error"].is_null());
                assert_eq!(
                    enclose_contract_tests::inventory(&svg),
                    enclose_contract_tests::inventory(row["svg"].as_str().unwrap()),
                    "{expression}"
                );
            }
            Err(e) => assert_eq!(
                Some(e.to_string()),
                row["render_error"].as_str().map(str::to_owned),
                "{expression} render error"
            ),
        }
    }
}
