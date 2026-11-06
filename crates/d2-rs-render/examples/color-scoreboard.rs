// SPDX-License-Identifier: MPL-2.0
//! Native adapter for original pinned lib/color tests. JSON is only transport.
use d2_rs_render::color::{Gradient, gradient_to_svg, parse_gradient};
use serde_json::{Value, json};
fn string<'a>(value: &'a Value, name: &str) -> &'a str {
    value[name].as_str().unwrap()
}
fn gradient(value: &Value) -> Gradient {
    Gradient {
        kind: string(value, "Type").into(),
        direction: string(value, "Direction").into(),
        id: string(value, "ID").into(),
        color_stops: value["ColorStops"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| (string(s, "Color").into(), string(s, "Position").into()))
            .collect(),
    }
}
fn evaluate(request: Value) -> Value {
    match string(&request, "op") {
        "parse" => {
            let parsed = parse_gradient(string(&request, "css"));
            let failed = parsed.is_none();
            let g = parsed.unwrap_or_default();
            let stops: Vec<_> = g
                .color_stops
                .iter()
                .map(|(c, p)| json!({"Color":c,"Position":p}))
                .collect();
            json!({"error":failed,"gradient":{"Type":g.kind,"Direction":g.direction,"ID":g.id,"ColorStops":if stops.is_empty(){Value::Null}else{json!(stops)}}})
        }
        "svg" => json!(gradient_to_svg(&gradient(&request["gradient"]))),
        // Exercise the existing compiler's lib/color.ValidColor consumer. This
        // is a boolean acceptance boundary, not a claim about error spelling.
        "valid" => {
            use d2_rs_core::ast::Shared;
            use d2_rs_language::{compiler, parse};
            let source = format!(
                "a.style.fill: {}",
                serde_json::to_string(string(&request, "css")).unwrap()
            );
            let parsed = parse("color.d2", source.as_bytes(), None);
            json!(
                parsed.error.is_none()
                    && compiler::compile_ir(Shared::new(parsed.map))
                        .is_ok_and(|(c, r)| compiler::lower_graph(&c, r).is_ok())
            )
        }
        op => panic!("unknown operation {op}"),
    }
}
fn main() {
    let input = std::env::args().nth(1).expect("request.json");
    let request = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    println!("{}", evaluate(request));
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn go_position_boundaries_and_constructed_radial_gradient() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/color-positions-api.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 11);
        for case in cases {
            assert_eq!(super::evaluate(case["request"].clone()), case["got"]);
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_pinned_color_api_calls() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/color-api.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 16);
        for case in cases {
            assert_eq!(
                super::evaluate(case["request"].clone()),
                case["got"],
                "{}",
                case["name"]
            );
        }
    }
}
