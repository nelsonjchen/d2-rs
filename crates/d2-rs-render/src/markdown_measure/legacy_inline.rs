// SPDX-License-Identifier: MPL-2.0
#[cfg(all(test, feature = "oracle-fixtures"))]
use super::render_html;

/// The `block-` documents of the merged recursive-paint oracle artifact
/// (formerly the separate markdown-block-measure fixture).
#[cfg(all(test, feature = "oracle-fixtures"))]
fn block_measure_rows() -> serde_json::Value {
    let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/markdown-recursive-paint-source-v090.json"
    ))
    .unwrap();
    serde_json::Value::Array(
        rows.as_array()
            .unwrap()
            .iter()
            .filter(|row| {
                row["request"]["Name"]
                    .as_str()
                    .unwrap()
                    .starts_with("block-")
            })
            .cloned()
            .collect(),
    )
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod source_contract {
    use crate::{MarkdownFamily, native_markdown_measurer};
    #[test]
    fn complete_original_natural_paragraph_viewports() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-natural-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 396);
        let mut failures = Vec::new();
        for row in rows {
            let request = &row["request"];
            let family = match request["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let actual = native_markdown_measurer(family)
                .unwrap()
                .measurer()
                .markdown_dimensions(
                    request["Markdown"].as_str().unwrap(),
                    request["FontSize"].as_u64().unwrap() as u32,
                    false,
                    false,
                    matches!(family, MarkdownFamily::Mono),
                );
            let expected = crate::Size {
                width: row["layout"]["Width"].as_f64().unwrap(),
                height: row["layout"]["Height"].as_f64().unwrap(),
            };
            if actual != expected {
                failures.push((request["Name"].as_str().unwrap(), actual, expected));
            }
        }
        assert!(
            failures.is_empty(),
            "Original natural viewport differences: {failures:?}"
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod table_source_contract {
    use crate::{MarkdownFamily, native_markdown_measurer};
    use d2_rs_core::NodeId;
    use d2_rs_language::compiler::SourceNodeStyle;
    use std::collections::BTreeMap;

    #[test]
    fn complete_original_table_html_dimensions_and_error_inventory() {
        // markdown-table-natural was markdown-table-paint without `svg`; read the
        // merged recipe artifact (tools/fixtures/renderexport-render.toml).
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-table-paint-source-v090.json"
        ))
        .unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 252);
        let mut outputs = Vec::new();
        let mut errors = 0;
        for row in rows {
            let r = &row["request"];
            let text = r["Markdown"].as_str().unwrap();
            let labels = BTreeMap::from([(NodeId(0), text.to_owned())]);
            let styles = BTreeMap::from([(
                NodeId(0),
                SourceNodeStyle {
                    markdown: true,
                    ..Default::default()
                },
            )]);
            let error = crate::validate_markdown_labels(&labels, &styles)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert_eq!(
                error,
                row["error"].as_str().unwrap_or_default(),
                "{}",
                r["Name"]
            );
            if !error.is_empty() {
                errors += 1;
                outputs.push(serde_json::json!({"request":r,"error":error}));
                continue;
            }
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let html = super::render_html(text.as_bytes());
            assert_eq!(
                html,
                row["rendered_html"].as_str().unwrap(),
                "{}",
                r["Name"]
            );
            let actual = native_markdown_measurer(family)
                .unwrap()
                .measurer()
                .markdown_dimensions(
                    text,
                    r["FontSize"].as_u64().unwrap() as u32,
                    false,
                    false,
                    matches!(family, MarkdownFamily::Mono),
                );
            assert_eq!(
                actual.width,
                row["layout"]["Width"].as_f64().unwrap(),
                "{} width",
                r["Name"]
            );
            assert_eq!(
                actual.height,
                row["layout"]["Height"].as_f64().unwrap(),
                "{} height",
                r["Name"]
            );
            outputs.push(serde_json::json!({"request":r,"width":actual.width,"height":actual.height,"rendered_html":html}));
        }
        assert_eq!(errors, 9);
        if let Some(path) = std::env::var_os("D2_RS_TEST_TABLE_MEASURE_OUTPUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&outputs).unwrap()).unwrap();
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod mixed_block_source_contract {
    use crate::{MarkdownFamily, Size, native_markdown_measurer};
    #[test]
    fn complete_original_mixed_natural_viewports() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-mixed-flow-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 126);
        let mut failures = Vec::new();
        let mut outcomes = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let actual = native_markdown_measurer(family)
                .unwrap()
                .measurer()
                .markdown_dimensions(
                    r["Markdown"].as_str().unwrap(),
                    r["FontSize"].as_u64().unwrap() as u32,
                    false,
                    false,
                    matches!(family, MarkdownFamily::Mono),
                );
            let expected = Size {
                width: row["layout"]["Width"].as_f64().unwrap(),
                height: row["layout"]["Height"].as_f64().unwrap(),
            };
            if actual != expected {
                failures.push((r["Name"].clone(), actual, expected));
            }
            outcomes.push(serde_json::json!({"request":r,"actual":actual,"expected":expected,"matched":actual==expected}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_MIXED_NATURAL_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "Original mixed natural differences: {failures:?}"
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod block_tree_source_contract {
    use crate::{MarkdownFamily, Size, native_markdown_measurer};
    #[test]
    fn complete_original_block_tree_dimensions() {
        // markdown-block-measure was the `block-` rows of markdown-recursive-paint.
        let rows = super::block_measure_rows();
        assert_eq!(rows.as_array().unwrap().len(), 252);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        let mut accepted = 0;
        let mut errors = 0;
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            if let Some(error) = row.get("error") {
                let labels = std::collections::BTreeMap::from([(
                    d2_rs_core::NodeId(0),
                    r["Markdown"].as_str().unwrap().to_owned(),
                )]);
                let styles = std::collections::BTreeMap::from([(
                    d2_rs_core::NodeId(0),
                    d2_rs_language::compiler::SourceNodeStyle {
                        markdown: true,
                        ..Default::default()
                    },
                )]);
                let actual = crate::validate_markdown_labels(&labels, &styles)
                    .unwrap_err()
                    .to_string();
                assert_eq!(actual, error.as_str().unwrap());
                errors += 1;
                outcomes.push(serde_json::json!({"request":r,"error":actual}));
                continue;
            }
            accepted += 1;
            let fonts = native_markdown_measurer(family).unwrap();
            let measurer = fonts.measurer();
            let mono = matches!(family, MarkdownFamily::Mono);
            let size = r["FontSize"].as_u64().unwrap() as u32;
            let actual = measurer
                .legacy_html_dimensions(row["rendered_html"].as_str().unwrap(), size, mono)
                .expect("supported original tree");
            let expected = Size {
                width: row["layout"]["Width"].as_f64().unwrap(),
                height: row["layout"]["Height"].as_f64().unwrap(),
            };
            if actual != expected {
                failures.push((r["Name"].clone(), actual, expected));
            }
            let native = measurer.markdown_dimensions(
                r["Markdown"].as_str().unwrap(),
                size,
                false,
                false,
                mono,
            );
            outcomes.push(serde_json::json!({"request":r,"source_tree_actual":actual,"expected":expected,"source_tree_matched":actual==expected,"native_pipeline_actual":native,"native_pipeline_matched":native==expected}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_BLOCK_TREE_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert_eq!((accepted, errors), (234, 18));
        assert!(failures.is_empty(), "Source tree differences: {failures:?}");
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod block_document_source_contract {
    use crate::{MarkdownFamily, Size, native_markdown_measurer};
    #[test]
    fn complete_original_native_block_documents() {
        // markdown-block-measure was the `block-` rows of markdown-recursive-paint.
        let rows = super::block_measure_rows();
        assert_eq!(rows.as_array().unwrap().len(), 252);
        let mut failures = Vec::new();
        let mut outcomes = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let labels = std::collections::BTreeMap::from([(
                d2_rs_core::NodeId(0),
                r["Markdown"].as_str().unwrap().to_owned(),
            )]);
            let styles = std::collections::BTreeMap::from([(
                d2_rs_core::NodeId(0),
                d2_rs_language::compiler::SourceNodeStyle {
                    markdown: true,
                    ..Default::default()
                },
            )]);
            let error = crate::validate_markdown_labels(&labels, &styles)
                .err()
                .map(|e| e.to_string());
            if let Some(expected) = row.get("error") {
                assert_eq!(error.as_deref(), expected.as_str());
                outcomes.push(serde_json::json!({"request":r,"error":error}));
                continue;
            }
            assert!(error.is_none());
            let actual = native_markdown_measurer(family)
                .unwrap()
                .measurer()
                .markdown_dimensions(
                    r["Markdown"].as_str().unwrap(),
                    r["FontSize"].as_u64().unwrap() as u32,
                    false,
                    false,
                    matches!(family, MarkdownFamily::Mono),
                );
            let expected = Size {
                width: row["layout"]["Width"].as_f64().unwrap(),
                height: row["layout"]["Height"].as_f64().unwrap(),
            };
            let html =
                d2_rs_text::markdown::html::render_html(r["Markdown"].as_str().unwrap().as_bytes());
            let html_matched = Some(html.as_str()) == row["rendered_html"].as_str();
            let matched = actual == expected && html_matched;
            if !matched {
                failures.push((r["Name"].clone(), actual, expected, html_matched));
            }
            outcomes.push(serde_json::json!({"request":r,"actual":actual,"expected":expected,"html":html,"html_matched":html_matched,"matched":matched}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_BLOCK_DOCUMENT_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "Original native block differences: {failures:?}"
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod recursive_document_source_contract {
    use crate::{MarkdownFamily, Size, native_markdown_measurer};
    #[test]
    fn complete_original_native_recursive_documents() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-recursive-paint-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 864);
        let mut failures = Vec::new();
        let mut outcomes = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let labels = std::collections::BTreeMap::from([(
                d2_rs_core::NodeId(0),
                r["Markdown"].as_str().unwrap().to_owned(),
            )]);
            let styles = std::collections::BTreeMap::from([(
                d2_rs_core::NodeId(0),
                d2_rs_language::compiler::SourceNodeStyle {
                    markdown: true,
                    ..Default::default()
                },
            )]);
            let error = crate::validate_markdown_labels(&labels, &styles)
                .err()
                .map(|e| e.to_string());
            if let Some(expected) = row.get("error") {
                assert_eq!(error.as_deref(), expected.as_str());
                outcomes.push(serde_json::json!({"request":r,"error":error}));
                continue;
            }
            assert!(error.is_none());
            let actual = native_markdown_measurer(family)
                .unwrap()
                .measurer()
                .markdown_dimensions(
                    r["Markdown"].as_str().unwrap(),
                    r["FontSize"].as_u64().unwrap() as u32,
                    false,
                    false,
                    matches!(family, MarkdownFamily::Mono),
                );
            let expected = Size {
                width: row["layout"]["Width"].as_f64().unwrap(),
                height: row["layout"]["Height"].as_f64().unwrap(),
            };
            let html =
                d2_rs_text::markdown::html::render_html(r["Markdown"].as_str().unwrap().as_bytes());
            let html_matched = Some(html.as_str()) == row["rendered_html"].as_str();
            let matched = actual == expected && html_matched;
            if !matched {
                failures.push((r["Name"].clone(), actual, expected, html_matched));
            }
            outcomes.push(serde_json::json!({"request":r,"actual":actual,"expected":expected,"html":html,"html_matched":html_matched,"matched":matched}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_RECURSIVE_DOCUMENT_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "Original native block differences: {failures:?}"
        );
    }
}
