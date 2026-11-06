// SPDX-License-Identifier: MPL-2.0
use super::*;
#[test]
fn paired_delimiter_declarations_and_expansions_match_original_go() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/paired-delimiters-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 244);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        match (measure(expression), row["measure_error"].as_str()) {
            (Ok(size), None) => {
                assert_eq!(size.width, row["width"].as_f64().unwrap(), "{expression}");
                assert_eq!(size.height, row["height"].as_f64().unwrap(), "{expression}");
            }
            (Err(error), Some(expected)) => assert_eq!(error.to_string(), expected, "{expression}"),
            (actual, expected) => panic!("{expression}: {actual:?} vs {expected:?}"),
        }
        match (render(expression), row["render_error"].as_str()) {
            (Ok(svg), None) => assert_eq!(
                enclose_contract_tests::inventory(&svg),
                enclose_contract_tests::inventory(row["svg"].as_str().unwrap()),
                "{expression}"
            ),
            (Err(error), Some(expected)) => assert_eq!(error.to_string(), expected, "{expression}"),
            (actual, expected) => panic!("{expression}: {actual:?} vs {expected:?}"),
        }
    }
}
