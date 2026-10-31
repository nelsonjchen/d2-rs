// SPDX-License-Identifier: MPL-2.0
//! Original remaining API calls. Input-only test transport, never compiled in production.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{
    framing::NativeSceneOptions,
    geometry_api_tests::node,
    native_solids::solid_api_tests as solid,
    raster::api_tests as raster,
    render::{Options, build_native_scene_with_options},
};
use super::composition_api_tests as composition;
use super::*;
use crate::scene::{RenderOpts, Scene};
use serde_json::{Value, json};
use std::path::Path;
type List<T> = std::vec::Vec<T>;
fn packets(ps: &[super::super::geometry_routes::PacketRoute]) -> Value {
    json!(ps.iter().map(|p|json!({"points":p.points,"lengths":p.lengths,"material":solid::mesh(&[Triangle{material:p.material.clone(),..Default::default()}]),"forward":p.forward,"reverse":p.reverse})).collect::<List<_>>())
}
fn camera(c: super::super::mesh::Camera) -> Value {
    let a = |v: Vec| [v.x, v.y, v.z];
    json!({"right":a(c.right),"up":a(c.up),"direction":a(c.direction),"center_x":c.center_x,"center_y":c.center_y,"center_depth":c.center_depth,"scale":c.scale,"width":c.width,"height":c.height})
}
fn native(v: &Value, images: &Path) -> Value {
    let target: crate::target::Diagram = serde_json::from_value(v["target"].clone()).unwrap();
    let o = &v["render"];
    let render = RenderOpts {
        theme_id: o["ThemeID"].as_i64(),
        theme_overrides: serde_json::from_value(o["ThemeOverrides"].clone()).unwrap(),
    };
    let scene = crate::scene::build_scene(&target, Some(&render)).unwrap();
    let result = build_native_scene_with_options(
        &scene,
        v["width"].as_i64().unwrap(),
        v["height"].as_i64().unwrap(),
        &Options::default(),
        NativeSceneOptions {
            fit_content: v["fit"].as_bool().unwrap(),
            defer_raster: v["defer"].as_bool().unwrap(),
            vector: v["vector"].as_bool().unwrap(),
            retain_vector_surfaces: v["retain_vectors"].as_bool().unwrap(),
            output_density: v["density"].as_f64().unwrap(),
            ..Default::default()
        },
        &Default::default(),
    );
    if v["op"] == "native-frame" {
        return match result {
            Ok(n) => {
                let capture = super::super::gif_capture::Capture {
                    context: Default::default(),
                    scene: n,
                };
                let at = v["at"].as_f64().unwrap();
                match capture.frame_image_at(
                    at,
                    v["packet"]
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(Value::as_f64)
                        .unwrap_or(at),
                    v["traffic"].as_bool().unwrap(),
                ) {
                    Ok(p) => json!({"error":"","frame":raster::frame(p,images)}),
                    Err(e) => json!({"error":e.to_string(),"frame":null}),
                }
            }
            Err(e) => json!({"error":e,"frame":null}),
        };
    }
    let mut out = json!({"error":"","scene":scene,"target":target,"mesh":null,"camera":null,"packets":null,"background":null});
    match result {
        Ok(n) => {
            out["mesh"] = solid::mesh(&n.triangles);
            out["camera"] = camera(n.camera);
            out["packets"] = packets(&n.packets);
            out["background"] = json!(n.background);
        }
        Err(e) => out["error"] = json!(e),
    }
    out
}
fn replay(v: &Value, images: &Path) -> Value {
    match v["op"].as_str().unwrap() {
        "native" | "native-frame" => return native(v, images),
        "shadow" | "validate" | "draw" => return raster::replay_residual(v, images),
        "snapshot" => {
            use base64::Engine;
            use sha2::{Digest, Sha256};
            let mut out = raster::replay(v, images);
            for triangle in out["triangles"].as_array_mut().unwrap() {
                if let Some(texture) = triangle["material"]["texture"].as_object_mut() {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(texture.remove("pixels").unwrap().as_str().unwrap())
                        .unwrap();
                    texture.insert(
                        "sha256".into(),
                        json!(format!("{:x}", Sha256::digest(&bytes))),
                    );
                    texture.insert("bytes".into(), json!(bytes.len()));
                }
            }
            return out;
        }
        "render" => return raster::replay(v, images),
        "knockout" => {
            let mut b = composition::builder(v);
            let s = &v["surface"];
            let c = &s["center"];
            let surface = super::super::geometry::LabelSurface {
                center: nv(
                    c["x"].as_f64().unwrap(),
                    c["y"].as_f64().unwrap(),
                    c["z"].as_f64().unwrap(),
                ),
                width: s["width"].as_f64().unwrap(),
                depth: s["depth"].as_f64().unwrap(),
                angle: s["angle"].as_f64().unwrap(),
                align: s["align"].as_str().unwrap().into(),
            };
            b.route_caption_knockout(v["first"].as_u64().unwrap() as usize, &surface);
            return solid::state(&b);
        }
        "edges" => {}
        _ => return composition::replay(v, images),
    }
    let mut b = composition::builder(v);
    if let Some(count) = v["text_count"].as_i64() {
        b.text = super::super::text::TextPainter::new(count, None).unwrap();
    }
    if v["icons"] == true {
        b.icons = super::super::surface_icons::SurfaceIconPainter::new(1, None, None).unwrap();
    }
    let edges: List<_> = v["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(solid::edge)
        .collect();
    let mut scene = Scene::default();
    if let Some(s) = v["scenes"].as_array().and_then(|a| a.first()) {
        scene.nodes = s["nodes"]
            .as_array()
            .map(|a| a.iter().map(node).collect())
            .unwrap_or_default();
        scene.theme_id = s["themeId"].as_i64().unwrap_or_default();
        scene.theme_overrides = serde_json::from_value(s["themeOverrides"].clone()).unwrap();
        assert!(s["boards"].as_array().is_none_or(|a| a.is_empty()));
    }
    let ps = b.edges(
        &edges,
        &mut super::super::route_labels::RouteCaptionPlacer::default(),
        &scene,
    );
    let links = std::mem::take(&mut b.links);
    let state = solid::state(&b);
    json!({"state":state,"packets":packets(&ps),"edges":edges,"links":links.iter().map(|l|json!({"points":l.points,"region":{"url":l.url,"tooltip":l.tooltip,"target":l.target}})).collect::<List<_>>()})
}
#[test]
fn vector_documents_follow_context_independently_of_cap_coverage() {
    let target: crate::target::Diagram = serde_json::from_value(json!({
        "shapes": [{"id":"a", "type":"rectangle", "pos":{"x":0,"y":0},
            "width":120, "height":80, "opacity":1, "strokeWidth":2,
            "fill":"#ffffff", "stroke":"#123456", "label":"label",
            "fontSize":16, "labelWidth":40, "labelHeight":20}],
        "connections": []
    }))
    .unwrap();
    let scene = crate::scene::build_scene(&target, None).unwrap();
    for coverage in [false, true] {
        for retain in [false, true] {
            let native = build_native_scene_with_options(
                &scene,
                320,
                300,
                &Options::default(),
                NativeSceneOptions {
                    vector: coverage,
                    retain_vector_surfaces: retain,
                    defer_raster: true,
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                native.triangles.iter().any(|t| t
                    .material
                    .as_ref()
                    .is_some_and(|m| m.borrow().vector.is_some())),
                retain,
                "coverage={coverage}, retain={retain}"
            );
        }
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn contact_caption_mesh_matches_original_go() {
    let cases: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/contact-caption-api.json"))
    .unwrap();
    for case in cases["cases"].as_array().unwrap() {
        for operation in case["operations"].as_array().unwrap() {
            let actual = replay(&operation["request"], Path::new("."));
            super::super::geometry_api_tests::assert_same(&operation["got"], &actual);
        }
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
    let actual: List<_> = cases["cases"]
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
    std::fs::write(output, serde_json::to_vec(&actual).unwrap()).unwrap();
}
