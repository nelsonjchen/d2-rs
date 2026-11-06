// SPDX-License-Identifier: MPL-2.0
//! Test-only native adapter for original master internal IR assertions.
//! Raw cancellation states are retained; the original tests assert partial
//! progress and existence of a cancellation point, not equal poll schedules.
use super::*;
use crate::parse;
use d2_rs_core::ast::Node as _;
use d2_rs_core::{LayoutError, ast};
use serde_json::{Value, json};
use std::{cell::Cell, rc::Rc};
#[path = "../../examples/support/ir_json.rs"]
mod projection;

fn budget(request: &Value) -> Value {
    let kind = request["kind"].as_str().unwrap();
    let limit = request["limit"].as_i64().unwrap();
    let mut limits = CompileLimits::default();
    match kind {
        "variable" => limits.max_variable_expansion = limit,
        "glob" => limits.max_glob_expansion = limit,
        "edge" => limits.max_edge_expansion = limit,
        "work" => limits.max_edge_expansion_work = limit,
        _ => panic!("unknown budget"),
    }
    // Use the production constructor/validation boundary, including default
    // selection and negative-limit rejection; no constructor clone in tests.
    let (mut c, _) = compile_ir_with_limits(
        Shared::new(parse("", b"", None).map),
        CompileOptions::default(),
        limits,
    )
    .unwrap();
    if let Some(error) = c.errors.first() {
        return json!({"error":error.to_string(),"results":[],"state":null});
    }
    let mut results = Vec::new();
    for op in request["operations"].as_array().unwrap() {
        for _ in 0..op["count"].as_u64().unwrap() {
            let units = op["units"].as_u64().unwrap();
            let result = match kind {
                "variable" => c
                    .variable_expansion
                    .reserve(units)
                    .map_err(|e| e.to_string()),
                "glob" if op["op"] == "field" => c.glob_expansion.reserve_field(),
                "glob" => c.glob_expansion.reserve(units),
                "edge" => c.edge_expansion.reserve_pair(),
                "work" => c.edge_expansion.reserve_work(),
                _ => unreachable!(),
            };
            results.push(result.err().unwrap_or_default());
        }
    }
    let (limit, used, created) = match kind {
        "variable" => (
            c.variable_expansion.limit.get(),
            c.variable_expansion.used.get(),
            0,
        ),
        "glob" => {
            let (used, fields) = c.glob_expansion.usage();
            (c.glob_expansion.limit, used, fields)
        }
        "edge" => (c.edge_expansion.limit, c.edge_expansion.usage().0, 0),
        "work" => (c.edge_expansion.work_limit, c.edge_expansion.usage().1, 0),
        _ => unreachable!(),
    };
    json!({"error":"","results":results,"state":{"limit":limit,"used":used,"created_fields":created}})
}

fn context(cancel_at: u64, initial: u64) -> (CompileContext, Rc<Cell<u64>>) {
    let calls = Rc::new(Cell::new(initial));
    let counter = calls.clone();
    (
        Box::new(move || {
            counter.set(counter.get() + 1);
            (counter.get() >= cancel_at).then_some(LayoutError::Cancelled)
        }),
        calls,
    )
}

fn evaluate(request: &Value) -> Value {
    match request["op"].as_str().unwrap() {
        "grid" => {
            let result = d2_rs_core::grid::grid_capacity(
                request["rows"].as_i64().unwrap(),
                request["columns"].as_i64().unwrap(),
            );
            let got = match result {
                Ok(capacity) => json!({"capacity":capacity,"error":""}),
                Err(error) => json!({"capacity":0,"error":error}),
            };
            json!({"got":got,"observations":null})
        }
        "safe" => {
            json!({"got":d2_rs_text::markdown::link_safety::safe_markdown_link(request["link"].as_str().unwrap()),"observations":null})
        }
        "danger" => {
            json!({"got":d2_rs_text::markdown::link_safety::is_dangerous_link(request["link"].as_str().unwrap().as_bytes()),"observations":null})
        }
        "ensure" => {
            let mut c = Compiler::default();
            let root = c.arena.insert(None, Node::Map(Map::default()));
            let path =
                crate::parse::parse_key(request["key"].as_str().unwrap().as_bytes()).unwrap();
            let result = c.construct_fields(
                root,
                0,
                &FieldQuery {
                    path: Shared::new(path),
                    context: None,
                    create: request["create"].as_bool().unwrap(),
                    glob: None,
                    indexed: false,
                },
            );
            json!({"got":{"fields":result.fields.iter().map(|id| projection::ir_json(&c.arena,*id)).collect::<Vec<_>>(),"error":result.error.map(|e|e.to_string()).unwrap_or_default()},"observations":null})
        }
        "contains" => {
            let mut c = Compiler::default();
            let nodes = request["nodes"].as_array().unwrap();
            let ids: Vec<_> = nodes
                .iter()
                .map(|n| {
                    c.arena.insert(
                        None,
                        match n["kind"].as_str().unwrap() {
                            "map" => Node::Map(Map::default()),
                            "field" => Node::Field(Field::default()),
                            _ => unreachable!(),
                        },
                    )
                })
                .collect();
            for (i, n) in nodes.iter().enumerate() {
                match c.arena.node_mut(ids[i]) {
                    Node::Map(m) => {
                        m.fields = Some(
                            n["children"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|v| ids[v.as_u64().unwrap() as usize])
                                .collect(),
                        )
                    }
                    Node::Field(f) => {
                        f.composite = n["composite"].as_u64().map(|v| ids[v as usize])
                    }
                    _ => unreachable!(),
                }
            }
            json!({"got":c.composite_contains(ids[request["root"].as_u64().unwrap() as usize],ids[request["target"].as_u64().unwrap() as usize]),"observations":null})
        }
        "budget" => json!({"got":budget(request),"observations":null}),
        "nil_reserve" => {
            let c = Compiler::default();
            let result = if request["copy"] == true {
                c.reserve_variable_expansion_copy(None, None)
            } else {
                c.reserve_variable_expansion_aliases(None, None)
            };
            json!({"got":{"error":result.unwrap_err().to_string()},"observations":null})
        }
        "array_cancel" => {
            let parsed = parse(
                request["path"].as_str().unwrap(),
                request["input"].as_str().unwrap().as_bytes(),
                None,
            );
            assert!(parsed.error.is_none());
            let scope = Shared::new(parsed.map);
            let ast::Child::Key(key) = scope.read().children().remove(0) else {
                panic!("array field")
            };
            let array = key.read().value.array.clone().unwrap();
            let source_values = array.read().children().len();
            let (context, calls) = context(
                request["cancel_at"].as_u64().unwrap(),
                request["initial_calls"].as_u64().unwrap(),
            );
            let mut c = Compiler {
                context: Some(context),
                ..Default::default()
            };
            let dst = c.arena.insert(None, Node::Array(Array::default()));
            c.compile_array(dst, array, scope.clone());
            let Node::Array(a) = c.arena.node(dst) else {
                unreachable!()
            };
            json!({"got":{"canceled":matches!(c.context_error,Some(LayoutError::Cancelled)),"incomplete":a.values.as_ref().map_or(0,Vec::len)<source_values},
                "observations":{"calls":calls.get(),"array":projection::ir_json(&c.arena,dst),"source_values":source_values,"used":c.variable_expansion.used.get()}})
        }
        "edge_cancel" => {
            let mut observations = Vec::new();
            let mut observed = false;
            for cancel_at in
                request["cancel_start"].as_u64().unwrap()..request["cancel_end"].as_u64().unwrap()
            {
                // Fresh base compilation is equivalent to the original's
                // immutable base.Copy; neither shares mutable test state.
                let parsed = parse(
                    "index-test.d2",
                    request["input"].as_str().unwrap().as_bytes(),
                    None,
                );
                assert!(parsed.error.is_none());
                let (mut c, root) = compile_ir(Shared::new(parsed.map)).unwrap();
                assert!(c.errors.is_empty());
                let key = Shared::new(
                    crate::parse::parse_map_key(request["key"].as_str().unwrap().as_bytes())
                        .unwrap(),
                );
                let id = new_edge_ids(&key.read()).unwrap().remove(0);
                let edge = key
                    .read()
                    .edges
                    .as_ref()
                    .unwrap()
                    .get(0)
                    .unwrap()
                    .read()
                    .clone();
                let refctx = Shared::new(RefContext {
                    key: Some(key),
                    edge,
                    scope_map: Some(root),
                    ..Default::default()
                });
                let (context, calls) = context(cancel_at, 0);
                c.context = Some(context);
                c.edge_expansion.limit = 100;
                // The original uses a new compiler for each copied base.
                c.glob_context_stack = vec![vec![]];
                if request["selector"] == true {
                    c.query_edges(root, &id, &refctx);
                } else {
                    c.create_edges(root, &id, &refctx);
                }
                let canceled = matches!(c.context_error, Some(LayoutError::Cancelled));
                let used = c.edge_expansion.usage().0;
                observations.push(json!({"cancel_at":cancel_at,"calls":calls.get(),"canceled":canceled,"used":used,"ir":projection::ir_json(&c.arena,root)}));
                if canceled && used > 0 && used < 9 {
                    observed = true;
                    break;
                }
            }
            json!({"got":{"observed_during_expansion":observed},"observations":observations})
        }
        op => panic!("unknown operation {op}"),
    }
}

#[test]
fn scoreboard_request() {
    let Ok(path) = std::env::var("D2RS_INTERNAL_REQUEST") else {
        return;
    };
    let request = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let output = std::env::var("D2RS_INTERNAL_RESULT").unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    serde_json::to_writer(&mut file, &evaluate(&request)).unwrap();
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_master_internal_contracts() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-ir-internal.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        assert_eq!(
            evaluate(&case["request"])["got"],
            case["got"],
            "{}",
            case["name"]
        );
    }
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_master_small_contracts() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/master-small.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        assert_eq!(
            evaluate(&case["request"])["got"],
            case["got"],
            "{}",
            case["name"]
        );
    }
}

#[test]
fn grid_admission_checks_positive_axes_before_limits() {
    for (rows, columns) in [(0, 1), (1, 0), (-1, i64::MAX), (i64::MIN, 0)] {
        assert_eq!(
            d2_rs_core::grid::grid_capacity(rows, columns),
            Err(format!(
                "grid dimensions must be positive, got {rows} by {columns}"
            ))
        );
    }
}

#[test]
fn safe_links_trim_without_decoding_returned_metadata() {
    use d2_rs_text::markdown::link_safety::safe_markdown_link;
    assert_eq!(
        safe_markdown_link("\u{2003}https://example.com/%20?a=1&amp;b=2\n"),
        "https://example.com/%20?a=1&amp;b=2"
    );
    assert_eq!(safe_markdown_link(" \t\n"), "");
    assert_eq!(
        safe_markdown_link("\u{2003}javascript:alert(1)\u{2003}"),
        ""
    );
}
