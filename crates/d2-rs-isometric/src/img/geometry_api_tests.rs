// SPDX-License-Identifier: MPL-2.0
//! Test-only replay of original header and bridge calls.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::{
    context::RenderContext, geometry::LabelSurface, hierarchy_headers::*, mesh::*,
    route_bridges::*, svg_visibility::SvgPoint,
};
use crate::scene::{Board, Edge, Node, NodeMetadata};
use d2_rs_core::CancellationToken;
use serde_json::{Value, json};

fn number(v: &Value) -> f64 {
    match v.as_str() {
        Some("NaN") => f64::NAN,
        Some("+Inf") => f64::INFINITY,
        Some("-Inf") => f64::NEG_INFINITY,
        _ => v.as_f64().unwrap(),
    }
}
fn vec(v: &Value) -> Vec {
    Vec {
        x: number(&v["x"]),
        y: number(&v["y"]),
        z: number(&v["z"]),
    }
}
fn points(v: &Value) -> std::vec::Vec<SvgPoint> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| SvgPoint {
            x: number(&p[0]),
            y: number(&p[1]),
            z: number(&p[2]),
        })
        .collect()
}
fn surface(v: &Value) -> LabelSurface {
    LabelSurface {
        center: vec(&v["center"]),
        width: number(&v["width"]),
        depth: number(&v["depth"]),
        angle: number(&v["angle"]),
        align: v["align"].as_str().unwrap().into(),
    }
}
fn surface_value(s: &LabelSurface) -> Value {
    json!({"center":s.center,"width":s.width,"depth":s.depth,"angle":s.angle,"align":s.align})
}
pub(super) fn node(v: &Value) -> Node {
    Node {
        id: v["id"].as_str().unwrap().into(),
        label: v["label"].as_str().unwrap().into(),
        type_: v["type"].as_str().unwrap().into(),
        board_id: v["boardId"].as_str().unwrap().into(),
        parent_id: v["parentId"].as_str().unwrap_or_default().into(),
        container: v["container"].as_bool().unwrap(),
        sequence_role: v["sequenceRole"].as_str().unwrap_or_default().into(),
        position: vec(&v["position"]),
        size: vec(&v["size"]),
        fill: v["fill"].as_str().unwrap().into(),
        fill_explicit: v["fillExplicit"].as_bool().unwrap(),
        stroke: v["stroke"].as_str().unwrap().into(),
        stroke_explicit: v["strokeExplicit"].as_bool().unwrap(),
        font_color: v["fontColor"].as_str().unwrap().into(),
        font_explicit: v["fontExplicit"].as_bool().unwrap(),
        opacity: number(&v["opacity"]),
        stroke_width: v["strokeWidth"].as_i64().unwrap(),
        stroke_dash: number(&v["strokeDash"]),
        tooltip: v["tooltip"].as_str().unwrap_or_default().into(),
        link: v["link"].as_str().unwrap_or_default().into(),
        icon: v["icon"].as_str().unwrap_or_default().into(),
        metadata: NodeMetadata {
            original: serde_json::from_value(v["metadata"]["original"].clone()).unwrap(),
        },
    }
}
pub(super) fn board(v: &Value) -> Board {
    Board {
        id: v["id"].as_str().unwrap().into(),
        source_id: v["sourceId"].as_str().unwrap_or_default().into(),
        label: v["label"].as_str().unwrap().into(),
        parent_id: v["parentId"].as_str().unwrap_or_default().into(),
        kind: v["kind"].as_str().unwrap().into(),
        level: v["level"].as_i64().unwrap(),
        position: vec(&v["position"]),
        size: vec(&v["size"]),
        node_ids: serde_json::from_value(v["nodeIds"].clone()).unwrap_or_default(),
        header_depth: number(&v["headerDepth"]),
    }
}

fn replay(op: &Value) -> Value {
    match op["op"].as_str().unwrap() {
        "header" => {
            let v = &op["board"];
            let board = board(v);
            let owner = node(&op["owner"]);
            let nodes: std::vec::Vec<_> = op["nodes"]
                .as_array()
                .map(|a| a.iter().map(node).collect())
                .unwrap_or_default();
            let (got, fits) = hierarchy_board_header_placement(
                surface(&op["surface"]),
                &board,
                &owner,
                &nodes,
                number(&op["scale"]),
                None,
            );
            let mut board_value = json!(board);
            // Preserve the Go slice boundary independently of the native borrow.
            if v["nodeIds"].is_null() && board.node_ids.is_empty() {
                board_value["nodeIds"] = Value::Null;
            }
            let node_values = if op["nodes"].is_null() && nodes.is_empty() {
                Value::Null
            } else {
                json!(nodes)
            };
            json!({"surface":surface_value(&got),"fits":if op["placement"]==true {json!(fits)} else {Value::Null},"board":board_value,"owner":owner,"nodes":node_values})
        }
        "bridge" | "lane" | "pipeline" => {
            let edges: std::vec::Vec<_> = op["edges"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| Edge {
                    id: e["id"].as_str().unwrap().into(),
                    source: e["source"].as_str().unwrap().into(),
                    target: e["target"].as_str().unwrap().into(),
                    stroke: e["stroke"].as_str().unwrap().into(),
                    stroke_explicit: e["strokeExplicit"].as_bool().unwrap(),
                    stroke_width: e["strokeWidth"].as_i64().unwrap(),
                    opacity: number(&e["opacity"]),
                    points: Some(
                        points(&e["points"])
                            .iter()
                            .map(|p| nv(p.x, p.y, p.z))
                            .collect(),
                    ),
                    label: e["label"].as_str().unwrap().into(),
                    source_label: serde_json::from_value(e["sourceLabel"].clone()).unwrap(),
                    target_label: serde_json::from_value(e["targetLabel"].clone()).unwrap(),
                    source_arrow: e["sourceArrow"].as_str().unwrap().into(),
                    target_arrow: e["targetArrow"].as_str().unwrap().into(),
                    font_color: e["fontColor"].as_str().unwrap().into(),
                    font_explicit: e["fontExplicit"].as_bool().unwrap(),
                    stroke_dash: number(&e["strokeDash"]),
                    link: e["link"].as_str().unwrap_or_default().into(),
                    tooltip: e["tooltip"].as_str().unwrap_or_default().into(),
                    icon: e["icon"].as_str().unwrap_or_default().into(),
                    sequence_role: e["sequenceRole"].as_str().unwrap_or_default().into(),
                    metadata: crate::scene::EdgeMetadata {
                        original: serde_json::from_value(e["metadata"]["original"].clone())
                            .unwrap(),
                    },
                })
                .collect();
            let token = CancellationToken::new();
            if op["context"] == "canceled" {
                token.cancel();
            }
            let context = RenderContext::from_token(&token);
            let supplied = (op["context"] != "nil").then_some(&context);
            let nodes = op["nodes"]
                .as_array()
                .map(|v| v.iter().map(node).collect::<std::vec::Vec<_>>())
                .unwrap_or_default();
            let boards = op["boards"]
                .as_array()
                .map(|v| v.iter().map(board).collect::<std::vec::Vec<_>>())
                .unwrap_or_default();
            let result = match op["op"].as_str().unwrap() {
                "lane" => super::route_lanes::native_resolve_lane_routes(
                    supplied,
                    &edges,
                    &nodes,
                    &boards,
                    op["limit"].as_u64().unwrap() as usize,
                ),
                "pipeline" => {
                    super::routes::native_edge_routes(&edges, &[], &[], &Default::default())
                        .map(|(_, p)| p)
                }
                _ => native_resolve_bridge_routes(
                    supplied,
                    &edges,
                    op["limit"].as_u64().unwrap() as usize,
                ),
            };
            let (paths, error) = match result {
                Ok(v) => (json!(v), String::new()),
                Err(e) => (Value::Null, e),
            };
            let source = json!(
                edges
                    .iter()
                    .map(|e| {
                        let mut value = json!(e);
                        value["points"] = json!(
                            e.points
                                .as_deref()
                                .unwrap()
                                .iter()
                                .map(|p| [float_value(p.x), float_value(p.y), float_value(p.z)])
                                .collect::<std::vec::Vec<_>>()
                        );
                        value
                    })
                    .collect::<std::vec::Vec<_>>()
            );
            let mut result = json!({"paths":paths,"error":error,"source":source});
            match op["op"].as_str().unwrap() {
                "lane" => {
                    result["nodes"] = if op["nodes"].is_null() {
                        Value::Null
                    } else {
                        json!(nodes)
                    };
                    result["boards"] = if op["boards"].is_null() {
                        Value::Null
                    } else {
                        json!(
                            boards
                                .iter()
                                .zip(op["boards"].as_array().unwrap())
                                .map(|(b, original)| {
                                    let mut value = json!(b);
                                    if original["nodeIds"].is_null() {
                                        value["nodeIds"] = Value::Null;
                                    }
                                    value
                                })
                                .collect::<std::vec::Vec<_>>()
                        )
                    };
                }
                "pipeline" => {
                    let lengths = paths
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| {
                            let points: std::vec::Vec<Vec> =
                                p.as_array().unwrap().iter().map(vec).collect();
                            super::geometry_routes::route_lengths(&points)
                        })
                        .collect::<std::vec::Vec<_>>();
                    result["lengths"] = json!(lengths);
                }
                _ => {}
            }
            result
        }
        unknown => panic!("unknown {unknown}"),
    }
}
fn float_value(v: f64) -> Value {
    if v.is_nan() {
        json!("NaN")
    } else if v == f64::INFINITY {
        json!("+Inf")
    } else if v == f64::NEG_INFINITY {
        json!("-Inf")
    } else {
        json!(v)
    }
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var_os("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|| d2_rs_testdata::oracle_fixture!("tests/fixtures/geometry-api.json").into());
    let request: Value = serde_json::from_str(&source).unwrap();
    let actual = json!(
        request["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| case["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|op| replay(&op["request"]))
                .collect::<std::vec::Vec<_>>())
            .collect::<std::vec::Vec<_>>()
    );
    if let Some(path) = std::env::var_os("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(path, serde_json::to_vec(&actual).unwrap()).unwrap();
    } else {
        let expected = json!(
            request["cases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|case| case["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|op| op["got"].clone())
                    .collect::<std::vec::Vec<_>>())
                .collect::<std::vec::Vec<_>>()
        );
        assert_same(&expected, &actual);
    }
}
pub(super) fn assert_same(a: &Value, b: &Value) {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => assert_eq!(a.as_f64(), b.as_f64()),
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert_same(a, b)
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len());
            for (k, a) in a {
                assert_same(a, b.get(k).unwrap())
            }
        }
        _ => assert_eq!(a, b),
    }
}

#[test]
fn edge_pipeline_keeps_the_callers_context() {
    let token = CancellationToken::new();
    token.cancel();
    let error = super::routes::native_edge_routes_with_context(
        &[],
        &[],
        &[],
        &Default::default(),
        &RenderContext::from_token(&token),
    )
    .unwrap_err();
    assert_eq!(error, "context canceled");
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_lane_contracts() {
    let request: Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/lanes-api.json")).unwrap();
    for case in request["cases"].as_array().unwrap() {
        for op in case["operations"].as_array().unwrap() {
            assert_same(&op["got"], &replay(&op["request"]));
        }
    }
}
#[test]
fn lanes_check_cancellation_after_start_and_before_invalid_pipeline_input() {
    let token = CancellationToken::new();
    let context = RenderContext::from_token(&token);
    let mut work = super::route_bridges::Work::with_stage_context(1, "route lanes", &context);
    work.spend(1).unwrap();
    token.cancel();
    assert_eq!(work.spend(1).unwrap_err(), "context canceled");
    let edge = Edge {
        id: "invalid".into(),
        points: Some(vec![nv(f64::NAN, 0.0, 0.0)]),
        ..Default::default()
    };
    let error = super::routes::native_edge_routes_with_context(
        &[edge],
        &[],
        &[],
        &Default::default(),
        &context,
    )
    .unwrap_err();
    assert_eq!(error, "context canceled");
}
