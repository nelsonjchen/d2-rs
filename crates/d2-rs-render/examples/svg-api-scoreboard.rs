// SPDX-License-Identifier: MPL-2.0
//! Transport adapter for original pinned lib/svg test calls.
use base64::Engine;
use d2_rs_render::svg::*;
use serde_json::{Value, json};
fn s<'a>(v: &'a Value, n: &str) -> &'a str {
    v[n].as_str().unwrap()
}
fn bytes(v: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s(v, "bytes"))
        .unwrap()
}
fn pair(a: &Attribute) -> Value {
    json!([
        a.name(),
        base64::engine::general_purpose::STANDARD.encode(a.value())
    ])
}
fn evaluate(v: Value) -> Value {
    let result = std::panic::catch_unwind(|| match s(&v, "op") {
        "escape_attribute" => json!(escape_attribute(bytes(&v))),
        "attr" => pair(&attr(s(&v, "name"), bytes(&v))),
        "render_attributes" => {
            let attrs: Vec<_> = v["attributes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| {
                    attr(
                        a[0].as_str().unwrap(),
                        base64::engine::general_purpose::STANDARD
                            .decode(a[1].as_str().unwrap())
                            .unwrap(),
                    )
                })
                .collect();
            json!(render_attributes(&attrs))
        }
        "href_attributes" => json!(
            href_attributes(bytes(&v))
                .iter()
                .map(pair)
                .collect::<Vec<_>>()
        ),
        "scoped_id" => {
            json!(scoped_id(s(&v, "prefix"), s(&v, "component"), s(&v, "suffix")).as_str())
        }
        "local_iri" => json!(local_iri(&literal_id(s(&v, "id")))),
        "parse_local_iri" => match parse_local_iri(s(&v, "value")) {
            Ok(id) => json!({"error":false,"id":id.as_str()}),
            Err(_) => json!({"error":true,"id":""}),
        },
        "parse_attributes" => match parse_attributes(s(&v, "value")) {
            Ok(attrs) => {
                json!({"error":false,"attributes":if attrs.is_empty(){Value::Null}else{json!(attrs.iter().map(pair).collect::<Vec<_>>())}})
            }
            Err(_) => json!({"error":true,"attributes":null}),
        },
        "text" => json!(text(bytes(&v)).as_str()),
        "trusted_fragment" => json!(trusted_fragment(bytes(&v)).as_str()),
        op => panic!("unknown op {op}"),
    });
    match result {
        Ok(value) => json!({"value":value}),
        Err(_) => json!({"panic":true}),
    }
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let path = std::env::args().nth(1).expect("request.json");
    let v = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    println!("{}", evaluate(v));
}
#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_pinned_svg_calls() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-api.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 51);
        for c in cases {
            assert_eq!(
                super::evaluate(c["request"].clone()),
                c["got"],
                "{}",
                c["name"]
            );
        }
    }
}
