// SPDX-License-Identifier: MPL-2.0
//! Original ink and card calls: input-only test transport.
use super::super::{
    geometry::MeshBuilder, geometry_api_tests::node, native_solids::solid_api_tests as solid,
    svg::hierarchy_api_tests as hierarchy,
};
use super::*;
use serde_json::{Value, json};
use std::vec::Vec as List;

fn segments(v: &Value) -> List<ClassicInkSegment> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| ClassicInkSegment {
            a: vector(&s["a"]),
            c: vector(&s["c"]),
            support: s["support"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    Rc::new(ClassicInkFacet {
                        points: std::array::from_fn(|i| vector(&f["points"][i])),
                        bias: f["bias"].as_f64().unwrap(),
                        keys: serde_json::from_value(f["keys"].clone()).unwrap(),
                        normal: vector(&f["normal"]),
                    })
                })
                .collect(),
        })
        .collect()
}
fn segment_values(ss: &[ClassicInkSegment]) -> Value {
    json!(ss.iter().map(|s|json!({"a":s.a,"c":s.c,"support":s.support.iter().map(|f|json!({"points":f.points,"bias":f.bias,"keys":f.keys,"normal":f.normal})).collect::<List<_>>()})).collect::<List<_>>())
}
fn vector(v: &Value) -> Vec {
    nv(
        v["x"].as_f64().unwrap(),
        v["y"].as_f64().unwrap(),
        v["z"].as_f64().unwrap(),
    )
}
pub(crate) fn builder(v: &Value) -> MeshBuilder {
    let mut b = solid::builder(&v["state"]);
    b.options.vector = v["vector"].as_bool().unwrap_or_default();
    if v["text"] == true {
        b.text = super::super::text::TextPainter::new(1, None).unwrap()
    }
    b.arrow_background = v["background"].as_str().unwrap_or_default().into();
    if !v["rich"].is_null() {
        let p = &v["rich"];
        b.rich.count = p["count"].as_i64().unwrap();
        b.rich.used = p["used"].as_i64().unwrap();
        b.rich.pixels = p["pixels"].as_i64().unwrap();
        b.rich.source_bytes = p["bytes"].as_u64().unwrap() as usize;
        b.rich.rows = p["rows"].as_u64().unwrap() as usize;
        b.rich.primary = p["primary"].as_str().unwrap().into();
        b.rich.mono = p["mono"].as_str().unwrap().into();
        b.rich.theme_id = p["theme"].as_i64();
        b.rich.theme_overrides = serde_json::from_value(p["overrides"].clone()).unwrap();
    }
    if !v["supports"].is_null() {
        b.hierarchy_supports = serde_json::from_value(v["supports"].clone()).unwrap()
    }
    b.node_support_drop = v["drop"].as_f64().unwrap_or_default();
    if !v["linkBudget"].is_null() {
        b.options.links = Some(d2_rs_scenebuild::LinkBudget {
            max_regions: v["linkBudget"]["MaxRegions"].as_i64().unwrap(),
            max_string_bytes: v["linkBudget"]["MaxStringBytes"].as_i64().unwrap(),
        })
    }
    b
}
fn card_state(b: &mut MeshBuilder, v: &Value) -> Value {
    let links = std::mem::take(&mut b.links);
    let state = solid::state(b);
    let rich = if v["rich"].is_null() {
        Value::Null
    } else {
        let p = &b.rich;
        json!({"count":p.count,"used":p.used,"pixels":p.pixels,"bytes":p.source_bytes,"rows":p.rows,"primary":p.primary,"mono":p.mono,"theme":p.theme_id,"overrides":p.theme_overrides})
    };
    let ls=json!(links.iter().map(|l|json!({"points":l.points,"region":{"url":l.url,"tooltip":l.tooltip,"target":l.target}})).collect::<List<_>>());
    b.links = links;
    json!({"builder":{"state":state,"rich":rich,"supports":if v["supports"].is_null(){Value::Null}else{json!(b.hierarchy_supports)},"drop":b.node_support_drop},"links":ls,"linkBytes":b.link_bytes})
}
pub(crate) fn replay(v: &Value, images: &std::path::Path) -> Value {
    match v["op"].as_str().unwrap() {
        "render" => return super::super::raster::api_tests::replay(v, images),
        "header" | "placement" | "render-nodes" => {
            let mut result = hierarchy::replay(v);
            // Preserve the absent source slice in the observer echo. Native
            // placement consumes the same empty slice in both representations.
            if v["nodes"].is_null() && result.get("nodes").is_some() {
                result["nodes"] = Value::Null;
            }
            return result;
        }
        "segments" => {
            let n = node(&v["node"]);
            let ts = solid::mesh_input(&v["mesh"]);
            let result = classic_ink_segments_with_context(
                &RenderContext::default(),
                &n,
                &ts,
                v["radius"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
            );
            return match result {
                Ok(ss) => json!({"segments":segment_values(&ss),"error":""}),
                Err(e) => json!({"segments":[],"error":e}),
            };
        }
        _ => {}
    }
    let mut b = builder(v);
    match v["op"].as_str().unwrap() {
        "ink" => {
            let n = node(&v["node"]);
            let supplied = solid::mesh_input(&v["mesh"]);
            let first = b.triangles.len() - supplied.len();
            super::super::geometry_api_tests::assert_same(
                &solid::mesh(&b.triangles[first..]),
                &solid::mesh(&supplied),
            );
            b.classic_ink_edges(&n, first);
            json!({"state":solid::state(&b),"node":n})
        }
        "junctions" => {
            let m = solid::mesh_input(&v["material"])[0]
                .material
                .clone()
                .unwrap();
            b.classic_ink_junctions(
                &segments(&v["segments"]),
                &segments(&v["ends"]),
                v["radius"].as_f64().unwrap(),
                &m,
            );
            solid::state(&b)
        }
        op @ ("node" | "hierarchy" | "content") => {
            let n = node(&v["node"]);
            let tint = v["tint"].as_str().unwrap();
            if op == "node" {
                b.node(&n, tint, None)
            } else if op == "hierarchy" {
                b.hierarchy_node(&n, tint)
            } else {
                b.canonical_node_content(
                    &n,
                    &serde_json::from_value(v["shape"].clone()).unwrap(),
                    v["fill"].as_str().unwrap(),
                    v["top"].as_f64().unwrap(),
                )
            }
            json!({"state":card_state(&mut b,v),"node":n})
        }
        op => panic!("unknown {op}"),
    }
}
#[test]
#[ignore = "requires an external original Go capture"]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST").unwrap();
    let output = std::path::PathBuf::from(std::env::var("D2RS_GEOMETRY_OUTPUT").unwrap());
    let images = output.parent().unwrap().join("images");
    std::fs::create_dir(&images).unwrap();
    let cases: Value = serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
    let result: List<_> = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| replay(&o["request"], &images))
                .collect::<List<_>>()
        })
        .collect();
    std::fs::write(output, serde_json::to_vec(&result).unwrap()).unwrap();
}
