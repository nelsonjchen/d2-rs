// SPDX-License-Identifier: MPL-2.0
//! Test-only replay of original svg_fragments/precision/polygon_precision calls.
//! Kept inside svg_visibility to exercise its private index without a public API.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::*;
use serde_json::{Value, json};

fn points(value: &Value) -> Vec<SvgPoint> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|p| SvgPoint {
            x: p[0].as_f64().unwrap(),
            y: p[1].as_f64().unwrap(),
            z: p[2].as_f64().unwrap(),
        })
        .collect()
}
fn values(points: &[SvgPoint]) -> Value {
    json!(points.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>())
}
fn box_values(b: SvgBox) -> [f64; 4] {
    [b.min_x, b.min_y, b.max_x, b.max_y]
}
fn snapshot(s: &FragmentSet, budget: &Budget, error: String) -> Value {
    json!({"error":error,
        "fragments":s.fragments.iter().map(|(id,(p,b))| json!({"id":id,"points":values(p),"box":box_values(*b)})).collect::<Vec<_>>(),
        "cells":s.cells.iter().map(|c|c.iter().copied().collect::<Vec<_>>()).collect::<Vec<_>>(),
        "large":s.large.iter().copied().collect::<Vec<_>>(), "next":s.next,"vertices":s.vertices,
        "references":s.references,"work":budget.work,"nx":s.nx,"ny":s.ny,"bounds":box_values(s.bounds)})
}
fn replay(cases: &Value) -> Value {
    json!(cases.as_array().unwrap().iter().map(|case| {
        let mut budget = Budget {
            context: super::super::context::RenderContext::default(),
            limits: SVG_DEFAULT_VISIBILITY_LIMITS, work:0, face:0, faces:0,candidates:0,fragments:0,
        };
        let mut fragments: Option<FragmentSet> = None;
        case["operations"].as_array().unwrap().iter().map(|operation| {
            let op = &operation["request"];
            let p = op.get("points").map(points).unwrap_or_default();
            match op["op"].as_str().unwrap() {
                "new" => {
                    budget.limits.grid_references = op["grid_references"].as_u64().unwrap() as usize;
                    fragments = Some(FragmentSet::new(&p, &mut budget,
                        op["max_fragments"].as_u64().unwrap() as usize,
                        op["max_vertices"].as_u64().unwrap() as usize).unwrap());
                    snapshot(fragments.as_ref().unwrap(), &budget, String::new())
                }
                "add" => {
                    let s = fragments.as_mut().unwrap();
                    let error = s.add(p, &mut budget).err().unwrap_or_default();
                    snapshot(s, &budget, error)
                }
                "remove" => {
                    let s = fragments.as_mut().unwrap();
                    let id = op["id"].as_u64().unwrap() as usize;
                    let (p,b) = s.fragments[&id].clone();
                    s.change_cells(id, &b, false, &mut budget).unwrap();
                    s.fragments.remove(&id);
                    s.vertices -= p.len();
                    snapshot(s, &budget, String::new())
                }
                "path" => json!({"path":super::super::svg::svg_polygon_path(std::slice::from_ref(&p)),"source":values(&p)}),
                "edge" => json!(format!("{:016x}",edge_distance(p[0],p[1])(p[2]).to_bits())),
                "area" => json!(format!("{:016x}",polygon_area(&p).to_bits())),
                "depth" => json!(format!("{:016x}",DepthPlane {origin:p[0],dx:op["dx"].as_f64().unwrap(),dy:op["dy"].as_f64().unwrap()}.at(p[1]).to_bits())),
                "split_y" => {
                    let (inside,outside) = split_polygon(&p, &|p|p.y);
                    json!({"inside":values(&inside.resolve(&p).unwrap_or_default()),
                        "outside":values(&outside.resolve(&p).unwrap_or_default()),"source":values(&p)})
                }
                unknown => panic!("unknown operation {unknown}"),
            }
        }).collect::<Vec<_>>()
    }).collect::<Vec<_>>())
}
fn assert_same(expected: &Value, actual: &Value) {
    match (expected, actual) {
        (Value::Number(a), Value::Number(b)) => assert_eq!(a.as_f64(), b.as_f64()),
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert_same(a, b);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len());
            for (key, a) in a {
                assert_same(a, b.get(key).unwrap());
            }
        }
        _ => assert_eq!(expected, actual),
    }
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var_os("D2RS_PRECISION_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|| {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-precision-api.json").to_owned()
        });
    let request: Value = serde_json::from_str(&source).unwrap();
    let actual = replay(&request["cases"]);
    if let Some(path) = std::env::var_os("D2RS_PRECISION_OUTPUT") {
        std::fs::write(path, serde_json::to_vec(&actual).unwrap()).unwrap();
    } else {
        assert_eq!(request["cases"].as_array().unwrap().len(), 6);
        let expected = json!(
            request["cases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    c["operations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|o| o["got"].clone())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        );
        assert_same(&expected, &actual);
    }
}
