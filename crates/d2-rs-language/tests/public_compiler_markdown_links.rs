// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "oracle-fixtures")]
use d2_rs_core::{Size, ast::Shared};
use d2_rs_language::{ParseOptions, compiler, parse};
use serde_json::Value;
use std::collections::HashMap;
fn bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
struct Unmeasured;
impl compiler::GraphMeasurer for Unmeasured {
    fn measure(&self, _: &str) -> Size {
        Size::default()
    }
}
#[test]
fn original_graph_compiler_validates_d2_sanitized_markdown() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/public-compiler-markdown-links-v090.json"
    ))
    .unwrap();
    assert_eq!(fixture["requests"].as_array().unwrap().len(), 16);
    for (request, source) in fixture["requests"]
        .as_array()
        .unwrap()
        .iter()
        .zip(fixture["source"].as_array().unwrap())
    {
        let utf16 = request["utf16"].as_bool().unwrap();
        let parsed = parse(
            request["path"].as_str().unwrap(),
            &bytes(request["hex"].as_str().unwrap()),
            Some(ParseOptions {
                utf16_positions: utf16,
                errors: None,
            }),
        );
        assert!(parsed.error.is_none());
        let files = request["files_hex"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), bytes(v.as_str().unwrap())))
            .collect::<HashMap<_, _>>();
        let (c, root) = compiler::compile_ir_with_options(
            Shared::new(parsed.map),
            compiler::CompileOptions {
                utf16_positions: utf16,
                import_opener: Some(Box::new(move |path| {
                    files
                        .get(path)
                        .map(|b| {
                            Box::new(std::io::Cursor::new(b.clone()))
                                as Box<dyn d2_rs_language::stream::ByteReader>
                        })
                        .ok_or_else(|| format!("open {path}: file does not exist"))
                })),
            },
        )
        .unwrap();
        let error = compiler::lower_graph_with_measurer(&c, root, &Unmeasured)
            .err()
            .map(|error| error.to_string());
        assert_eq!(error.as_deref(), source["error"].as_str(), "{request}");
    }
}
