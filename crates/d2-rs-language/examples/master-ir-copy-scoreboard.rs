// SPDX-License-Identifier: MPL-2.0
//! Replay original post-Compile reservation sessions and shallow node costs.
#[path = "support/ir_json.rs"]
mod ir_projection;
use d2_rs_core::{ast, ir};
use d2_rs_language::{compiler, parse};
use serde_json::{Value, json};

fn scalar(value: &Value) -> ast::ScalarBox {
    let child = if value.get("raw").is_some() {
        assert!(
            value["value"].is_null(),
            "non-nil number requires a rational decoder"
        );
        ast::Child::Number(ast::Shared::new(ast::Number {
            raw: value["raw"].as_str().unwrap().into(),
            ..Default::default()
        }))
    } else {
        let slots = value["value"]
            .as_array()
            .unwrap()
            .iter()
            .map(|slot| {
                assert!(
                    slot.get("substitution").is_none(),
                    "substitution decoder required"
                );
                ast::InterpolationBox {
                    string: slot["string"].as_str().map(|s| ast::Shared::new(s.into())),
                    raw_string: slot["raw_string"]
                        .as_str()
                        .map(|s| ast::Shared::new(s.into())),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        ast::Child::Unquoted(ast::Shared::new(ast::UnquotedString {
            value: Some(slots.into()),
            pattern: value["pattern"].as_array().map(|parts| {
                parts
                    .iter()
                    .map(|p| p.as_str().unwrap().into())
                    .collect::<Vec<_>>()
                    .into()
            }),
            ..Default::default()
        }))
    };
    ast::ScalarBox::from_child(Some(child))
}
fn node_cost(request: &Value) -> Value {
    let mut c = compiler::Compiler::default();
    let node = &request["node"];
    let empty = c.arena.nil_scalar();
    let node = if node.get("fields").is_some() {
        ir::Node::Map(ir::Map {
            fields: Some(vec![empty; node["fields"].as_array().unwrap().len()]),
            edges: Some(vec![empty; node["edges"].as_array().unwrap().len()]),
            ..Default::default()
        })
    } else if node.get("values").is_some() {
        ir::Node::Array(ir::Array {
            values: Some(vec![empty; node["values"].as_array().unwrap().len()]),
        })
    } else if node.get("name").is_some() {
        ir::Node::Field(ir::Field {
            name: ast::StringBox {
                unquoted_string: scalar(&node["name"]).unquoted_string,
                ..Default::default()
            },
            references: Some(
                (0..request["references"].as_u64().unwrap())
                    .map(|_| ast::Shared::new(ir::FieldReference::default()))
                    .collect(),
            ),
            ..Default::default()
        })
    } else if let Some(id) = node.get("edge_id") {
        let path = |key: &str| {
            Some(
                id[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|part| {
                        assert!(part.is_null(), "non-nil endpoint decoder required");
                        None
                    })
                    .collect(),
            )
        };
        ir::Node::Edge(ir::Edge {
            id: Some(ast::Shared::new(ir::EdgeId {
                src_path: path("src_path"),
                dst_path: path("dst_path"),
                ..Default::default()
            })),
            references: Some(
                (0..request["references"].as_u64().unwrap())
                    .map(|_| ast::Shared::new(ir::EdgeReference::default()))
                    .collect(),
            ),
            ..Default::default()
        })
    } else {
        ir::Node::Scalar(ir::Scalar {
            value: scalar(&node["value"]),
        })
    };
    let id = c.arena.insert(None, node);
    json!({"units":c.variable_expansion_node_units(id)})
}
fn snapshot(c: &compiler::Compiler, root: ir::NodeId, imports: Value, error: &str) -> Value {
    let (limit, used) = c.variable_expansion_usage();
    json!({"ir":ir_projection::ir_json(&c.arena,root),"imports":imports,"error":error,"error_kind":"","limit":limit,"used":used})
}
pub fn evaluate(request: Value) -> Value {
    if request["op"] == "node_units" {
        return node_cost(&request);
    }
    assert_eq!(request["op"], "compile");
    let parsed = parse(
        request["path"].as_str().unwrap(),
        request["input"].as_str().unwrap().as_bytes(),
        None,
    );
    assert!(parsed.error.is_none());
    let files = request["files"].clone();
    let opener = (request["has_fs"] == true).then(|| {
        Box::new(move |path: &str| {
            files[path]
                .as_str()
                .map(|text| {
                    Box::new(std::io::Cursor::new(text.as_bytes().to_vec()))
                        as Box<dyn d2_rs_language::stream::ByteReader>
                })
                .ok_or_else(|| format!("open {path}: file does not exist"))
        }) as compiler::ImportOpener
    });
    let (c, root) = compiler::compile_ir_with_limits(
        ast::Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: opener,
            ..Default::default()
        },
        compiler::CompileLimits {
            max_variable_expansion: request["max_variable_expansion"].as_i64().unwrap(),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    let imports = if c.imports.is_empty() {
        Value::Null
    } else {
        json!(c.imports)
    };
    let mut results = vec![snapshot(&c, root, imports, "")];
    for copy in request["copies"].as_array().unwrap() {
        assert_eq!(copy["context"], "background");
        let result = c.reserve_variable_expansion_copy(root, None);
        let error = result.err().map(|e| e.to_string()).unwrap_or_default();
        results.push(snapshot(&c, root, Value::Null, &error));
    }
    json!({"results":results})
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).ok_or("request path")?,
    )?)?;
    println!("{}", evaluate(request));
    Ok(())
}
