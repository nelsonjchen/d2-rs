// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of original source-shape and sequence calls.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{
    geometry::LabelSurface, geometry_api_tests::node, mesh::*, native_shapes::*, text::TextPainter,
};
use super::solid_api_tests::{builder, mesh_input, replay as solid_replay, state, texture};
use crate::target::Shape;
use serde_json::{Value, json};
fn number(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn vec(v: &Value) -> Vec {
    nv(number(&v["x"]), number(&v["y"]), number(&v["z"]))
}
fn surface(s: &LabelSurface) -> Value {
    json!({"center":s.center,"width":s.width,"depth":s.depth,"angle":s.angle,"align":s.align})
}
fn replay(v: &Value) -> Value {
    let op = v["op"].as_str().unwrap();
    if op == "solid" {
        return solid_replay(v);
    }
    if op == "source" {
        return json!(native_face_source(
            &node(&v["node"]),
            v["fill"].as_str().unwrap()
        ));
    }
    if op == "label" {
        return surface(&native_node_label_surface(
            &node(&v["node"]),
            &serde_json::from_value(v["shape"].clone()).unwrap(),
            number(&v["y"]),
        ));
    }
    if op == "profile" || op == "profiles" {
        let s: Shape = serde_json::from_value(v["shape"].clone()).unwrap();
        return if op == "profile" {
            match native_shape_profile(&s, None) {
                Ok(p) => json!({"profile":p,"error":""}),
                Err(e) => json!({"profile":null,"error":e}),
            }
        } else {
            match native_shape_profiles(&s) {
                Ok(p) => json!({"profile":p,"error":""}),
                Err(e) => json!({"profile":null,"error":e}),
            }
        };
    }
    let mut b = builder(&v["state"]);
    let token = d2_rs_core::CancellationToken::new();
    if v["canceled"] == true {
        token.cancel()
    }
    b.context = super::super::context::RenderContext::from_token(&token);
    b.arrow_background = v["background"].as_str().unwrap().into();
    if v["text"] == true {
        b.text = TextPainter::new(1, None).unwrap()
    }
    if v["vector"] == true {
        b.options.vector = true;
        b.vectors = super::super::vector_surface::native_vector_context()
    }
    match op {
        "face" | "layers" => {
            let s: Shape = serde_json::from_value(v["shape"].clone()).unwrap();
            if op == "face" {
                let (t, a) = b.native_face(&s);
                return json!({"state":state(&b),"texture":texture(t.as_ref()),"area":surface(&a)});
            }
            let opacity = v["opacity"].as_array().and_then(|a| a.first()).map(number);
            let (f, i, a) = b.native_face_layers(&s, opacity);
            return json!({"state":state(&b),"fill":texture(f.as_ref()),"ink":texture(i.as_ref()),"area":surface(&a)});
        }
        "arrow" => {
            let ts = mesh_input(&v["material"]);
            let m = ts[0].material.as_ref().unwrap();
            b.arrow_with_opacity(
                v["kind"].as_str().unwrap(),
                vec(&v["tip"]),
                vec(&v["dir"]),
                m,
                v["width"].as_i64().unwrap(),
                number(&v["opacity"]),
            );
            return state(&b);
        }
        "board" => {
            let s = &v["board"];
            let board = crate::scene::Board {
                id: s["id"].as_str().unwrap().into(),
                source_id: s["sourceId"].as_str().unwrap_or_default().into(),
                parent_id: s["parentId"].as_str().unwrap_or_default().into(),
                label: s["label"].as_str().unwrap().into(),
                level: s["level"].as_i64().unwrap(),
                header_depth: number(&s["headerDepth"]),
                node_ids: s["nodeIds"]
                    .as_array()
                    .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
                    .unwrap_or_default(),
                kind: s["kind"].as_str().unwrap().into(),
                position: crate::scene::Vec3::new(
                    number(&s["position"]["x"]),
                    number(&s["position"]["y"]),
                    number(&s["position"]["z"]),
                ),
                size: crate::scene::Vec3::new(
                    number(&s["size"]["x"]),
                    number(&s["size"]["y"]),
                    number(&s["size"]["z"]),
                ),
            };
            let n = node(&v["node"]);
            b.hierarchy_board(
                &board,
                Some(&n),
                v["tint"].as_str().unwrap(),
                number(&v["opacity"]),
            );
            return state(&b);
        }
        _ => {}
    }
    let n = node(&v["node"]);
    let tint = v["tint"].as_str().unwrap();
    match op {
        "node" => b.node(&n, tint, None),
        "hierarchy" => b.hierarchy_node(&n, tint),
        "sequence" => b.sequence_node(&n, tint),
        "canonical" => b.canonical_node(&n, tint),
        _ => panic!("unknown {op}"),
    }
    json!({"state":state(&b),"node":n})
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|_| d2_rs_testdata::oracle_fixture!("tests/fixtures/shape-api.json").into());
    let cases: Value = serde_json::from_str(&source).unwrap();
    let output: std::vec::Vec<_> = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| replay(&o["request"]))
                .collect::<std::vec::Vec<_>>()
        })
        .collect();
    if let Ok(p) = std::env::var("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(p, serde_json::to_vec(&output).unwrap()).unwrap()
    } else {
        let expected: std::vec::Vec<_> = cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                c["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| o["got"].clone())
                    .collect::<std::vec::Vec<_>>()
            })
            .collect();
        super::super::geometry_api_tests::assert_same(&json!(expected), &json!(output))
    }
}
