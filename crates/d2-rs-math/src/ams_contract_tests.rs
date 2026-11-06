// SPDX-License-Identifier: MPL-2.0
use super::*;
#[cfg(feature = "oracle-fixtures")]
fn links(svg: &str) -> Vec<(String, String, String)> {
    let doc = roxmltree::Document::parse(svg).unwrap();
    doc.descendants()
        .filter(|n| n.is_element())
        .flat_map(|n| {
            n.attributes()
                .filter(|a| {
                    matches!(a.name(), "id" | "href")
                        || a.name() == "class"
                            && a.value()
                                .split_whitespace()
                                .any(|class| class == "MathJax_ref")
                })
                .map(move |a| {
                    (
                        n.tag_name().name().to_owned(),
                        a.name().to_owned(),
                        a.value().to_owned(),
                    )
                })
        })
        .collect()
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_equation_split_labels_tags_references_and_boxed_contracts() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/ams-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 273);
    for row in rows {
        let e = row["expression"].as_str().unwrap();
        let result = measure(e);
        match row["measure_error"].as_str() {
            Some(error) => assert_eq!(result.unwrap_err().to_string(), error, "{e}"),
            None => {
                let size = result.unwrap();
                assert_eq!(size.width, row["width"].as_f64().unwrap(), "{e}");
                assert_eq!(size.height, row["height"].as_f64().unwrap(), "{e}");
            }
        }
        let native = render(e).unwrap();
        let source = row["svg"].as_str().unwrap();
        assert_eq!(
            enclose_contract_tests::inventory(&native),
            enclose_contract_tests::inventory(source),
            "{e}"
        );
        assert_eq!(links(&native), links(source), "{e}");
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_aligned_optional_position_contracts() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/aligned-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 243);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let native = render(expression).unwrap();
        assert_eq!(
            enclose_contract_tests::inventory(&native),
            enclose_contract_tests::inventory(row["svg"].as_str().unwrap()),
            "{expression}"
        );
        let size = measure(expression).unwrap();
        assert_eq!(
            (size.width, size.height),
            (
                row["width"].as_f64().unwrap(),
                row["height"].as_f64().unwrap()
            ),
            "{expression}"
        );
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_tag_container_width_alignment_scale_and_phantom_contracts() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/tag-container-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 152);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let native = render(expression).unwrap();
        assert_eq!(
            enclose_contract_tests::inventory(&native),
            enclose_contract_tests::inventory(row["svg"].as_str().unwrap()),
            "{expression}"
        );
        assert_eq!(
            links(&native),
            links(row["svg"].as_str().unwrap()),
            "{expression}"
        );
        match row["measure_error"].as_str() {
            Some(error) => assert_eq!(
                measure(expression).unwrap_err().to_string(),
                error,
                "{expression}"
            ),
            None => {
                let size = measure(expression).unwrap();
                assert_eq!(
                    (size.width, size.height),
                    (
                        row["width"].as_f64().unwrap(),
                        row["height"].as_f64().unwrap()
                    ),
                    "{expression}"
                );
            }
        }
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_starred_matrix_and_alignment_argument_contracts() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/environment-arguments-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 63);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let native = render(expression).unwrap();
        assert_eq!(
            enclose_contract_tests::inventory(&native),
            enclose_contract_tests::inventory(row["svg"].as_str().unwrap()),
            "{expression}"
        );
        let size = measure(expression).unwrap();
        assert_eq!(
            (size.width, size.height),
            (
                row["width"].as_f64().unwrap(),
                row["height"].as_f64().unwrap()
            ),
            "{expression}"
        );
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_percentage_table_top_nested_scaled_and_long_width_contracts() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/percentage-table-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 118);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let native = render(expression).unwrap();
        assert_eq!(
            enclose_contract_tests::inventory(&native),
            enclose_contract_tests::inventory(row["svg"].as_str().unwrap()),
            "{expression}"
        );
        let size = measure(expression).unwrap();
        assert_eq!(
            (size.width, size.height),
            (
                row["width"].as_f64().unwrap(),
                row["height"].as_f64().unwrap()
            ),
            "{expression}"
        );
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_multline_multlined_options_and_colon_relation_contracts() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/multline-options-relations-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 184);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let native = render(expression).unwrap();
        assert_eq!(
            enclose_contract_tests::inventory(&native),
            enclose_contract_tests::inventory(row["svg"].as_str().unwrap()),
            "{expression}"
        );
        match row["measure_error"].as_str() {
            Some(error) => assert_eq!(
                measure(expression).unwrap_err().to_string(),
                error,
                "{expression}"
            ),
            None => {
                let size = measure(expression).unwrap();
                assert_eq!(
                    (size.width, size.height),
                    (
                        row["width"].as_f64().unwrap(),
                        row["height"].as_f64().unwrap()
                    ),
                    "{expression}"
                );
            }
        }
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_default_environment_dispatch_and_package_error_contracts() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/environment-dispatch-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 74);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let native = render(expression).unwrap();
        assert_eq!(
            enclose_contract_tests::inventory(&native),
            enclose_contract_tests::inventory(row["svg"].as_str().unwrap()),
            "{expression}"
        );
        match row["measure_error"].as_str() {
            Some(error) => assert_eq!(
                measure(expression).unwrap_err().to_string(),
                error,
                "{expression}"
            ),
            None => {
                let size = measure(expression).unwrap();
                assert_eq!(
                    (size.width, size.height),
                    (
                        row["width"].as_f64().unwrap(),
                        row["height"].as_f64().unwrap()
                    ),
                    "{expression}"
                );
            }
        }
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_spreadlines_recursive_spacing_flalign_and_cd_paint_contracts() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/spreadlines-flalign-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 128);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let native = render(expression).unwrap();
        assert_eq!(
            enclose_contract_tests::inventory(&native),
            enclose_contract_tests::inventory(row["svg"].as_str().unwrap()),
            "{expression}"
        );
        match row["measure_error"].as_str() {
            Some(error) => assert_eq!(
                measure(expression).unwrap_err().to_string(),
                error,
                "{expression}"
            ),
            None => {
                let size = measure(expression).unwrap();
                assert_eq!(
                    (size.width, size.height),
                    (
                        row["width"].as_f64().unwrap(),
                        row["height"].as_f64().unwrap()
                    ),
                    "{expression}"
                );
            }
        }
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_equation_split_nested_scope_styles_tags_and_reference_metadata() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/equation-split-nested-source-v090.json"
    ))
    .unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 1008);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        match (measure(expression), row["measure_error"].as_str()) {
            (Ok(size), None) => {
                assert_eq!(
                    (size.width, size.height),
                    (
                        row["width"].as_f64().unwrap(),
                        row["height"].as_f64().unwrap()
                    ),
                    "{expression}"
                );
            }
            (Err(error), Some(expected)) => assert_eq!(error.to_string(), expected, "{expression}"),
            (actual, expected) => panic!("{expression}: {actual:?} vs {expected:?}"),
        }
        assert!(row["render_error"].is_null());
        let native = render(expression).unwrap();
        let source = row["svg"].as_str().unwrap();
        assert_eq!(
            enclose_contract_tests::inventory(&native),
            enclose_contract_tests::inventory(source),
            "{expression}"
        );
        // Compare raw id/href/class attributes, including source whitespace.
        assert_eq!(links(&native), links(source), "{expression}");
        // These source controls now qualify the complete serialized SVG too.
        assert_eq!(native, source, "SVG bytes: {expression}");
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_named_and_attached_prime_tokens_have_distinct_source_svg() {
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/prime-token-origin-source-v090.json"
    ))
    .unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 32);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let size = measure(expression).unwrap();
        assert!(row["measure_error"].is_null() && row["render_error"].is_null());
        assert_eq!(
            (size.width, size.height),
            (
                row["width"].as_f64().unwrap(),
                row["height"].as_f64().unwrap()
            ),
            "{expression}"
        );
        assert_eq!(
            render(expression).unwrap(),
            row["svg"].as_str().unwrap(),
            "{expression}"
        );
    }
}
