// SPDX-License-Identifier: MPL-2.0
//! Replay original low-level raster sequences. This adapter stores the captured
//! static mesh; each native call owns its complete returned frame.
use base64::{Engine, engine::general_purpose::STANDARD};
use d2_rs_core::CancellationToken;
use d2_rs_isometric::img::{context::RenderContext, mesh::*, raster};
use d2_rs_scene::Nrgba;
use serde_json::{Value, json};
use std::{collections::BTreeMap, rc::Rc};

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn vector(v: &Value) -> Vec {
    nv(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn color(v: &Value) -> Nrgba {
    Nrgba::new(
        v[0].as_u64().unwrap() as u8,
        v[1].as_u64().unwrap() as u8,
        v[2].as_u64().unwrap() as u8,
        v[3].as_u64().unwrap() as u8,
    )
}
fn triangles(v: &Value) -> std::vec::Vec<Triangle> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let vertices = t["v"].as_array().unwrap();
            let v = std::array::from_fn(|i| Vertex {
                position: vector(&vertices[i]["p"]),
                normal: vector(&vertices[i]["n"]),
                u: number(&vertices[i]["u"]),
                v: number(&vertices[i]["v"]),
            });
            let m = &t["material"];
            let paint = (!m.is_null()).then(|| {
                let tex = &m["texture"];
                let texture = (!tex.is_null()).then(|| {
                    assert_eq!(
                        tex["stride"].as_u64().unwrap(),
                        tex["width"].as_u64().unwrap() * 4
                    );
                    Rc::new(Texture::new(
                        tex["width"].as_u64().unwrap() as usize,
                        tex["height"].as_u64().unwrap() as usize,
                        STANDARD.decode(tex["pixels"].as_str().unwrap()).unwrap(),
                    ))
                });
                material(Material {
                    color: color(&m["color"]),
                    roughness: number(&m["roughness"]),
                    metalness: number(&m["metalness"]),
                    emissive: color(&m["emissive"]),
                    unlit: m["unlit"].as_bool().unwrap(),
                    multiply: m["multiply"].as_bool().unwrap(),
                    texture,
                    ..Default::default()
                })
            });
            Triangle {
                v,
                material: paint,
                cast_shadow: t["cast_shadow"].as_bool().unwrap(),
                depth_bias: number(&t["depth_bias"]),
                no_depth_write: t["no_depth_write"].as_bool().unwrap(),
                ..Default::default()
            }
        })
        .collect()
}
struct Static {
    width: i64,
    height: i64,
    triangles: std::vec::Vec<Triangle>,
    background: Nrgba,
}
fn replay(case: &Value) -> Value {
    let mut rasters = BTreeMap::<u64, Static>::new();
    let mut results = vec![];
    for op in case["operations"].as_array().unwrap() {
        let id = op["id"].as_u64().unwrap();
        let input = triangles(&op["triangles"]);
        let is_new = op["op"] == "new";
        if is_new {
            assert!(!rasters.contains_key(&id));
            rasters.insert(
                id,
                Static {
                    width: op["width"].as_i64().unwrap(),
                    height: op["height"].as_i64().unwrap(),
                    triangles: input.clone(),
                    background: color(&op["background"]),
                },
            );
        } else {
            assert_eq!(op["op"], "frame");
        }
        let s = &rasters[&id];
        let token = CancellationToken::new();
        if op["canceled"].as_bool().unwrap() {
            token.cancel();
        }
        let dynamic = if is_new { &[][..] } else { &input };
        let got = raster::render_mesh_with_dynamic(
            s.width,
            s.height,
            &s.triangles,
            dynamic,
            s.background,
            &RenderContext::from_token(&token),
        );
        results.push(match got {
            Ok(frame) => json!({"frame": {"width": frame.rect.dx(), "height": frame.rect.dy(), "stride": frame.stride, "pixels": STANDARD.encode(frame.pix)}, "error": ""}),
            Err(error) => json!({"frame": null, "error": error}),
        });
    }
    json!(results)
}
fn main() {
    let request: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let results: std::vec::Vec<_> = request["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(replay)
        .collect();
    println!("{}", json!(results));
}
