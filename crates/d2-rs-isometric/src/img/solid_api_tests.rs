// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of original solids, reliefs and caption mesh calls.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{
    geometry::{LabelSurface, MeshBuilder},
    geometry_api_tests::node,
    mesh::*,
    route_labels::RouteCaptionPlacer,
    surface_icons::SurfaceIconPainter,
    text::TextPainter,
};
use super::*;
use crate::scene::{Edge, EdgeMetadata, Scene};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::rc::Rc;
use std::vec::Vec as List;
fn number(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn vec(v: &Value) -> Vec {
    nv(number(&v["x"]), number(&v["y"]), number(&v["z"]))
}
pub(crate) fn texture(t: Option<&Rc<Texture>>) -> Value {
    let Some(t) = t else { return Value::Null };
    assert_eq!(t.format, TextureFormat::Rgba);
    let bytes = t.pixels.borrow();
    assert_eq!(bytes.len(), t.width * t.height * 4);
    json!({"width":t.width,"height":t.height,"sha256":format!("{:x}",Sha256::digest(&*bytes))})
}
pub(crate) fn mesh(ts: &[Triangle]) -> Value {
    let (mut materials, mut groups, mut owners, mut triangles) = (vec![], vec![], vec![], vec![]);
    let (mut ms, mut gs, mut os) = (vec![], vec![], vec![]);
    fn index<T>(rs: &mut List<Rc<T>>, r: &Rc<T>) -> (usize, bool) {
        if let Some(i) = rs.iter().position(|a| Rc::ptr_eq(a, r)) {
            (i + 1, false)
        } else {
            rs.push(r.clone());
            (rs.len(), true)
        }
    }
    for t in ts {
        let mi = if let Some(m) = &t.material {
            let (i, new) = index(&mut ms, m);
            if new {
                let m = m.borrow();
                materials.push(json!({"color":m.color,"roughness":m.roughness,"metalness":m.metalness,"emissive":m.emissive,"unlit":m.unlit,"multiply":m.multiply,"contour":m.svg_contour,"solid":m.svg_solid_texture,"gradient":m.gradient,"vector":m.vector.is_some(),"texture":texture(m.texture.as_ref())}));
            }
            i
        } else {
            0
        };
        let gi = if let Some(g) = &t.opacity_group {
            let (i, new) = index(&mut gs, g);
            if new {
                groups.push(g.borrow().opacity);
            }
            i
        } else {
            0
        };
        let oi = if let Some(o) = &t.paint_owner {
            let (i, new) = index(&mut os, o);
            if new {
                owners.push(o.borrow().opaque);
            }
            i
        } else {
            0
        };
        triangles.push(json!({"v":t.v.iter().map(|v|json!({"Position":v.position,"Normal":v.normal,"U":v.u,"V":v.v})).collect::<List<_>>(),"material":mi,"group":gi,"owner":oi,"shadow":t.cast_shadow,"opacity":t.shadow_opacity,"alpha":t.shadow_fill_alpha,"ground":t.shadow_ground,"bias":t.depth_bias,"noDepth":t.no_depth_write,"coverage":t.svg_coverage_only}));
    }
    json!({"triangles":triangles,"materials":materials,"groups":groups,"owners":owners})
}
pub(crate) fn mesh_input(v: &Value) -> List<Triangle> {
    let materials: List<_> = v["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            assert_eq!(m["vector"], false);
            material(Material {
                color: serde_json::from_value(m["color"].clone()).unwrap(),
                roughness: number(&m["roughness"]),
                metalness: number(&m["metalness"]),
                emissive: serde_json::from_value(m["emissive"].clone()).unwrap(),
                unlit: m["unlit"].as_bool().unwrap(),
                multiply: m["multiply"].as_bool().unwrap(),
                svg_contour: m["contour"].as_bool().unwrap(),
                svg_solid_texture: m["solid"].as_bool().unwrap(),
                gradient: m["gradient"].as_str().unwrap().into(),
                texture: if m["texture"].is_null() {
                    None
                } else {
                    let t = &m["texture"];
                    Some(Rc::new(Texture::new(
                        t["width"].as_u64().unwrap() as usize,
                        t["height"].as_u64().unwrap() as usize,
                        STANDARD.decode(t["pixels"].as_str().unwrap()).unwrap(),
                    )))
                },
                ..Default::default()
            })
        })
        .collect();
    let groups: List<_> = v["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| Rc::new(std::cell::RefCell::new(OpacityGroup { opacity: number(g) })))
        .collect();
    let owners: List<_> = v["owners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            Rc::new(std::cell::RefCell::new(PaintOwner {
                opaque: o.as_bool().unwrap(),
            }))
        })
        .collect();
    fn entry<T>(v: &Value, rs: &[Rc<T>]) -> Option<Rc<T>> {
        let i = v.as_u64().unwrap() as usize;
        if i == 0 {
            None
        } else {
            Some(rs[i - 1].clone())
        }
    }
    v["triangles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| Triangle {
            v: std::array::from_fn(|i| {
                let v = &t["v"][i];
                Vertex {
                    position: vec(&v["Position"]),
                    normal: vec(&v["Normal"]),
                    u: number(&v["U"]),
                    v: number(&v["V"]),
                }
            }),
            material: entry(&t["material"], &materials),
            opacity_group: entry(&t["group"], &groups),
            paint_owner: entry(&t["owner"], &owners),
            cast_shadow: t["shadow"].as_bool().unwrap(),
            shadow_opacity: t["opacity"].as_f64(),
            shadow_fill_alpha: t["alpha"].as_u64().map(|a| a as u8),
            shadow_ground: t["ground"].as_f64(),
            depth_bias: number(&t["bias"]),
            no_depth_write: t["noDepth"].as_bool().unwrap(),
            svg_coverage_only: t["coverage"].as_bool().unwrap(),
        })
        .collect()
}
pub(crate) fn builder(v: &Value) -> MeshBuilder {
    let mut b = MeshBuilder {
        scale: number(&v["scale"]),
        output_density: number(&v["density"]),
        face_pixels: v["pixels"].as_i64().unwrap(),
        face_max_pixels: v["budget"].as_i64().unwrap(),
        ..Default::default()
    };
    b.triangles = mesh_input(&v["mesh"]);
    b.err = v["error"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);
    assert!(v["animated"].is_null());
    b
}
pub(crate) fn state(b: &MeshBuilder) -> Value {
    assert!(b.panels.is_empty() && b.links.is_empty());
    json!({"mesh":mesh(&b.triangles),"error":b.err.as_deref().unwrap_or_default(),"scale":b.scale,"density":b.output_density,"pixels":b.face_pixels,"budget":b.face_max_pixels,"animated":if b.animated_nodes.is_empty(){Value::Null}else{json!(b.animated_nodes.iter().map(|a|json!({"first":a.first,"last":a.last})).collect::<List<_>>())}})
}
pub(crate) fn edge(e: &Value) -> Edge {
    Edge {
        sequence_role: e["sequenceRole"].as_str().unwrap_or_default().into(),
        icon: e["icon"].as_str().unwrap_or_default().into(),
        link: e["link"].as_str().unwrap_or_default().into(),
        tooltip: e["tooltip"].as_str().unwrap_or_default().into(),
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
        points: e["points"].as_array().map(|a| a.iter().map(vec).collect()),
        metadata: EdgeMetadata {
            original: serde_json::from_value(e["metadata"]["original"].clone()).unwrap(),
        },
        ..Default::default()
    }
}
pub(crate) fn replay(v: &Value) -> Value {
    let op = v["op"].as_str().unwrap();
    let mut b = if op == "solid" || op == "relief" {
        let mut text = TextPainter::new(2, None).unwrap();
        let mut icons = SurfaceIconPainter::new(1, None, None).unwrap();
        if op == "solid" {
            text.output_density = 100.;
            icons.output_density = 100.;
        }
        MeshBuilder {
            text,
            icons,
            scale: 0.01,
            output_density: if op == "solid" { 100. } else { 0. },
            face_max_pixels: 128 << 10,
            ..Default::default()
        }
    } else {
        builder(&v["state"])
    };
    if op == "knockout" {
        let s = &v["surface"];
        b.route_caption_knockout(
            v["first"].as_u64().unwrap() as usize,
            &LabelSurface {
                center: vec(&s["center"]),
                width: number(&s["width"]),
                depth: number(&s["depth"]),
                angle: number(&s["angle"]),
                align: s["align"].as_str().unwrap().into(),
            },
        );
        return state(&b);
    }
    if op == "edges" {
        if v["text"] == true {
            b.text = TextPainter::new(1, None).unwrap();
        }
        let es: List<_> = v["edges"].as_array().unwrap().iter().map(edge).collect();
        let mut scene = Scene::default();
        if let Some(s) = v["scenes"].as_array().and_then(|s| s.first()) {
            scene.background = s["background"].as_str().unwrap().into();
            scene.theme_id = s["themeId"].as_i64().unwrap();
            scene.theme_overrides = serde_json::from_value(s["themeOverrides"].clone()).unwrap();
        }
        b.edges(&es, &mut RouteCaptionPlacer::new(), &scene);
        return json!({"state":state(&b),"edges":es});
    }
    let n = node(&v["node"]);
    match op {
        "solid" => b.node(&n, "#849ebc", None),
        "relief" => {
            if v["hierarchy"] == true {
                b.hierarchy_node(&n, "#849ebc")
            } else {
                b.node(&n, "#849ebc", None)
            }
        }
        "node" => b.node(&n, v["tint"].as_str().unwrap(), None),
        "canonical" => b.canonical_node(&n, v["tint"].as_str().unwrap()),
        "paint" => {
            let p = b.solid_paint(&n, v["kind"].as_str().unwrap(), v["fill"].as_str().unwrap());
            let ts = [p.cap, p.ink, p.wall].map(|m| Triangle {
                material: Some(m),
                ..Default::default()
            });
            return json!({"state":state(&b),"node":n,"paint":mesh(&ts),"fit":p.ink_fit,"alpha":p.shadow_fill_alpha});
        }
        _ => panic!("unknown operation {op}"),
    }
    json!({"state":state(&b),"node":n})
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|_| {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/solid-api.json").into()
        });
    let cases: Value = serde_json::from_str(&source).unwrap();
    let output: List<_> = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| replay(&o["request"]))
                .collect::<List<_>>()
        })
        .collect();
    if let Ok(p) = std::env::var("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(p, serde_json::to_vec(&output).unwrap()).unwrap();
    } else {
        let expected: List<_> = cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                c["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| o["got"].clone())
                    .collect::<List<_>>()
            })
            .collect();
        super::super::geometry_api_tests::assert_same(&json!(expected), &json!(output));
    }
}
