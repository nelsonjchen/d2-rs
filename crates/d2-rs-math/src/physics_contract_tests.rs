// SPDX-License-Identifier: MPL-2.0
use super::*;
#[test]
fn native_physics_quantities_derivatives_functions_and_all_paint_match_original_go() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/physics-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 1227);
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
