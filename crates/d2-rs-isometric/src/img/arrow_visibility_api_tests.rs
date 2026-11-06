// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of original endpoint-visibility observations.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::*;
use serde_json::{Value, json};
use std::rc::Rc;
fn num(v: &Value) -> f64 {
    match v.as_str() {
        Some("+Inf") => f64::INFINITY,
        Some("-Inf") => f64::NEG_INFINITY,
        _ => v.as_f64().unwrap(),
    }
}
fn vector(v: &Value) -> Vec {
    nv(num(&v["x"]), num(&v["y"]), num(&v["z"]))
}
fn vectors(v: &Value) -> std::vec::Vec<Vec> {
    v.as_array().unwrap().iter().map(vector).collect()
}
fn point(v: &Value) -> RouteCaptionPoint {
    RouteCaptionPoint {
        x: num(&v["x"]),
        z: num(&v["z"]),
    }
}
fn triangle(v: &Value) -> Triangle {
    let pts = vectors(&v["points"]);
    let m = &v["material"];
    Triangle {
        v: std::array::from_fn(|i| Vertex {
            position: pts[i],
            ..Default::default()
        }),
        depth_bias: num(&v["bias"]),
        no_depth_write: v["noDepth"] == true,
        svg_coverage_only: v["coverage"] == true,
        material: (!m.is_null()).then(|| {
            material(Material {
                color: serde_json::from_value(m["color"].clone()).unwrap(),
                texture: (m["texture"] == true).then(|| Rc::new(Texture::default())),
                unlit: m["unlit"] == true,
                ..Default::default()
            })
        }),
        ..Default::default()
    }
}
fn triangles(v: &Value) -> std::vec::Vec<Triangle> {
    v.as_array().unwrap().iter().map(triangle).collect()
}
fn builder(v: &Value) -> MeshBuilder {
    let mut b = MeshBuilder {
        scale: num(&v["scale"]),
        arrow_background: v["background"].as_str().unwrap().into(),
        arrow_work: v["work"].as_u64().unwrap() as usize,
        arrow_cache_points: v["cachePoints"].as_u64().unwrap() as usize,
        ..Default::default()
    };
    b.triangles = triangles(&v["triangles"]);
    if v["error"] != "" {
        b.err = Some(v["error"].as_str().unwrap().into())
    }
    let token = d2_rs_core::CancellationToken::new();
    if v["canceled"] == true {
        token.cancel()
    };
    b.context = super::super::context::RenderContext::from_token(&token);
    for (id, o) in v["owners"].as_object().unwrap() {
        let mut obstacles = HashMap::new();
        for item in o["obstacles"].as_array().unwrap() {
            let polygons = item["polygons"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| Obstacle {
                    points: p["points"].as_array().unwrap().iter().map(point).collect(),
                    lo: point(&p["lo"]),
                    hi: point(&p["hi"]),
                })
                .collect();
            obstacles.insert(num(&item["key"]).to_bits(), polygons);
        }
        b.arrow_owners.insert(
            id.clone(),
            NativeArrowOwner {
                first: o["first"].as_u64().unwrap() as usize,
                last: o["last"].as_u64().unwrap() as usize,
                node: super::super::geometry_api_tests::node(&o["node"]),
                board: (!o["board"].is_null())
                    .then(|| super::super::geometry_api_tests::board(&o["board"])),
                caps: (o["prepared"] == true).then(|| triangles(&o["caps"])),
                obstacles,
            },
        );
    }
    for m in v["markers"].as_array().unwrap() {
        b.arrow_markers.insert(
            (
                m["kind"].as_str().unwrap().into(),
                m["width"].as_i64().unwrap(),
            ),
            NativeArrowMarker {
                vertices: vectors(&m["vertices"]),
                min_y: num(&m["minY"]),
                reach: num(&m["reach"]),
            },
        );
    }
    b
}
fn counters(b: &MeshBuilder) -> Value {
    let owners:serde_json::Map<String,Value>=b.arrow_owners.iter().map(|(id,o)|{let mut keys:std::vec::Vec<_>=o.obstacles.keys().map(|k|f64::from_bits(*k)).collect();keys.sort_by(f64::total_cmp);let sizes:std::vec::Vec<_>=keys.iter().map(|k|json!([k,o.obstacles[&k.to_bits()].len()])).collect();(id.clone(),json!({"prepared":o.caps.is_some(),"caps":o.caps.as_ref().map_or(0,|v|v.len()),"obstacles":sizes}))}).collect();
    json!({"work":b.arrow_work,"cachePoints":b.arrow_cache_points,"markers":b.arrow_markers.len(),"owners":owners,"error":b.err.as_deref().unwrap_or("")})
}
fn replay(v: &Value) -> Value {
    if v["op"] == "occluder" {
        let p = occluder(&triangle(&v["triangle"]), num(&v["minimumY"]));
        return json!({"points":p.iter().map(|p|json!({"x":p.x,"z":p.z})).collect::<std::vec::Vec<_>>()});
    }
    let mut b = builder(&v["builder"]);
    let e = &v["edge"];
    let edge = Edge {
        source: e["source"].as_str().unwrap().into(),
        target: e["target"].as_str().unwrap().into(),
        source_arrow: e["sourceArrow"].as_str().unwrap().into(),
        target_arrow: e["targetArrow"].as_str().unwrap().into(),
        stroke_width: e["strokeWidth"].as_i64().unwrap(),
        ..Default::default()
    };
    let p = vectors(&v["points"]);
    let (start, end) = b.visible_arrow_range(&edge, &p);
    let route = if start == 0.0 && end == 1.0 {
        p.clone()
    } else {
        super::super::geometry_routes::native_route_section(&p, &route_lengths(&p), start, end)
    };
    json!({"route":route,"state":counters(&b),"source":p})
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|_| {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/arrow-api.json").into()
        });
    let cases: Value = serde_json::from_str(&source).unwrap();
    let out: std::vec::Vec<_> = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|op| replay(&op["request"]))
                .collect::<std::vec::Vec<_>>()
        })
        .collect();
    if let Ok(path) = std::env::var("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(path, serde_json::to_vec(&out).unwrap()).unwrap();
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
                    .map(|op| op["got"].clone())
                    .collect::<std::vec::Vec<_>>()
            })
            .collect();
        super::super::geometry_api_tests::assert_same(&json!(expected), &json!(out));
    }
}

#[test]
fn cancellation_precedes_budget_and_preserves_prior_error() {
    let token = d2_rs_core::CancellationToken::new();
    let mut b = MeshBuilder::default();
    b.context = super::super::context::RenderContext::from_token(&token);
    assert!(b.arrow_budget(1));
    token.cancel();
    b.arrow_work = 100_000_000;
    assert!(!b.arrow_budget(1));
    assert_eq!(b.arrow_work, 100_000_000);
    assert_eq!(b.err.as_deref(), Some("context canceled"));
    b.err = Some("prior failure".into());
    assert!(!b.arrow_budget(1));
    assert_eq!(b.err.as_deref(), Some("prior failure"));
}
