// SPDX-License-Identifier: MPL-2.0
//! Original master inline-test inputs and diagnostics, not reconstructed graphs.
#![cfg(feature = "oracle-fixtures")]
use d2_rs_core::ast::Shared;
use d2_rs_language::{compiler, parse};

#[test]
fn master_variable_expansion_and_cycle_contracts() {
    let corpus: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-variable-expansion.json"
    ))
    .unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        let request = &case["request"];
        let parsed = parse(
            request["path"].as_str().unwrap(),
            request["input"].as_str().unwrap().as_bytes(),
            None,
        );
        assert!(parsed.error.is_none(), "{}", case["name"]);
        let ast = Shared::new(parsed.map);
        let (c, root) = compiler::compile_ir_with_limits(
            ast.clone(),
            compiler::CompileOptions::default(),
            compiler::CompileLimits {
                max_variable_expansion: request["max_variable_expansion"].as_i64().unwrap(),
                ..Default::default()
            },
        )
        .unwrap();
        let actual = if c.errors.is_empty() {
            compiler::source_graph_json(&c, root, ast)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default()
        } else {
            c.errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(actual, case["error"].as_str().unwrap(), "{}", case["name"]);
    }
}
