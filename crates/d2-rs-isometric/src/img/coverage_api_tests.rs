// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of original SVG coverage and ground-receiver calls.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{
    context::RenderContext,
    geometry_api_tests::{board, node},
    ground_receivers::{HierarchyShadowSpan, hierarchy_shadow_receivers},
    native_solids::solid_api_tests::{mesh, mesh_input},
    vector_surface_svg::native_surface_svg,
};
use super::*;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};
fn number(v: &Value) -> f64 {
    match v.as_str() {
        Some("NaN") => f64::NAN,
        Some("+Inf") => f64::INFINITY,
        Some("-Inf") => f64::NEG_INFINITY,
        _ => v.as_f64().unwrap(),
    }
}
fn float(v: f64) -> Value {
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
fn vector(v: &Value) -> Vec {
    nv(number(&v["x"]), number(&v["y"]), number(&v["z"]))
}
fn points(v: &Value) -> Vec2<SvgPoint> {
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
fn polygons(v: &Value) -> Vec2<Vec2<SvgPoint>> {
    v.as_array().unwrap().iter().map(points).collect()
}
fn points_value(ps: &[SvgPoint]) -> Value {
    json!(ps.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec2<_>>())
}
fn polygons_value(ps: &[Vec2<SvgPoint>]) -> Value {
    json!(ps.iter().map(|p| points_value(p)).collect::<Vec2<_>>())
}
fn camera(v: &Value) -> Camera {
    Camera {
        right: vector(&v["right"]),
        up: vector(&v["up"]),
        direction: vector(&v["direction"]),
        center_x: number(&v["centerX"]),
        center_y: number(&v["centerY"]),
        center_depth: number(&v["centerDepth"]),
        scale: number(&v["scale"]),
        width: v["width"].as_i64().unwrap(),
        height: v["height"].as_i64().unwrap(),
    }
}
fn surface(v: &Value) -> Option<Rc<VectorSurface>> {
    if v.is_null() {
        return None;
    };
    Some(Rc::new(VectorSurface {
        document: Rc::new(RefCell::new({
            let mut d = v["document"].clone();
            hydrate(&mut d);
            serde_json::from_value(d).unwrap()
        })),
        ink_coverage: RefCell::new(surface(&v["coverage"])),
        coverage_opacity: Cell::new(number(&v["opacity"])),
        cap_background: serde_json::from_value(v["cap"].clone()).unwrap(),
        cap_paint: serde_json::from_value(v["paint"].clone()).unwrap(),
    }))
}
fn triangles(v: &Value) -> Vec2<Triangle> {
    let mut input = v["mesh"].clone();
    for m in input["materials"].as_array_mut().unwrap() {
        m["vector"] = json!(false)
    }
    let mut ts = mesh_input(&input);
    for (i, t) in ts.iter_mut().enumerate() {
        t.shadow_ground = (!v["grounds"][i].is_null()).then(|| number(&v["grounds"][i]));
        if let Some(m) = &t.material {
            let mut m = m.borrow_mut();
            if m.vector.is_none() {
                m.vector = surface(&v["surfaces"][i])
            }
        }
    }
    ts
}
fn mesh_value(ts: &[Triangle]) -> Value {
    let mut safe = ts.to_vec();
    for t in &mut safe {
        t.shadow_ground = None
    }
    json!({"mesh":mesh(&safe),"grounds":ts.iter().map(|t|t.shadow_ground.map(float)).collect::<Vec2<_>>(),"surfaces":ts.iter().map(|t|t.material.as_ref().is_some_and(|m|m.borrow().vector.is_some())).collect::<Vec2<_>>()})
}
fn batches(v: &Value) -> Vec2<Batch> {
    let ts = triangles(&v["mesh"]);
    v["batches"]
        .as_array()
        .unwrap()
        .iter()
        .zip(ts)
        .map(|(b, t)| Batch {
            material: t.material.as_ref().unwrap().clone(),
            triangle: t,
            cover: None,
            polygons: polygons(&b["polygons"]),
            points: points(&b["points"]).try_into().unwrap(),
            depth: number(&b["depth"]),
            first: b["first"].as_u64().unwrap() as usize,
            affine: serde_json::from_value(b["affine"].clone()).unwrap(),
            texture: b["texture"].as_bool().unwrap(),
            color: b["color"].as_str().unwrap().into(),
        })
        .collect()
}
fn batches_value(bs: &[Batch]) -> Value {
    json!({"mesh":mesh_value(&bs.iter().map(|b|b.triangle.clone()).collect::<Vec2<_>>()),"batches":bs.iter().map(|b|json!({"polygons":polygons_value(&b.polygons),"points":points_value(&b.points),"depth":b.depth,"first":b.first,"affine":b.affine,"texture":b.texture,"color":b.color})).collect::<Vec2<_>>()})
}
fn writer<'a>(context: &'a RenderContext, prefix: &str) -> Writer<'a> {
    Writer {
        context,
        buf: prefix.into(),
        err: None,
        surfaces: HashMap::new(),
        surface_svg: &native_surface_svg,
    }
}
#[derive(Default)]
struct ReplayState {
    surfaces: HashMap<u64, Rc<VectorSurface>>,
    writers: HashMap<u64, (String, HashMap<usize, String>)>,
}
fn replay(v: &Value, state: &mut ReplayState) -> Value {
    let context = RenderContext::default();
    let op = v["op"].as_str().unwrap();
    match op {
        "ground" => {
            let ts = triangles(&v["mesh"]);
            json!({"mesh":mesh_value(&raster_ground_triangles(&ts,number(&v["ground"]),vector(&v["view"]))),"source":mesh_value(&ts)})
        }
        "receivers" => {
            let mut ts = triangles(&v["mesh"]);
            let bs = v["boards"]
                .as_array()
                .unwrap()
                .iter()
                .map(board)
                .collect::<Vec2<_>>();
            let spans = v["spans"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| HierarchyShadowSpan {
                    first: s["first"].as_u64().unwrap() as usize,
                    last: s["last"].as_u64().unwrap() as usize,
                    board: s["board"].as_str().unwrap().into(),
                })
                .collect::<Vec2<_>>();
            hierarchy_shadow_receivers(&mut ts, &bs, &spans);
            json!({"mesh":mesh_value(&ts)})
        }
        "snapshot" | "validate" => {
            let ts = triangles(&v["mesh"]);
            if op == "validate" {
                return json!({"error":super::super::raster::coverage_validate(&ts,&context).err().unwrap_or_default()});
            };
            let out = super::super::raster::snapshot_triangles(&ts, &context);
            {
                match out {
                    Ok(out) => json!({"mesh":mesh_value(&out),"source":mesh_value(&ts),"error":""}),
                    Err(e) => json!({"mesh":mesh_value(&[]),"source":mesh_value(&ts),"error":e}),
                }
            }
        }
        "expand" => points_value(&expand_cover_polygon(
            &points(&v["points"]),
            number(&v["width"]),
        )),
        "prepare" => {
            let bs = batches(&v["batches"]);
            let mut covers = vec![];
            let mapping = prepare_covers(&bs, &(0..bs.len()).collect::<Vec2<_>>(), &mut covers);
            let mut ids = BTreeMap::new();
            let out = (0..bs.len())
                .map(|b| {
                    mapping
                        .iter()
                        .find(|(i, _)| *i == b)
                        .map(|(_, c)| {
                            let next = ids.len() + 1;
                            *ids.entry(*c).or_insert(next)
                        })
                        .unwrap_or(0)
                })
                .collect::<Vec2<_>>();
            json!({"covers":out,"source":batches_value(&bs)})
        }
        "cover" => {
            let bs = batches(&v["batches"]);
            let mut covers = vec![Cover {
                batches: (0..bs.len()).collect(),
                polygons: bs.iter().flat_map(|b| b.polygons.clone()).collect(),
                first: bs[0].first,
                written: v["written"].as_bool().unwrap(),
            }];
            let mut w = writer(&context, v["prefix"].as_str().unwrap());
            write_cover(&mut w, &mut covers, 0, &bs, &camera(&v["camera"]));
            json!({"svg":w.buf,"error":w.err.unwrap_or_default(),"written":covers[0].written,"source":batches_value(&bs)})
        }
        "visibility" => {
            let fs = v["faces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| SvgVisibilityFace {
                    points: points(&f["points"]),
                    opaque: f["opaque"].as_bool().unwrap(),
                    order: f["order"].as_i64().unwrap(),
                    group: f["group"].as_u64().filter(|x| *x != 0).map(|x| x as usize),
                    owner: f["owner"].as_u64().filter(|x| *x != 0).map(|x| x as usize),
                    contour: f["contour"].as_bool().unwrap(),
                })
                .collect::<Vec2<_>>();
            match super::super::svg_visibility::svg_visible_faces(&fs) {
                Ok(ps) => {
                    json!({"visible":ps.iter().map(|p|polygons_value(p)).collect::<Vec2<_>>(),"error":""})
                }
                Err(e) => json!({"visible":[],"error":e}),
            }
        }
        "project" => {
            let p = camera(&v["camera"]).project(vector(&v["point"]));
            json!([p.x, p.y, p.z])
        }
        "links" => {
            let mut scene = NativeScene {
                camera: camera(&v["camera"]),
                ..Default::default()
            };
            scene.links = v["links"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| NativeLink {
                    points: l["points"].as_array().unwrap().iter().map(vector).collect(),
                    url: l["region"]["URL"].as_str().unwrap().into(),
                    tooltip: l["region"]["Tooltip"].as_str().unwrap().into(),
                    target: l["region"]["Target"].as_str().unwrap().into(),
                })
                .collect();
            let mut w = writer(&context, "");
            write_links(&mut w, &scene);
            json!({"svg":w.buf,"error":w.err.unwrap_or_default()})
        }
        "scene" | "shadow" => {
            let mut scene = NativeScene {
                triangles: triangles(&v["mesh"]),
                camera: camera(&v["camera"]),
                width: v["width"].as_i64().unwrap(),
                height: v["height"].as_i64().unwrap(),
                background: if v["background"].is_null() {
                    Default::default()
                } else {
                    serde_json::from_value(v["background"].clone()).unwrap()
                },
                ..Default::default()
            };
            if op == "scene" {
                match native_scene_svg(&mut scene, &native_surface_svg) {
                    Ok(svg) => json!({"svg":svg,"error":""}),
                    Err(e) => json!({"svg":"","error":e}),
                }
            } else {
                let mut w = writer(&context, "");
                write_ground_shadow(&mut w, &scene);
                json!({"svg":w.buf,"error":w.err.unwrap_or_default()})
            }
        }
        "definition" => {
            let id = v["surface"].as_u64().unwrap();
            if !v["document"].is_null() {
                assert!(
                    state
                        .surfaces
                        .insert(id, surface(&v["document"]).unwrap())
                        .is_none()
                )
            };
            let (buf, surfaces) = state
                .writers
                .remove(&v["writer"].as_u64().unwrap())
                .unwrap_or_default();
            let mut w = writer(&context, &buf);
            w.surfaces = surfaces;
            let name = w.surface_definition(&state.surfaces[&id]);
            let result = json!({"id":name,"svg":w.buf,"error":w.err.clone().unwrap_or_default()});
            state
                .writers
                .insert(v["writer"].as_u64().unwrap(), (w.buf, w.surfaces));
            result
        }
        "markdown" => json!(super::super::markdown_cards::native_markdown_card(&node(
            &v["node"]
        ))),
        _ => panic!("unknown operation {op}"),
    }
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var_os("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|| {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/coverage-api.json").to_owned()
        });
    let input: Value = serde_json::from_str(&source).unwrap();
    let actual = json!(
        input["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                let mut state = ReplayState::default();
                c["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|op| replay(&op["request"], &mut state))
                    .collect::<Vec2<_>>()
            })
            .collect::<Vec2<_>>()
    );
    if let Some(path) = std::env::var_os("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(path, serde_json::to_vec(&actual).unwrap()).unwrap()
    } else {
        let expected = json!(
            input["cases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|op| op["got"].clone())
                    .collect::<Vec2<_>>())
                .collect::<Vec2<_>>()
        );
        super::super::geometry_api_tests::assert_same(&expected, &actual)
    }
}

fn hydrate(v: &mut Value) {
    use base64::Engine;
    match v {
        Value::Object(o) => {
            if let Some(p) = o.remove("DataFile") {
                o.insert(
                    "Data".into(),
                    json!(
                        base64::engine::general_purpose::STANDARD
                            .encode(std::fs::read(p.as_str().unwrap()).unwrap())
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
