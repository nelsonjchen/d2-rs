// Actual pinned Go D2 v0.9.0 API output, captured through the trace overlay.
// This checks geometry and diagnostics independently of native AST wrappers.
use super::*;
use std::collections::BTreeMap;

fn multiply(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

fn local(raw: &str) -> [f64; 6] {
    let mut result = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    for operation in raw.split(')') {
        let Some((name, values)) = operation.trim().split_once('(') else {
            continue;
        };
        let v = values
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|x| !x.is_empty())
            .map(|x| x.parse::<f64>().unwrap())
            .collect::<Vec<_>>();
        let own = match name {
            "translate" => [1.0, 0.0, 0.0, 1.0, v[0], v.get(1).copied().unwrap_or(0.0)],
            "scale" => [v[0], 0.0, 0.0, v.get(1).copied().unwrap_or(v[0]), 0.0, 0.0],
            "matrix" => v.try_into().unwrap(),
            _ => panic!("unsupported source transform {name}"),
        };
        result = multiply(result, own);
    }
    result
}

fn world(node: roxmltree::Node<'_, '_>) -> [f64; 6] {
    node.ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .fold([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], |parent, child| {
            multiply(parent, local(child.attribute("transform").unwrap_or("")))
        })
}

fn attributes(node: roxmltree::Node<'_, '_>, omit_transform: bool) -> BTreeMap<String, String> {
    node.attributes()
        .filter(|a| !omit_transform || a.name() != "transform")
        .map(|a| (a.name().to_owned(), a.value().to_owned()))
        .collect()
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_prime_script_operator_metrics_diagnostics_and_complete_glyph_placement() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/prime-script-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 582);
    assert_source_rows(&rows);
}

// Compare independent original API output, not native node wrappers.
fn assert_source_rows(rows: &[serde_json::Value]) {
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        let emitted = render(expression).unwrap();
        let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
        let native = roxmltree::Document::parse(&emitted).unwrap();
        assert_eq!(
            attributes(native.root_element(), false),
            attributes(source.root_element(), false),
            "{expression}: complete root attributes"
        );
        let measured = measure(expression).unwrap();
        assert_eq!(
            (measured.width, measured.height),
            (
                row["width"].as_f64().unwrap(),
                row["height"].as_f64().unwrap()
            ),
            "{expression}: measurement"
        );
        let paths = |doc: &roxmltree::Document<'_>| {
            doc.descendants()
                .filter(|n| n.has_tag_name("path"))
                .map(|n| {
                    (
                        n.attribute("data-c").map(str::to_owned),
                        n.attribute("d").map(str::to_owned),
                        world(n),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            paths(&native),
            paths(&source),
            "{expression}: all glyph paths and six affine coordinates"
        );
        let texts = |doc: &roxmltree::Document<'_>| {
            doc.descendants()
                .filter(|n| n.has_tag_name("text"))
                .map(|n| {
                    (
                        attributes(n, true),
                        n.descendants()
                            .filter(|c| c.is_text())
                            .filter_map(|c| c.text())
                            .collect::<String>(),
                        world(n),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            texts(&native),
            texts(&source),
            "{expression}: all text attributes, content and placement"
        );
        // Nested SVG viewport attributes control repeated delimiter clips;
        // rule geometry and its inherited transform are independent of glyphs.
        let paint_frames = |doc: &roxmltree::Document<'_>| {
            doc.descendants()
                .filter(|n| {
                    n.has_tag_name("rect") || (n.has_tag_name("svg") && *n != doc.root_element())
                })
                .map(|n| {
                    (
                        n.tag_name().name().to_owned(),
                        attributes(n, true),
                        world(n),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            paint_frames(&native),
            paint_frames(&source),
            "{expression}: complete rule and nested viewport attributes and placement"
        );
        let errors = |doc: &roxmltree::Document<'_>| {
            doc.descendants()
                .filter_map(|n| n.attribute("data-mjx-error"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            errors(&native),
            errors(&source),
            "{expression}: complete diagnostics"
        );
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_four_removed_witnesses_and_all_whitespace_forms_use_general_parsing() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/general-former-witness-source-v090.json"))
    .unwrap();
    assert_eq!(rows.len(), 16);
    assert_source_rows(&rows);
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_complete_raw_operator_spelling_inventory() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/raw-operator-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 4895);
    assert_source_rows(&rows);
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_explicit_spacing_preserves_scriptlevel_and_notchar_precedence() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/explicit-spacing-source-v090.json"))
    .unwrap();
    assert_eq!(rows.len(), 104);
    assert_source_rows(&rows);
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_tables_reset_prime_in_radicals_fractions_and_scripts() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/table-prime-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 96);
    assert_source_rows(&rows);
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_complete_fraction_frame_investigation_including_tall_radicals() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/fraction-frame-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 55);
    assert_source_rows(&rows);
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_word_sum_and_whitespace_forms_use_general_parsing() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/word-sum-source-v090.json")).unwrap();
    assert_eq!(rows.len(), 4);
    assert_source_rows(&rows);
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_raw_operator_script_core_bounds_variants_and_italic_corrections() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/operator-context-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 252);
    assert_source_rows(&rows);
}
