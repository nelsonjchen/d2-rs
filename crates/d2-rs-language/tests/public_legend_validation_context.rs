// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "oracle-fixtures")]
use d2_rs_core::{Size, ast::Shared};
use d2_rs_language::{ParseOptions, compiler, parse};
use serde_json::{Value, json};
struct Unmeasured;
impl compiler::GraphMeasurer for Unmeasured {
    fn measure(&self, _: &str) -> Size {
        Size::default()
    }
}
#[test]
fn original_legend_compiles_objects_without_board_late_validation_or_nested_legends() {
    let requests: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/legend-validation-inputs-v090.json")).unwrap();
    let expected: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/legend-validation-source-v090.json"
    ))
    .unwrap();
    assert_eq!(requests.len(), 20);
    assert_eq!(expected.len(), 20);
    for (request, source) in requests.iter().zip(&expected) {
        assert_eq!(request["name"], source["name"]);
        let parsed = parse(
            "legend.d2",
            request["source"].as_str().unwrap().as_bytes(),
            Some(ParseOptions::default()),
        );
        assert!(
            parsed.error.is_none(),
            "{}: {:?}",
            request["name"],
            parsed.error
        );
        let (c, root) = compiler::compile_ir_with_options(
            Shared::new(parsed.map),
            compiler::CompileOptions::default(),
        )
        .unwrap();
        let actual = match compiler::lower_graph_with_measurer(&c, root, &Unmeasured) {
            Ok(graph) => {
                json!({"name":request["name"],"error":Value::Null,"legend":graph.legend.as_ref().map(|legend|json!({"objects":legend.shapes.len(),"edges":legend.connections.len()}))})
            }
            Err(error) => {
                json!({"name":request["name"],"error":error.to_string(),"legend":Value::Null})
            }
        };
        assert_eq!(&actual, source, "{}", request["name"]);
    }
}
