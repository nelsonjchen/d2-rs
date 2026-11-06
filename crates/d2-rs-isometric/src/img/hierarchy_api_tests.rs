// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of original hierarchy and SVG-lighting calls.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{
    geometry::{LabelSurface, MeshBuilder},
    geometry_api_tests::{board, node},
    hierarchy_boards::*,
    hierarchy_headers::hierarchy_board_header_placement,
    hierarchy_support::*,
    native_solids::solid_api_tests::{builder, mesh_input, state},
};
use super::*;
use serde_json::{Value, json};
use std::collections::HashMap;
fn num(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn vector(v: &Value) -> Vec {
    nv(num(&v["x"]), num(&v["y"]), num(&v["z"]))
}
fn nodes(v: &Value) -> Vec2<crate::scene::Node> {
    v.as_array()
        .map(|a| a.iter().map(node).collect())
        .unwrap_or_default()
}
fn boards(v: &Value) -> Vec2<crate::scene::Board> {
    v.as_array()
        .map(|a| a.iter().map(board).collect())
        .unwrap_or_default()
}
// Go distinguishes nil and allocated empty node-ID slices. Rust's Board owns
// a Vec; retain the input nil marker when serializing an unchanged empty slice.
// Nonempty native results always serialize their actual complete values.
fn board_value(b: &crate::scene::Board, source: &Value) -> Value {
    let mut out = json!(b);
    if b.node_ids.is_empty() && source["nodeIds"].is_null() {
        out["nodeIds"] = Value::Null;
    }
    out
}
fn boards_value(bs: &[crate::scene::Board], source: &Value) -> Value {
    assert_eq!(bs.len(), source.as_array().unwrap().len());
    json!(
        bs.iter()
            .zip(source.as_array().unwrap())
            .map(|(b, s)| board_value(b, s))
            .collect::<Vec2<_>>()
    )
}
fn surface(v: &Value) -> LabelSurface {
    LabelSurface {
        center: vector(&v["center"]),
        width: num(&v["width"]),
        depth: num(&v["depth"]),
        angle: num(&v["angle"]),
        align: v["align"].as_str().unwrap().into(),
    }
}
fn surface_value(s: &LabelSurface) -> Value {
    json!({"center":s.center,"width":s.width,"depth":s.depth,"angle":s.angle,"align":s.align})
}
fn rich(b: &MeshBuilder, present: bool) -> Value {
    if !present {
        return Value::Null;
    }
    let p = &b.rich;
    json!({"count":p.count,"used":p.used,"pixels":p.pixels,"bytes":p.source_bytes,"rows":p.rows,"primary":p.primary,"mono":p.mono,"theme":p.theme_id,"overrides":p.theme_overrides})
}
fn builder_value(b: &MeshBuilder, request: &Value) -> Value {
    json!({"state":state(b),"rich":rich(b,!request["rich"].is_null()),"supports":if request["supports"].is_null(){Value::Null}else{json!(b.hierarchy_supports)},"drop":b.node_support_drop})
}
fn camera(v: &Value) -> Camera {
    Camera {
        right: vector(&v["right"]),
        up: vector(&v["up"]),
        direction: vector(&v["direction"]),
        center_x: num(&v["centerX"]),
        center_y: num(&v["centerY"]),
        center_depth: num(&v["centerDepth"]),
        scale: num(&v["scale"]),
        width: v["width"].as_i64().unwrap(),
        height: v["height"].as_i64().unwrap(),
    }
}
fn point(v: &Value) -> SvgPoint {
    SvgPoint {
        x: num(&v[0]),
        y: num(&v[1]),
        z: num(&v[2]),
    }
}
fn point_value(p: SvgPoint) -> Value {
    json!([p.x, p.y, p.z])
}
fn vertex(v: &Value) -> LightingVertex {
    LightingVertex {
        point: point(&v["point"]),
        position: vector(&v["position"]),
        normal: vector(&v["normal"]),
    }
}
fn vertices(v: &Value) -> [LightingVertex; 3] {
    [vertex(&v[0]), vertex(&v[1]), vertex(&v[2])]
}
fn vertices_value(v: &[LightingVertex; 3]) -> Value {
    json!(
        v.iter()
            .map(|v| json!({"point":point_value(v.point),"position":v.position,"normal":v.normal}))
            .collect::<Vec2<_>>()
    )
}
fn gradient_value(g: &Gradient) -> Value {
    json!({"start":point_value(g.start),"end":point_value(g.end),"values":g.values,"valid":g.valid,"stops":g.stops.iter().map(|(o,c)|json!({"offset":o,"color":c})).collect::<Vec2<_>>()})
}
fn paint(v: &Value) -> RasterPaint {
    RasterPaint {
        base: vector(&v["base"]),
        emission: vector(&v["emission"]),
        roughness: num(&v["roughness"]),
        metalness: num(&v["metalness"]),
        unlit: v["unlit"].as_bool().unwrap(),
        alpha: num(&v["alpha"]),
        texture: false,
        multiply: v["multiply"].as_bool().unwrap(),
    }
}
pub(crate) fn replay(v: &Value) -> Value {
    let op = v["op"].as_str().unwrap();
    match op {
        "boards" | "header-ink" => {
            let ns: HashMap<String, crate::scene::Node> = v["nodes"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), node(v)))
                .collect();
            let index = ns.iter().map(|(k, n)| (k.as_str(), n)).collect();
            if op == "boards" {
                let bs = boards(&v["boards"]);
                return json!({"boards":boards_value(&hierarchy_presentation_boards(&bs,&index),&v["boards"]),"source":boards_value(&bs,&v["boards"]),"nodes":ns});
            }
            let bs: HashMap<String, crate::scene::Board> = v["boards"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), board(v)))
                .collect();
            return json!(hierarchy_header_ink(
                &board(&v["board"]),
                &bs,
                &index,
                &serde_json::from_value(v["tints"].clone()).unwrap(),
                v["background"].as_str().unwrap()
            ));
        }
        "render-nodes" => {
            let ns = nodes(&v["nodes"]);
            let supports: Option<HashMap<String, f64>> = v["supports"]
                .as_array()
                .and_then(|a| a.first())
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()).unwrap());
            return json!({"nodes":hierarchy_render_nodes(&ns,supports.as_ref()),"source":ns,"supports":v["supports"]});
        }
        "supports" => return json!(hierarchy_support_offsets(&boards(&v["boards"]))),
        "tier" => return json!(hierarchy_tier_height(v["depth"].as_i64().unwrap())),
        "spacer" => return json!(hierarchy_spacer(&node(&v["node"]))),
        "relief" => return json!(hierarchy_node_relief(&node(&v["node"]))),
        "header" | "placement" => {
            let bs = v["boards"].as_array().and_then(|a| a.first()).map(boards);
            let n = node(&v["owner"]);
            let b = board(&v["board"]);
            let ns = nodes(&v["nodes"]);
            let (s, f) = hierarchy_board_header_placement(
                surface(&v["surface"]),
                &b,
                &n,
                &ns,
                num(&v["scale"]),
                bs.as_deref(),
            );
            return json!({"surface":surface_value(&s),"fits":if op=="placement"{json!(f)}else{Value::Null},"board":board_value(&b,&v["board"]),"owner":n,"nodes":ns});
        }
        "texture-lighting" | "caster" | "ground-caster" => {
            let ts = mesh_input(&v["mesh"]);
            let t = &ts[0];
            return match op {
                "caster" => json!(caster_opacity(t)),
                "ground-caster" => json!(
                    ground_caster(
                        t,
                        num(&v["ground"]),
                        num(&v["receiver"]),
                        &camera(&v["camera"]),
                        vector(&v["light"])
                    )
                    .into_iter()
                    .map(point_value)
                    .collect::<Vec2<_>>()
                ),
                _ => {
                    let m = t.material.as_ref().unwrap().borrow();
                    let (a, b) = texture_lighting(&camera(&v["camera"]), t, &m);
                    json!({"slope":a,"intercept":b})
                }
            };
        }
        "gradient" | "patches" => {
            let c = camera(&v["camera"]);
            let p = paint(&v["paint"]);
            let vs = vertices(&v["vertices"]);
            return if op == "gradient" {
                let (g, e) = fit_lighting_gradient(&c, &p, &vs);
                json!({"gradient":gradient_value(&g),"error":e})
            } else {
                json!(lighting_patches(&c,&p,vs).iter().map(|(vs,g)|json!({"vertices":vertices_value(vs),"gradient":gradient_value(g)})).collect::<Vec2<_>>())
            };
        }
        "shadow" => {
            let s = NativeScene {
                triangles: mesh_input(&v["mesh"]),
                camera: camera(&v["camera"]),
                width: v["width"].as_i64().unwrap(),
                height: v["height"].as_i64().unwrap(),
                ..Default::default()
            };
            let context = super::super::context::RenderContext::default();
            let mut w = Writer {
                context: &context,
                buf: v["prefix"].as_str().unwrap().into(),
                err: None,
                surfaces: HashMap::new(),
                surface_svg: &super::super::vector_surface_svg::native_surface_svg,
            };
            write_ground_shadow(&mut w, &s);
            return json!({"svg":w.buf,"error":w.err.unwrap_or_default()});
        }
        _ => {}
    }
    let mut b = builder(&v["state"]);
    let token = d2_rs_core::CancellationToken::new();
    if v["canceled"] == true {
        token.cancel()
    }
    b.context = super::super::context::RenderContext::from_token(&token);
    b.options.vector = v["vector"].as_bool().unwrap();
    if v["retainVector"] == true {
        b.vectors = Some(Rc::new(std::cell::RefCell::new(Default::default())))
    }
    if v["text"] == true {
        b.text = super::super::text::TextPainter::new(1, None).unwrap()
    }
    b.arrow_background = v["background"].as_str().unwrap().into();
    b.node_support_drop = num(&v["drop"]);
    b.hierarchy_supports = if v["supports"].is_null() {
        HashMap::new()
    } else {
        serde_json::from_value(v["supports"].clone()).unwrap()
    };
    let p = &v["rich"];
    if !p.is_null() {
        b.rich.count = p["count"].as_i64().unwrap();
        b.rich.used = p["used"].as_i64().unwrap();
        b.rich.pixels = p["pixels"].as_i64().unwrap();
        b.rich.source_bytes = p["bytes"].as_u64().unwrap() as usize;
        b.rich.rows = p["rows"].as_u64().unwrap() as usize;
        b.rich.primary = p["primary"].as_str().unwrap().into();
        b.rich.mono = p["mono"].as_str().unwrap().into();
        b.rich.theme_id = p["theme"].as_i64();
        b.rich.theme_overrides = serde_json::from_value(p["overrides"].clone()).unwrap()
    }
    let n = (!v["node"].is_null()).then(|| node(&v["node"]));
    let tint = v["tint"].as_str().unwrap();
    match op {
        "board" => {
            b.hierarchy_board(&board(&v["board"]), n.as_ref(), tint, num(&v["opacity"]));
            builder_value(&b, v)
        }
        "node" | "hierarchy" => {
            let n = n.unwrap();
            if op == "node" {
                b.node(&n, tint, None)
            } else {
                b.hierarchy_node(&n, tint)
            }
            json!({"builder":builder_value(&b,v),"node":n})
        }
        _ => panic!("unknown {op}"),
    }
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|_| {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/hierarchy-api.json").into()
        });
    let cases: Value = serde_json::from_str(&source).unwrap();
    let cs = cases["cases"].as_array().unwrap();
    assert!(!cs.is_empty());
    let output: Vec2<_> = cs
        .iter()
        .map(|c| {
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| replay(&o["request"]))
                .collect::<Vec2<_>>()
        })
        .collect();
    if let Ok(p) = std::env::var("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(p, serde_json::to_vec(&output).unwrap()).unwrap()
    } else {
        let expected: Vec2<_> = cs
            .iter()
            .map(|c| {
                c["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| o["got"].clone())
                    .collect::<Vec2<_>>()
            })
            .collect();
        super::super::geometry_api_tests::assert_same(&json!(expected), &json!(output))
    }
}
