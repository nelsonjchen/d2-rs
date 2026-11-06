// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of the original route-caption tests.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::*;
use crate::target::{Connection, Text};
use serde_json::{Value, json};

fn number(v: &Value) -> f64 {
    match v.as_str() {
        Some("NaN") => f64::NAN,
        Some("+Inf") => f64::INFINITY,
        Some("-Inf") => f64::NEG_INFINITY,
        _ => v.as_f64().unwrap(),
    }
}
fn vector(v: &Value) -> Vec {
    nv(number(&v["x"]), number(&v["y"]), number(&v["z"]))
}
fn vectors(v: &Value) -> std::vec::Vec<Vec> {
    v.as_array().unwrap().iter().map(vector).collect()
}
fn rect(v: &Value) -> RouteCaptionRect {
    RouteCaptionRect {
        x: number(&v["x"]),
        z: number(&v["z"]),
        ux: number(&v["ux"]),
        uz: number(&v["uz"]),
        hw: number(&v["hw"]),
        hd: number(&v["hd"]),
        left: number(&v["left"]),
        right: number(&v["right"]),
        front: number(&v["front"]),
        back: number(&v["back"]),
        hard: v["hard"].as_bool().unwrap(),
        hull: v["hull"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| RouteCaptionPoint {
                x: number(&p[0]),
                z: number(&p[1]),
            })
            .collect(),
    }
}
fn rect_value(r: &RouteCaptionRect) -> Value {
    json!({"x":r.x,"z":r.z,"ux":r.ux,"uz":r.uz,"hw":r.hw,"hd":r.hd,"left":r.left,"right":r.right,"front":r.front,"back":r.back,"hard":r.hard,"hull":r.hull.iter().map(|p|[p.x,p.z]).collect::<std::vec::Vec<_>>()})
}
fn placer(v: &Value) -> RouteCaptionPlacer {
    let mut p = RouteCaptionPlacer::new();
    for run in v["rects"].as_array().unwrap() {
        for _ in 0..run["count"].as_u64().unwrap() {
            p.reserve(rect(&run["rect"]));
        }
    }
    assert_eq!(p.refs, v["refs"].as_u64().unwrap() as usize);
    p.work = v["work"].as_u64().unwrap() as usize;
    p
}
fn state(p: &RouteCaptionPlacer) -> Value {
    let mut runs = vec![];
    let mut i = 0;
    while i < p.rects.len() {
        let r = rect_value(&p.rects[i]);
        let mut j = i + 1;
        while j < p.rects.len() && rect_value(&p.rects[j]) == r {
            j += 1;
        }
        runs.push(json!({"rect":r,"count":j-i}));
        i = j;
    }
    json!({"rects":runs,"work":p.work,"refs":p.refs})
}
fn surface(s: &LabelSurface) -> Value {
    json!({"center":s.center,"width":s.width,"depth":s.depth,"angle":s.angle,"align":s.align})
}
fn replay(op: &Value) -> Value {
    use super::super::route_caption_style::{
        native_connection_caption_surface, native_route_caption_size,
    };
    match op["op"].as_str().unwrap() {
        "rounded" => json!(super::super::geometry_routes::native_rounded_route(
            &vectors(&op["points"])
        )),
        "dashes" => {
            let lengths: std::vec::Vec<f64> = op["lengths"]
                .as_array()
                .unwrap()
                .iter()
                .map(number)
                .collect();
            json!(super::super::geometry_routes::native_route_dashes(
                &vectors(&op["points"]),
                &lengths,
                number(&op["start"]),
                number(&op["end"]),
                number(&op["dash"]),
                op["budget"].as_u64().unwrap() as usize
            ))
        }
        "depth" => {
            let f = &op["face"];
            let points = vectors(&f["points"]);
            let face = super::super::native_edges::ClassicInkFacet {
                points: points.try_into().unwrap(),
                bias: number(&f["bias"]),
                keys: serde_json::from_value(f["keys"].clone()).unwrap(),
                normal: vector(&f["normal"]),
            };
            let depth = face.depth_at(vector(&op["point"]));
            json!({"depth":depth.unwrap_or(0.0),"inside":depth.is_some()})
        }
        "segments" => replay_segments(op),
        "face" => {
            use super::super::{
                geometry::MeshBuilder,
                mesh::{Camera, Material, Vertex, material},
                native_edges::ClassicInkFacet,
            };
            let vertices: [Vertex; 3] = std::array::from_fn(|i| {
                let v = &op["vertices"][i];
                Vertex {
                    position: vector(&v["Position"]),
                    normal: vector(&v["Normal"]),
                    u: number(&v["U"]),
                    v: number(&v["V"]),
                }
            });
            let faces: std::vec::Vec<_> = op["support"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    Rc::new(ClassicInkFacet {
                        points: vectors(&f["points"]).try_into().unwrap(),
                        bias: number(&f["bias"]),
                        keys: serde_json::from_value(f["keys"].clone()).unwrap(),
                        normal: vector(&f["normal"]),
                    })
                })
                .collect();
            let camera = Camera {
                right: vector(&op["camera"]["right"]),
                up: vector(&op["camera"]["up"]),
                direction: vector(&op["camera"]["direction"]),
                ..Default::default()
            };
            let m = material(Material {
                color: serde_json::from_value(op["color"].clone()).unwrap(),
                roughness: number(&op["roughness"]),
                metalness: number(&op["metalness"]),
                emissive: serde_json::from_value(op["emissive"].clone()).unwrap(),
                unlit: op["unlit"].as_bool().unwrap(),
                multiply: op["multiply"].as_bool().unwrap(),
                svg_contour: op["contour"].as_bool().unwrap(),
                svg_solid_texture: op["solid"].as_bool().unwrap(),
                gradient: op["gradient"].as_str().unwrap().into(),
                ..Default::default()
            });
            let mut b = MeshBuilder::default();
            b.classic_ink_face(vertices, &faces, &m, &camera);
            let triangles: std::vec::Vec<_> = b.triangles.iter().map(|t| {
                assert!(Rc::ptr_eq(t.material.as_ref().unwrap(),&m));
                assert!(t.opacity_group.is_none()&&t.paint_owner.is_none()&&t.shadow_ground.is_none());
                json!({"vertices":t.v.iter().map(|v|json!({"Position":v.position,"Normal":v.normal,"U":v.u,"V":v.v})).collect::<std::vec::Vec<_>>(),"shadow":t.cast_shadow,"noDepth":t.no_depth_write,"bias":t.depth_bias})
            }).collect();
            json!({"triangles":triangles,"error":b.err.as_deref().unwrap_or_default()})
        }
        "size" => {
            let text: Text = serde_json::from_value(op["style"].clone()).unwrap();
            let (w, d) = native_route_caption_size(
                &text,
                op["icon"].as_bool().unwrap(),
                number(&op["scale"]),
            );
            json!({"width":w,"depth":d})
        }
        "overlap" => json!(caption_overlap(&rect(&op["a"]), &rect(&op["b"]))),
        "place" => {
            let points: std::vec::Vec<Vec> = op["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| nv(number(&p["x"]), number(&p["y"]), number(&p["z"])))
                .collect();
            let mut p = placer(&op["state"]);
            let s = p.place(
                &points,
                number(&op["fraction"]),
                number(&op["width"]),
                number(&op["depth"]),
            );
            json!({"surface":surface(&s),"state":state(&p),"points":points})
        }
        "surface" => {
            let points: std::vec::Vec<Vec> = op["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| nv(number(&p["x"]), number(&p["y"]), number(&p["z"])))
                .collect();
            let c: Connection = serde_json::from_value(op["connection"].clone()).unwrap();
            let s = native_connection_caption_surface(
                &points,
                &c,
                number(&op["width"]),
                number(&op["depth"]),
                number(&op["scale"]),
            );
            json!({"surface":surface(s.as_ref().unwrap_or(&LabelSurface::default())),"ok":s.is_some(),"points":points,"connection":op["connection"]})
        }
        _ => panic!("unknown caption operation"),
    }
}
fn replay_segments(op: &Value) -> Value {
    use super::super::mesh::{Material, Texture, Triangle, Vertex, material};
    use super::super::{context::RenderContext, native_edges::classic_ink_segments_with_context};
    let triangles: std::vec::Vec<_> = op["triangles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let material_value = &t["material"];
            let m = if material_value.is_null() {
                None
            } else {
                Some(material(Material {
                    color: serde_json::from_value(material_value["color"].clone()).unwrap(),
                    texture: if material_value["texture"] == true {
                        Some(Rc::new(Texture::new(0, 0, vec![])))
                    } else {
                        None
                    },
                    ..Default::default()
                }))
            };
            Triangle {
                v: std::array::from_fn(|i| {
                    let v = &t["vertices"][i];
                    Vertex {
                        position: vector(&v["Position"]),
                        normal: vector(&v["Normal"]),
                        u: number(&v["U"]),
                        v: number(&v["V"]),
                    }
                }),
                material: m,
                cast_shadow: t["shadow"].as_bool().unwrap(),
                no_depth_write: t["noDepth"].as_bool().unwrap(),
                depth_bias: number(&t["bias"]),
                ..Default::default()
            }
        })
        .collect();
    let token = d2_rs_core::CancellationToken::new();
    if op["canceled"] == true {
        token.cancel();
    }
    let context = RenderContext::from_token(&token);
    let node = super::super::geometry_api_tests::node(&op["node"]);
    let radius = op["radius"]
        .as_array()
        .and_then(|a| a.first())
        .map(number)
        .unwrap_or(0.0);
    match classic_ink_segments_with_context(&context, &node, &triangles, radius) {
        Err(error) => json!({"segments":[],"error":error}),
        Ok(segments) => {
            json!({"segments":segments.iter().map(|s|json!({"a":s.a,"c":s.c,"support":s.support.iter().map(|f|json!({"points":f.points,"bias":f.bias,"keys":f.keys,"normal":f.normal})).collect::<std::vec::Vec<_>>()})).collect::<std::vec::Vec<_>>(),"error":""})
        }
    }
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|_| {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/caption-api.json").into()
        });
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
    if let Ok(path) = std::env::var("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(path, serde_json::to_vec(&output).unwrap()).unwrap();
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
        super::super::geometry_api_tests::assert_same(&json!(expected), &json!(output));
    }
}
