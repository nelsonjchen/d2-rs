// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of original solid contacts and projection geometry.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{
    geometry::MeshBuilder,
    geometry_api_tests::{board, node},
    hierarchy_boards::hierarchy_casing_floor,
    mesh::*,
    native_shapes::*,
    native_solids::*,
    native_style::native_classic_node,
};
use super::*;
use crate::scene::{Edge, EdgeMetadata};
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
pub(crate) fn placer(v: &Value) -> RouteCaptionPlacer {
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
pub(crate) fn state(p: &RouteCaptionPlacer) -> Value {
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
fn point(v: &Value) -> Vec {
    nv(number(&v[0]), number(&v[1]), number(&v[2]))
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
fn point_value(p: Vec) -> Value {
    json!([float_value(p.x), float_value(p.y), float_value(p.z)])
}
fn edge(e: &Value) -> Edge {
    Edge {
        id: e["id"].as_str().unwrap().into(),
        source: e["source"].as_str().unwrap().into(),
        target: e["target"].as_str().unwrap().into(),
        label: e["label"].as_str().unwrap().into(),
        source_label: serde_json::from_value(e["sourceLabel"].clone()).unwrap(),
        target_label: serde_json::from_value(e["targetLabel"].clone()).unwrap(),
        source_arrow: e["sourceArrow"].as_str().unwrap().into(),
        target_arrow: e["targetArrow"].as_str().unwrap().into(),
        stroke: e["stroke"].as_str().unwrap().into(),
        stroke_explicit: e["strokeExplicit"].as_bool().unwrap(),
        font_color: e["fontColor"].as_str().unwrap().into(),
        font_explicit: e["fontExplicit"].as_bool().unwrap(),
        stroke_dash: number(&e["strokeDash"]),
        stroke_width: e["strokeWidth"].as_i64().unwrap(),
        opacity: number(&e["opacity"]),
        points: e["points"].as_array().map(|_| vectors(&e["points"])),
        link: e["link"].as_str().unwrap_or_default().into(),
        tooltip: e["tooltip"].as_str().unwrap_or_default().into(),
        icon: e["icon"].as_str().unwrap_or_default().into(),
        sequence_role: e["sequenceRole"].as_str().unwrap_or_default().into(),
        metadata: EdgeMetadata {
            original: serde_json::from_value(e["metadata"]["original"].clone()).unwrap(),
        },
    }
}
fn vertex(v: &Value) -> Vertex {
    Vertex {
        position: vector(&v["Position"]),
        normal: vector(&v["Normal"]),
        u: number(&v["U"]),
        v: number(&v["V"]),
    }
}
fn vertex_value(v: &Vertex) -> Value {
    json!({"Position":v.position,"Normal":v.normal,"U":v.u,"V":v.v})
}
fn material_input(v: &Value) -> Option<MaterialRef> {
    if v.is_null() {
        return None;
    }
    Some(material(Material {
        color: serde_json::from_value(v["color"].clone()).unwrap(),
        roughness: number(&v["roughness"]),
        metalness: number(&v["metalness"]),
        emissive: serde_json::from_value(v["emissive"].clone()).unwrap(),
        unlit: v["unlit"].as_bool().unwrap(),
        multiply: v["multiply"].as_bool().unwrap(),
        svg_contour: v["contour"].as_bool().unwrap(),
        svg_solid_texture: v["solid"].as_bool().unwrap(),
        gradient: v["gradient"].as_str().unwrap().into(),
        ..Default::default()
    }))
}
fn material_value(m: Option<&MaterialRef>) -> Value {
    let Some(m) = m else { return Value::Null };
    let m = m.borrow();
    assert!(m.texture.is_none() && m.vector.is_none());
    json!({"color":m.color,"roughness":m.roughness,"metalness":m.metalness,"emissive":m.emissive,"unlit":m.unlit,"multiply":m.multiply,"contour":m.svg_contour,"solid":m.svg_solid_texture,"gradient":m.gradient})
}
fn triangles_input(ts: &Value) -> std::vec::Vec<Triangle> {
    ts.as_array()
        .unwrap()
        .iter()
        .map(|t| Triangle {
            v: std::array::from_fn(|i| vertex(&t["vertices"][i])),
            material: material_input(&t["material"]),
            depth_bias: number(&t["bias"]),
            cast_shadow: t["shadow"].as_bool().unwrap(),
            no_depth_write: t["noDepth"].as_bool().unwrap(),
            ..Default::default()
        })
        .collect()
}
fn builder(v: &Value) -> MeshBuilder {
    let mut b = MeshBuilder {
        route_casing_floor: number(&v["floor"]),
        ..Default::default()
    };
    b.triangles = triangles_input(&v["triangles"]);
    b.err = v["error"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);
    b
}
fn builder_value(b: &MeshBuilder) -> Value {
    let triangles:std::vec::Vec<_>=b.triangles.iter().map(|t| {
 assert!(t.opacity_group.is_none()&&t.paint_owner.is_none()&&t.shadow_ground.is_none()&&t.shadow_opacity.is_none()&&t.shadow_fill_alpha.is_none()&&!t.svg_coverage_only);
 json!({"vertices":t.v.iter().map(vertex_value).collect::<std::vec::Vec<_>>(),"material":material_value(t.material.as_ref()),"bias":t.depth_bias,"shadow":t.cast_shadow,"noDepth":t.no_depth_write})
 }).collect();
    json!({"triangles":triangles,"error":b.err.as_deref().unwrap_or_default(),"floor":b.route_casing_floor})
}
fn projection_mesh(v: &Value) -> std::vec::Vec<Triangle> {
    let materials: std::vec::Vec<_> = v["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            material(Material {
                color: serde_json::from_value(m["color"].clone()).unwrap(),
                texture: if m["texture"] == true {
                    Some(Rc::new(Texture::new(1, 1, vec![0; 4])))
                } else {
                    None
                },
                ..Default::default()
            })
        })
        .collect();
    v["triangles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| Triangle {
            v: std::array::from_fn(|i| vertex(&t["vertices"][i])),
            material: if t["material"].as_i64().unwrap() < 0 {
                None
            } else {
                Some(materials[t["material"].as_u64().unwrap() as usize].clone())
            },
            ..Default::default()
        })
        .collect()
}
fn replay(op: &Value) -> Value {
    match op["op"].as_str().unwrap() {
        "queue" | "upright" => {
            let n = node(&op["node"]);
            let p = point(&op["point"]);
            let d = point(&op["direction"]);
            let (p, m) = if op["op"] == "queue" {
                native_queue_contact(&n, p, d)
            } else {
                native_upright_contact(&n, p, d)
            };
            json!({"point":point_value(p),"moved":m})
        }
        "crown" => json!(native_queue_crown(&node(&op["node"]))),
        "classic" => {
            let n = node(&op["node"]);
            json!({"styled":native_classic_node(&n),"node":n})
        }
        "faceSource" => {
            let n = node(&op["node"]);
            json!({"shape":native_face_source(&n,op["fill"].as_str().unwrap()),"node":n})
        }
        "profile" => {
            let s = serde_json::from_value(op["shape"].clone()).unwrap();
            let r = native_shape_profile(&s, None);
            match r {
                Ok(p) => json!({"profile":p,"shape":s,"error":""}),
                Err(e) => json!({"profile":null,"shape":s,"error":e}),
            }
        }
        "floor" => {
            let bs = op["boards"]
                .as_array()
                .unwrap()
                .iter()
                .map(board)
                .collect::<std::vec::Vec<_>>();
            let ns = op["nodes"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.as_str(), node(v)))
                .collect::<std::collections::HashMap<_, _>>();
            let refs = ns.iter().map(|(k, v)| (*k, v)).collect();
            json!(hierarchy_casing_floor(&bs, &refs))
        }
        "ink" | "casing" | "extrude" => {
            let mut b = builder(&op["builder"]);
            let ps = vectors(&op["points"]);
            match op["op"].as_str().unwrap() {
                "ink" => b.route_ink(
                    &ps,
                    number(&op["radius"]),
                    &material_input(&op["material"]).unwrap(),
                ),
                "casing" => b.route_casing(&ps, number(&op["radius"]), number(&op["opacity"])),
                _ => b.extruded_profile(
                    &ps,
                    number(&op["base"]),
                    material_input(&op["top"]).as_ref(),
                    &material_input(&op["side"]).unwrap(),
                ),
            };
            json!({"builder":builder_value(&b),"points":ps})
        }
        "contacts" | "pipeline" => {
            let es = op["edges"]
                .as_array()
                .unwrap()
                .iter()
                .map(edge)
                .collect::<std::vec::Vec<_>>();
            let ns = op["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(node)
                .collect::<std::vec::Vec<_>>();
            let support = op["support"]
                .as_array()
                .and_then(|s| s.first())
                .filter(|s| !s.is_null())
                .map(|s| {
                    serde_json::from_value::<std::collections::HashMap<String, f64>>(s.clone())
                        .unwrap()
                });
            let token = d2_rs_core::CancellationToken::new();
            if op["context"] == "context canceled" {
                token.cancel()
            };
            let context = super::super::context::RenderContext::from_token(&token);
            let r = if op["op"] == "contacts" {
                let ps = op["paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(vectors)
                    .collect::<std::vec::Vec<_>>();
                native_solid_contact_routes_with_context(
                    (op["context"] != "nil").then_some(&context),
                    &es,
                    &ns,
                    &ps,
                    support.as_ref(),
                )
                .map(|ps| (vec![], ps))
            } else {
                super::super::routes::native_edge_routes_with_context(
                    &es,
                    &ns,
                    &[],
                    &support.clone().unwrap_or_default(),
                    &context,
                )
            };
            let (lanes, paths, error) = match r {
                Ok((l, p)) => (json!(l), json!(p), String::new()),
                Err(e) => (Value::Null, Value::Null, e),
            };
            let mut out =
                json!({"paths":paths,"error":error,"edges":es,"nodes":ns,"support":op["support"]});
            if op["op"] == "contacts" {
                out["sourcePaths"] = op["paths"].clone()
            } else {
                out["boards"] = op["boards"].clone();
                out["lanes"] = lanes
            };
            out
        }
        "place" | "avoid" | "avoidMesh" | "avoidRoute" | "score" => {
            let mut p = placer(&op["state"]);
            match op["op"].as_str().unwrap() {
                "place" => {
                    let ps = vectors(&op["points"]);
                    let s = p.place(
                        &ps,
                        number(&op["fraction"]),
                        number(&op["width"]),
                        number(&op["depth"]),
                    );
                    json!({"surface":surface(&s),"state":state(&p),"points":ps})
                }
                "avoid" => {
                    p.avoid(
                        vector(&op["center"]),
                        number(&op["width"]),
                        number(&op["depth"]),
                    );
                    state(&p)
                }
                "avoidMesh" => {
                    p.avoid_mesh(&projection_mesh(&op["mesh"]));
                    state(&p)
                }
                "avoidRoute" => {
                    let ps = vectors(&op["points"]);
                    p.avoid_route(&ps, number(&op["radius"]));
                    json!({"state":state(&p),"points":ps})
                }
                _ => {
                    let (s, c) = p.score(&rect(&op["rect"]));
                    json!({"score":s,"complete":c,"state":state(&p)})
                }
            }
        }
        _ => panic!("unknown operation {op}"),
    }
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|_| {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/contact-api.json").into()
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
