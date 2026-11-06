// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of original structured-node calls; compiled only in tests.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{
    geometry_api_tests::node,
    native_solids::solid_api_tests::{builder, state},
};
use super::*;
use serde_json::{Value, json};
fn number(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn surface(s: &LabelSurface) -> Value {
    json!({"center":s.center,"width":s.width,"depth":s.depth,"angle":s.angle,"align":s.align})
}
// Go serializes the concrete paint held by its interface. Preserve every field
// while expressing the Rust closed enum in that same typed wire shape.
fn run_value(run: &d2_rs_scene::TextRun) -> Value {
    let mut out = json!(run);
    out["Fill"] = match &run.fill {
        None => Value::Null,
        Some(d2_rs_scene::Paint::SolidPaint { color }) => json!({"Color":color}),
        Some(d2_rs_scene::Paint::LinearGradient(p)) => json!(p),
        Some(d2_rs_scene::Paint::RadialGradient(p)) => json!(p),
        Some(d2_rs_scene::Paint::PatternPaint(_)) => {
            panic!("pattern text paint requires a typed node bridge")
        }
    };
    assert!(
        run.stroke.is_none(),
        "stroked text requires a typed stroke bridge"
    );
    out
}
fn rich(b: &MeshBuilder, present: bool) -> Value {
    if !present {
        return Value::Null;
    }
    let p = &b.rich;
    json!({"count":p.count,"used":p.used,"pixels":p.pixels,"bytes":p.source_bytes,"rows":p.rows,"primary":p.primary,"mono":p.mono,"theme":p.theme_id,"overrides":p.theme_overrides})
}
fn result(b: &MeshBuilder, n: &Node, present: bool) -> Value {
    json!({"state":state(b),"rich":rich(b,present),"node":n})
}
fn edge_value(v: &Value) -> crate::scene::Edge {
    super::super::native_solids::solid_api_tests::edge(v)
}
fn replay(v: &Value) -> Value {
    let n = if v["node"].is_null() {
        Node::default()
    } else {
        node(&v["node"])
    };
    let op = v["op"].as_str().unwrap();
    if op == "icon" {
        return surface(&native_structured_icon_surface(&n, number(&v["y"])));
    }
    if op == "caption" {
        return super::super::sequence_routes::caption_api_value(
            &edge_value(&v["edge"]),
            number(&v["scale"]),
        );
    }
    if op == "routes" {
        let token = d2_rs_core::CancellationToken::new();
        if v["canceled"] == true {
            token.cancel()
        }
        let ctx = super::super::context::RenderContext::from_token(&token);
        let es = v["edges"]
            .as_array()
            .map(|a| a.iter().map(edge_value).collect::<Vec<_>>())
            .unwrap_or_default();
        let ns = v["nodes"]
            .as_array()
            .map(|a| a.iter().map(node).collect::<Vec<_>>())
            .unwrap_or_default();
        let bs = v["boards"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(super::super::geometry_api_tests::board)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let supports = v["supports"]
            .as_array()
            .and_then(|a| a.first())
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()).unwrap())
            .unwrap_or_default();
        return match super::super::routes::native_edge_routes_with_context(
            &es, &ns, &bs, &supports, &ctx,
        ) {
            Ok((lanes, paths)) => json!({"lanes":lanes,"paths":paths,"error":""}),
            Err(e) => json!({"lanes":null,"paths":null,"error":e}),
        };
    }
    let mut b = builder(&v["state"]);
    let token = d2_rs_core::CancellationToken::new();
    if v["canceled"] == true {
        token.cancel()
    }
    b.context = super::super::context::RenderContext::from_token(&token);
    b.arrow_background = v["background"].as_str().unwrap().into();
    if v["text"] == true {
        b.text = super::super::text::TextPainter::new(1, None).unwrap();
    }
    assert_eq!(v["vector"], false);
    let p = &v["rich"];
    let present = !p.is_null();
    if present {
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
    if op == "edges" {
        use super::super::{native_solids::solid_api_tests::mesh, route_labels::contact_api_tests};
        let es = v["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(edge_value)
            .collect::<Vec<_>>();
        let mut p = contact_api_tests::placer(&v["placer"]);
        let mut scene = crate::scene::Scene::default();
        if let Some(s) = v["scenes"].as_array().and_then(|a| a.first()) {
            scene.background = s["background"].as_str().unwrap().into();
            scene.theme_id = s["themeId"].as_i64().unwrap();
            scene.theme_overrides = serde_json::from_value(s["themeOverrides"].clone()).unwrap();
            assert!(s["nodes"].is_null());
            assert!(s["boards"].is_null());
        }
        let packets = b.edges(&es, &mut p, &scene);
        let ps=packets.iter().map(|v|json!({"points":v.points,"lengths":v.lengths,"material":mesh(&[super::super::mesh::Triangle{material:v.material.clone(),..Default::default()}]),"forward":v.forward,"reverse":v.reverse})).collect::<Vec<_>>();
        return json!({"state":state(&b),"rich":rich(&b,present),"edges":es,"placer":contact_api_tests::state(&p),"packets":ps});
    }
    match op {
        "hierarchy" => b.hierarchy_node(&n, v["tint"][0].as_str().unwrap()),
        "structured" => b.structured_node(&n),
        "rail" => b.structured_rail(
            &n,
            number(&v["z"]),
            number(&v["depth"]),
            number(&v["bottom"]),
            number(&v["top"]),
            v["fill"].as_str().unwrap(),
        ),
        "document" => {
            let (cells, viewport, error) = match b.structured_document(&n) {
                Ok((d, cs)) => (
                    cs.iter()
                        .map(|(n, row)| {
                            let Some(Primitive::TextRun(run)) = &n.primitive else {
                                panic!("not text")
                            };
                            json!({"id":n.id,"row":row,"run":run_value(run)})
                        })
                        .collect::<Vec<_>>(),
                    json!({"box":d.view_box,"width":d.logical_width,"height":d.logical_height,"fit":d.viewport_fit,"align":d.viewport_align}),
                    String::new(),
                ),
                Err(e) => (vec![], Value::Null, e),
            };
            return json!({"result":result(&b,&n,present),"error":error,"cells":cells,"viewport":viewport});
        }
        _ => panic!("unknown operation {op}"),
    }
    result(&b, &n, present)
}
#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let source = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_else(|_| {
            d2_rs_testdata::oracle_fixture!("tests/fixtures/structured-api.json").into()
        });
    let cases: Value = serde_json::from_str(&source).unwrap();
    let output: Vec<_> = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| replay(&o["request"]))
                .collect::<Vec<_>>()
        })
        .collect();
    if let Ok(p) = std::env::var("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(p, serde_json::to_vec(&output).unwrap()).unwrap()
    } else {
        let expected: Vec<_> = cases["cases"]
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
            .collect();
        super::super::geometry_api_tests::assert_same(&json!(expected), &json!(output))
    }
}
