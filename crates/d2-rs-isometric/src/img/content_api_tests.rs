// SPDX-License-Identifier: MPL-2.0
//! Original-test request replay. This module and all JSON construction compile only in tests.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::{geometry::MeshBuilder, mesh::*, vector_surface::*, vector_surface_svg};
use crate::scene::{Edge, EdgeMetadata, Node, NodeMetadata, Scene};
use base64::Engine;
use d2_rs_scene::{Document, Nrgba};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
fn n(v: &Value) -> f64 {
    if v == "+Inf" {
        f64::INFINITY
    } else {
        v.as_f64().unwrap()
    }
}
fn vec(v: &Value) -> Vec {
    nv(n(&v["x"]), n(&v["y"]), n(&v["z"]))
}
fn strv(v: &Value) -> String {
    v.as_str().unwrap_or_default().into()
}
fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_default()
}
fn flag(v: &Value) -> bool {
    v.as_bool().unwrap_or_default()
}
fn col(v: &Value) -> Nrgba {
    serde_json::from_value(v.clone()).unwrap()
}
fn pts(v: &Value) -> std::vec::Vec<Vec> {
    v.as_array()
        .map(|v| v.iter().map(vec).collect())
        .unwrap_or_default()
}
fn edge(v: &Value) -> Edge {
    Edge {
        id: strv(&v["id"]),
        source: strv(&v["source"]),
        target: strv(&v["target"]),
        label: strv(&v["label"]),
        source_label: serde_json::from_value(v["sourceLabel"].clone()).unwrap(),
        target_label: serde_json::from_value(v["targetLabel"].clone()).unwrap(),
        source_arrow: strv(&v["sourceArrow"]),
        target_arrow: strv(&v["targetArrow"]),
        stroke: strv(&v["stroke"]),
        stroke_explicit: flag(&v["strokeExplicit"]),
        font_color: strv(&v["fontColor"]),
        font_explicit: flag(&v["fontExplicit"]),
        stroke_width: v["strokeWidth"].as_i64().unwrap_or_default(),
        stroke_dash: num(&v["strokeDash"]),
        opacity: num(&v["opacity"]),
        points: (!v["points"].is_null()).then(|| pts(&v["points"])),
        link: strv(&v["link"]),
        tooltip: strv(&v["tooltip"]),
        icon: strv(&v["icon"]),
        sequence_role: strv(&v["sequenceRole"]),
        metadata: EdgeMetadata {
            original: serde_json::from_value(v["metadata"]["original"].clone()).unwrap(),
        },
    }
}
fn surface(v: &Value) -> Option<Rc<VectorSurface>> {
    if v.is_null() {
        return None;
    }
    Some(Rc::new(VectorSurface {
        document: Rc::new(RefCell::new(document(&v["document"]))),
        ink_coverage: RefCell::new(surface(&v["coverage"])),
        coverage_opacity: std::cell::Cell::new(n(&v["opacity"])),
        cap_background: serde_json::from_value(v["cap"].clone()).unwrap(),
        cap_paint: serde_json::from_value(v["paint"].clone()).unwrap(),
    }))
}
fn hydrate(v: &mut Value) {
    match v {
        Value::Object(o) => {
            if let Some(path) = o.remove("DataFile") {
                o.insert(
                    "Data".into(),
                    json!(
                        base64::engine::general_purpose::STANDARD
                            .encode(std::fs::read(path.as_str().unwrap()).unwrap())
                    ),
                );
            }
            for v in o.values_mut() {
                hydrate(v)
            }
        }
        Value::Array(a) => {
            for v in a {
                hydrate(v)
            }
        }
        _ => {}
    }
}
fn document(v: &Value) -> Document {
    let mut v = v.clone();
    hydrate(&mut v);
    serde_json::from_value(v).unwrap()
}
fn surf_value(v: Option<&VectorSurface>) -> Value {
    v.map(|s|json!({"document":&*s.document.borrow(),"coverage":surf_value(s.ink_coverage.borrow().as_deref()),"opacity":s.coverage_opacity.get(),"cap":s.cap_background,"paint":s.cap_paint})).unwrap_or(Value::Null)
}
fn image(t: &Texture) -> Value {
    json!({"width":t.width,"height":t.height,"stride":t.width*4,"pixels":base64::engine::general_purpose::STANDARD.encode(&*t.pixels.borrow())})
}
fn mat(v: &Value) -> MaterialRef {
    let t = &v["texture"];
    material(Material {
        color: col(&v["color"]),
        emissive: col(&v["emissive"]),
        roughness: n(&v["roughness"]),
        metalness: n(&v["metalness"]),
        unlit: flag(&v["unlit"]),
        multiply: flag(&v["multiply"]),
        svg_contour: flag(&v["contour"]),
        svg_solid_texture: flag(&v["solid"]),
        gradient: strv(&v["gradient"]),
        vector: surface(&v["vector"]),
        texture: (!t.is_null()).then(|| {
            Rc::new(Texture::new(
                t["width"].as_u64().unwrap() as usize,
                t["height"].as_u64().unwrap() as usize,
                base64::engine::general_purpose::STANDARD
                    .decode(t["pixels"].as_str().unwrap())
                    .unwrap(),
            ))
        }),
    })
}
fn mat_value(m: Option<&MaterialRef>) -> Value {
    m.map(|m|{let m=m.borrow();json!({"color":m.color,"roughness":m.roughness,"metalness":m.metalness,"emissive":m.emissive,"texture":m.texture.as_deref().map(image),"vector":surf_value(m.vector.as_deref()),"unlit":m.unlit,"multiply":m.multiply,"contour":m.svg_contour,"solid":m.svg_solid_texture,"gradient":m.gradient})}).unwrap_or(Value::Null)
}
fn triangles(ts: &[Triangle]) -> Value {
    let mut ids = HashMap::new();
    json!(ts.iter().map(|t|{let id=t.material.as_ref().map(|m|{let next=ids.len()+1;*ids.entry(Rc::as_ptr(m) as usize).or_insert(next)}).unwrap_or(0);json!({"vertices":t.v.iter().map(|v|json!({"Position":v.position,"Normal":v.normal,"U":v.u,"V":v.v})).collect::<std::vec::Vec<_>>(),"material":mat_value(t.material.as_ref()),"material_id":id,"shadow":t.cast_shadow,"opacity":t.shadow_opacity,"fill_alpha":t.shadow_fill_alpha,"ground":t.shadow_ground,"bias":t.depth_bias,"no_depth":t.no_depth_write,"coverage":t.svg_coverage_only,"group":t.opacity_group.as_ref().map(|g|json!({"Opacity":g.borrow().opacity})),"owner":t.paint_owner.as_ref().map(|o|json!({"Opaque":o.borrow().opaque}))})}).collect::<std::vec::Vec<_>>())
}
fn builder(v: &Value) -> MeshBuilder {
    let mut b = MeshBuilder {
        scale: n(&v["scale"]),
        output_density: n(&v["density"]),
        face_max_pixels: v["face_max"].as_i64().unwrap(),
        ..Default::default()
    };
    if flag(&v["text"]) {
        b.text = super::text::TextPainter::new(1, None).unwrap()
    }
    if flag(&v["rich"]) {
        b.rich = super::rich_labels::RichLabelPainter::new(1, None).unwrap()
    }
    b
}
fn area(s: &super::geometry::LabelSurface) -> Value {
    json!({"center":s.center,"width":s.width,"depth":s.depth,"angle":s.angle,"align":s.align})
}
fn replay(v: &Value) -> Value {
    match v["op"].as_str().unwrap() {
        "svg" => {
            let s = surface(&v["surface"]).unwrap();
            let token = d2_rs_core::CancellationToken::new();
            if flag(&v["canceled"]) {
                token.cancel();
            }
            let context = super::context::RenderContext::from_token(&token);
            let r = vector_surface_svg::native_surface_svg_with_context(
                &s,
                v["prefix"].as_str().unwrap(),
                &context,
            );
            match r {
                Ok(s) => json!({"error":"","svg":s}),
                Err(e) => json!({"error":e,"svg":""}),
            }
        }

        "aperture" => {
            let surface = surface(&v["surface"]).unwrap();
            native_vector_aperture(&surface, serde_json::from_value(v["area"].clone()).unwrap());
            json!({"error":"","surface":surf_value(Some(&surface))})
        }
        "face_opacity" => {
            let vectors = native_vector_context();
            let texture = |v: &Value| {
                Rc::new(Texture::new(
                    v["width"].as_u64().unwrap() as usize,
                    v["height"].as_u64().unwrap() as usize,
                    base64::engine::general_purpose::STANDARD
                        .decode(v["pixels"].as_str().unwrap())
                        .unwrap(),
                ))
            };
            let fill = texture(&v["fill"]);
            let ink = texture(&v["ink"]);
            for (t, s) in [(&fill, &v["fill_vector"]), (&ink, &v["ink_vector"])] {
                if let Some(s) = surface(s) {
                    retain_native_vector_surface(&vectors, t, &s.document.borrow()).unwrap();
                }
            }
            let mut b = MeshBuilder {
                vectors: vectors.clone(),
                ..Default::default()
            };
            b.native_face_opacity(&fill, &ink, n(&v["opacity"]));
            json!({"error":b.err.clone().unwrap_or_default(),"fill":image(&fill),"ink":image(&ink),"fill_vector":surf_value(native_vector_for_texture(&vectors,&fill).as_deref()),"ink_vector":surf_value(native_vector_for_texture(&vectors,&ink).as_deref())})
        }
        "cap" => {
            let s = surface(&v["surface"]);
            let out = native_vector_solid_cap(s.clone(), col(&v["color"]));
            let same = match (&s, &out) {
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            json!({"error":"","surface":surf_value(out.as_deref()),"same":same})
        }
        "matrix" => {
            json!({"error":"","value":vector_surface_svg::matrix(serde_json::from_value(v["matrix"].clone()).unwrap())})
        }
        "view" => json!({"error":"","value":native_view_direction()}),
        "tint" => {
            json!({"error":"","value":super::route_style::native_route_tint(&edge(&v["edge"]))})
        }
        "section" => {
            let lengths: std::vec::Vec<f64> = serde_json::from_value(v["lengths"].clone()).unwrap();
            json!({"error":"","value":super::geometry_routes::native_route_section(&pts(&v["points"]),&lengths,n(&v["a"]),n(&v["b"]))})
        }
        "shade" => {
            let p = &v["paint"];
            let p = RasterPaint {
                base: vec(&p["base"]),
                emission: vec(&p["emission"]),
                alpha: n(&p["alpha"]),
                roughness: n(&p["roughness"]),
                metalness: n(&p["metalness"]),
                unlit: flag(&p["unlit"]),
                multiply: flag(&p["multiply"]),
                ..Default::default()
            };
            let (a, b, c) = shade(
                &p,
                super::framing::native_camera_axes().direction,
                vec(&v["position"]),
                vec(&v["normal"]),
                0.0,
            );
            json!({"error":"","rgb":[a,b,c]})
        }
        "document" => {
            let d = serde_json::from_value(v["diagram"].clone()).unwrap();
            match super::source_panels::surface_document(&d, None) {
                Ok(d) => json!({"error":"","document":d}),
                Err(e) => json!({"error":e,"document":null}),
            }
        }
        "raster_doc" => {
            let doc = document(&v["document"]);
            let w = v["width"].as_u64().unwrap() as usize;
            let h = v["height"].as_u64().unwrap() as usize;
            let at = v["at"].as_array().unwrap().first().map(n).unwrap_or(0.0);
            let result = super::backends::raster_surface_at(&doc, w, h, at);
            match result {
                Ok(p) => {
                    json!({"error":"","frame":image(&Texture::new(w,h,p)),"source":doc})
                }
                Err(e) => json!({"error":e,"frame":null,"source":doc}),
            }
        }
        op => {
            let mut b = builder(v);
            match op {
                "node" => b.node(&node(&v["node"]), v["tint"].as_str().unwrap(), None),
                "edges" => {
                    let edges = v["edges"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(edge)
                        .collect::<std::vec::Vec<_>>();
                    b.edges(
                        &edges,
                        &mut super::route_labels::RouteCaptionPlacer::default(),
                        &Scene::default(),
                    );
                }
                "ink" => b.route_ink(&pts(&v["points"]), n(&v["radius"]), &mat(&v["material"])),
                "tube" => b.tube(&pts(&v["points"]), n(&v["radius"]), &mat(&v["material"])),
                "casing" => b.route_casing(&pts(&v["points"]), n(&v["radius"]), n(&v["opacity"])),
                "face" => {
                    let (t, a) =
                        b.native_face(&serde_json::from_value(v["shape"].clone()).unwrap());
                    return json!({"error":b.err.clone().unwrap_or_default(),"frame":t.as_deref().map(image),"area":area(&a)});
                }
                _ => panic!("unknown content op {op}"),
            };
            json!({"error":b.err.clone().unwrap_or_default(),"triangles":triangles(&b.triangles)})
        }
    }
}
#[test]
#[ignore = "requires original Go capture; run via content scoreboard"]
fn replay_original_content_calls() {
    let input = std::env::var("D2RS_CONTENT_REQUEST").unwrap();
    let output = std::env::var("D2RS_CONTENT_OUTPUT").unwrap();
    assert!(!std::path::Path::new(&output).exists());
    let input: Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    std::fs::create_dir(&output).unwrap();
    for (i, c) in input["cases"].as_array().unwrap().iter().enumerate() {
        let values = c["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(replay)
            .collect::<std::vec::Vec<_>>();
        std::fs::write(
            std::path::Path::new(&output).join(format!("{i}.json")),
            serde_json::to_vec(&values).unwrap(),
        )
        .unwrap();
    }
}

fn node(v: &Value) -> Node {
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
        opacity: n(&v["opacity"]),
        stroke_width: v["strokeWidth"].as_i64().unwrap(),
        stroke_dash: n(&v["strokeDash"]),
        tooltip: v["tooltip"].as_str().unwrap_or_default().into(),
        link: v["link"].as_str().unwrap_or_default().into(),
        icon: v["icon"].as_str().unwrap_or_default().into(),
        metadata: NodeMetadata {
            original: serde_json::from_value(v["metadata"]["original"].clone()).unwrap(),
        },
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn retained_content_original_contracts() {
    let cases: Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/content-api.json")).unwrap();
    for case in cases.as_array().unwrap() {
        for op in case["operations"].as_array().unwrap() {
            let got = replay(&op["request"]);
            assert_same(&got, &op["got"]);
        }
    }
}

// JSON has one number type; compare exact values across 1 versus 1.0 spellings.
fn assert_same(a: &Value, b: &Value) {
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
