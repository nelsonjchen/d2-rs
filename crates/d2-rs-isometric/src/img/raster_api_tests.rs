// SPDX-License-Identifier: MPL-2.0
//! Test-only transport for original raster requests. No JSON code enters native builds.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::*;
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, collections::HashMap, path::Path};
fn number(v: &Value) -> f64 {
    match v.as_str() {
        Some("NaN") => f64::NAN,
        Some("+Inf") => f64::INFINITY,
        Some("-Inf") => f64::NEG_INFINITY,
        _ => v.as_f64().unwrap(),
    }
}
fn vector(v: &Value) -> Vec {
    nv(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn flag(v: &Value) -> bool {
    v.as_bool().unwrap_or(false)
}
fn integer(v: &Value) -> i64 {
    v.as_i64().unwrap()
}
fn color(v: &Value) -> Nrgba {
    serde_json::from_value(v.clone()).unwrap()
}
fn texture(v: &Value) -> Result<Rc<Texture>, String> {
    if flag(&v["nil"]) {
        return Err("Go typed nil image has no native texture representation".into());
    }
    let r = &v["rect"];
    let (w, h) = (
        integer(&r[2]) - integer(&r[0]),
        integer(&r[3]) - integer(&r[1]),
    );
    let format = match v["kind"].as_str().unwrap() {
        "rgba" => TextureFormat::Rgba,
        "nrgba" => TextureFormat::Nrgba,
        "alpha" => TextureFormat::Alpha,
        _ => return Err("unhandled image format".into()),
    };
    Ok(Rc::new(Texture::with_storage(
        w as usize,
        h as usize,
        integer(&v["stride"]),
        format,
        if let Some(path) = v["pixels_file"].as_str() {
            std::fs::read(path).unwrap()
        } else {
            base64::engine::general_purpose::STANDARD
                .decode(v["pixels"].as_str().unwrap())
                .unwrap()
        },
    )))
}
fn triangles(v: &Value) -> Result<std::vec::Vec<Triangle>, String> {
    let mut materials = HashMap::new();
    let mut groups = HashMap::new();
    let mut owners = HashMap::new();
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let m = &v["material"];
            let paint = if m.is_null() {
                None
            } else {
                let id = integer(&v["material_id"]);
                if let std::collections::hash_map::Entry::Vacant(e) = materials.entry(id) {
                    e.insert(material(Material {
                        color: color(&m["color"]),
                        emissive: color(&m["emissive"]),
                        roughness: number(&m["roughness"]),
                        metalness: number(&m["metalness"]),
                        unlit: flag(&m["unlit"]),
                        multiply: flag(&m["multiply"]),
                        svg_contour: flag(&m["contour"]),
                        svg_solid_texture: flag(&m["solid"]),
                        texture: if m["texture"].is_null() {
                            None
                        } else {
                            Some(texture(&m["texture"])?)
                        },
                        ..Default::default()
                    }));
                }
                Some(materials[&id].clone())
            };
            let vs = v["vertices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Vertex {
                    position: vector(&v["position"]),
                    normal: vector(&v["normal"]),
                    u: number(&v["u"]),
                    v: number(&v["v"]),
                })
                .collect::<std::vec::Vec<_>>();
            let g = &v["group"];
            let group = if g.is_null() {
                None
            } else {
                Some(
                    groups
                        .entry(integer(&g["id"]))
                        .or_insert_with(|| {
                            Rc::new(RefCell::new(OpacityGroup {
                                opacity: number(&g["opacity"]),
                            }))
                        })
                        .clone(),
                )
            };
            let o = &v["owner"];
            let owner = if o.is_null() {
                None
            } else {
                Some(
                    owners
                        .entry(integer(&o["id"]))
                        .or_insert_with(|| {
                            Rc::new(RefCell::new(PaintOwner {
                                opaque: flag(&o["opaque"]),
                            }))
                        })
                        .clone(),
                )
            };
            Ok(Triangle {
                v: vs.try_into().unwrap(),
                material: paint,
                cast_shadow: flag(&v["cast_shadow"]),
                shadow_opacity: (!v["shadow_opacity"].is_null())
                    .then(|| number(&v["shadow_opacity"])),
                shadow_fill_alpha: v["shadow_fill_alpha"].as_u64().map(|v| v as u8),
                shadow_ground: v["shadow_ground"].as_f64(),
                depth_bias: number(&v["depth_bias"]),
                no_depth_write: flag(&v["no_depth_write"]),
                opacity_group: group,
                paint_owner: owner,
                svg_coverage_only: flag(&v["coverage"]),
            })
        })
        .collect()
}
fn texture_value(t: &Texture) -> Value {
    json!({"kind":match t.format{TextureFormat::Rgba=>"rgba",TextureFormat::Nrgba=>"nrgba",TextureFormat::Alpha=>"alpha"},"rect":[0,0,t.width,t.height],"stride":t.stride.unwrap_or((t.width*t.channels())as i64),"pixels":base64::engine::general_purpose::STANDARD.encode(&*t.pixels.borrow()),"nil":false})
}
fn triangles_value(ts: &[Triangle]) -> Value {
    let mut materials = HashMap::new();
    let mut groups = HashMap::new();
    let mut owners = HashMap::new();
    json!(ts.iter().map(|t|{
  let id=t.material.as_ref().map(|m|{let next=materials.len()+1;*materials.entry(Rc::as_ptr(m)as usize).or_insert(next)}).unwrap_or(0);
  let m=t.material.as_ref().map(|m|{let m=m.borrow();json!({"color":m.color,"emissive":m.emissive,"roughness":m.roughness,"metalness":m.metalness,"texture":m.texture.as_deref().map(texture_value),"unlit":m.unlit,"multiply":m.multiply,"contour":m.svg_contour,"solid":m.svg_solid_texture})});
  let g=t.opacity_group.as_ref().map(|g|{let next=groups.len()+1;json!({"id":*groups.entry(Rc::as_ptr(g)as usize).or_insert(next),"opacity":g.borrow().opacity})});
  let o=t.paint_owner.as_ref().map(|o|{let next=owners.len()+1;json!({"id":*owners.entry(Rc::as_ptr(o)as usize).or_insert(next),"opaque":o.borrow().opaque})});
  json!({"vertices":t.v.iter().map(|v|json!({"position":[v.position.x,v.position.y,v.position.z],"normal":[v.normal.x,v.normal.y,v.normal.z],"u":v.u,"v":v.v})).collect::<std::vec::Vec<_>>(),"material":m,"material_id":id,"cast_shadow":t.cast_shadow,"shadow_opacity":t.shadow_opacity,"shadow_fill_alpha":t.shadow_fill_alpha,"shadow_ground":t.shadow_ground,"depth_bias":t.depth_bias,"no_depth_write":t.no_depth_write,"group":g,"owner":o,"coverage":t.svg_coverage_only})
 }).collect::<std::vec::Vec<_>>())
}
fn camera(v: &Value) -> Option<Camera> {
    (!v.is_null()).then(|| Camera {
        right: vector(&v["right"]),
        up: vector(&v["up"]),
        direction: vector(&v["direction"]),
        center_x: number(&v["center_x"]),
        center_y: number(&v["center_y"]),
        center_depth: number(&v["center_depth"]),
        scale: number(&v["scale"]),
        width: integer(&v["width"]),
        height: integer(&v["height"]),
    })
}
fn context(v: &Value) -> super::super::context::RenderContext {
    let token = d2_rs_core::CancellationToken::new();
    if flag(&v["canceled"]) {
        token.cancel()
    }
    super::super::context::RenderContext::from_token(&token)
}
pub(crate) fn frame(p: d2_rs_raster::NrgbaImage, directory: &Path) -> Value {
    let digest = format!("{:x}", Sha256::digest(&p.pix));
    let path = directory.join(format!("{digest}.rgba"));
    if !path.exists() {
        std::fs::write(path, &p.pix).unwrap()
    }
    json!({"width":p.rect.dx(),"height":p.rect.dy(),"stride":p.stride,"sha256":digest})
}
pub(crate) fn replay(v: &Value, directory: &Path) -> Value {
    match v["op"].as_str().unwrap() {
        "render" => {
            let ts = match triangles(&v["triangles"]) {
                Ok(v) => v,
                Err(e) => return json!({"unsupported":e}),
            };
            let dynamic = match triangles(&v["dynamic"]) {
                Ok(v) => v,
                Err(e) => return json!({"unsupported":e}),
            };
            match render_mesh_frame(
                integer(&v["width"]),
                integer(&v["height"]),
                &ts,
                &dynamic,
                color(&v["background"]),
                camera(&v["camera"]).as_ref(),
                None,
                camera(&v["shadow_camera"]).as_ref(),
                camera(&v["ground_camera"]).as_ref(),
                &context(v),
            ) {
                Ok(p) => json!({"error":"","frame":frame(p,directory)}),
                Err(e) => json!({"error":e,"frame":null}),
            }
        }
        "sample" => match texture(&v["texture"]) {
            Ok(t) => json!({"error":"","rgba":texture_sample(&t,number(&v["u"]),number(&v["v"]))}),
            Err(e) => json!({"unsupported":e}),
        },
        "snapshot" => {
            let ts = match triangles(&v["triangles"]) {
                Ok(v) => v,
                Err(e) => return json!({"unsupported":e}),
            };
            match snapshot_triangles(&ts, &context(v)) {
                Ok(ts) => json!({"error":"","triangles":triangles_value(&ts)}),
                Err(e) => json!({"error":e,"triangles":[]}),
            }
        }
        "multiply" => {
            let mut pixel = v["pixel"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect::<std::vec::Vec<_>>();
            multiply_over(
                &mut pixel,
                number(&v["rgb"][0]),
                number(&v["rgb"][1]),
                number(&v["rgb"][2]),
                number(&v["alpha"]),
            );
            json!({"error":"","pixel":pixel})
        }
        "span" => {
            let p: [RasterPoint; 3] = v["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| RasterPoint {
                    x: number(&v[0]),
                    y: number(&v[1]),
                    z: number(&v[2]),
                })
                .collect::<std::vec::Vec<_>>()
                .try_into()
                .unwrap();
            let (a, b) = row_span(p, integer(&v["y"]), integer(&v["x0"]), integer(&v["x1"]));
            json!({"error":"","span":[a,b]})
        }
        "charge" => {
            let mut work = Work {
                remaining: integer(&v["remaining"]),
                context: context(v),
                ..Work::new()
            };
            let error = work.charge(integer(&v["n"])).err().unwrap_or_default();
            json!({"error":error,"remaining":work.remaining})
        }
        _ => panic!("unhandled raster request"),
    }
}
#[test]
#[ignore = "requires an external original Go capture"]
fn replay_original_raster_calls() {
    let input = std::env::var("D2RS_RASTER_REQUEST").unwrap();
    let directory = std::path::PathBuf::from(std::env::var("D2RS_RASTER_OUTPUT").unwrap());
    std::fs::create_dir_all(directory.join("images")).unwrap();
    let request: Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    for (i, c) in request["cases"].as_array().unwrap().iter().enumerate() {
        let out = c["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| replay(v, &directory.join("images")))
            .collect::<std::vec::Vec<_>>();
        std::fs::write(
            directory.join(format!("{i}.json")),
            serde_json::to_vec(&out).unwrap(),
        )
        .unwrap()
    }
}

#[test]
fn snapshot_owns_inputs_and_preserves_internal_sharing() {
    let texture = Rc::new(Texture::with_storage(
        2,
        2,
        12,
        TextureFormat::Nrgba,
        vec![
            101, 73, 202, 127, 0, 0, 0, 0, 99, 99, 99, 99, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    ));
    let first = material(Material {
        texture: Some(texture.clone()),
        color: Nrgba::new(255, 255, 255, 255),
        ..Default::default()
    });
    let second = material(first.borrow().clone());
    let group = Rc::new(RefCell::new(OpacityGroup { opacity: 0.5 }));
    let owner = Rc::new(RefCell::new(PaintOwner { opaque: true }));
    let mut source = vec![
        Triangle {
            material: Some(first.clone()),
            opacity_group: Some(group.clone()),
            paint_owner: Some(owner.clone()),
            ..Default::default()
        },
        Triangle {
            material: Some(second),
            opacity_group: Some(group.clone()),
            paint_owner: Some(owner.clone()),
            ..Default::default()
        },
    ];
    let copy = snapshot_triangles(&source, &Default::default()).unwrap();
    let a = copy[0].material.as_ref().unwrap().borrow();
    let b = copy[1].material.as_ref().unwrap().borrow();
    let stored = a.texture.as_ref().unwrap();
    assert!(!Rc::ptr_eq(stored, &texture));
    assert!(Rc::ptr_eq(stored, b.texture.as_ref().unwrap()));
    assert_eq!(stored.stride, Some(8));
    assert_eq!(stored.pixels.borrow().len(), 16);
    let sample = texture_sample(stored, 0.25, 0.25);
    assert_eq!(sample, texture_sample(&texture, 0.25, 0.25));
    texture.pixels.borrow_mut()[0] = 0;
    first.borrow_mut().color.r = 0;
    source[0].v[0].position.x = 1000.0;
    group.borrow_mut().opacity = 0.0;
    owner.borrow_mut().opaque = false;
    assert_eq!(texture_sample(stored, 0.25, 0.25), sample);
    assert_eq!(a.color.r, 255);
    assert_eq!(copy[0].v[0].position.x, 0.0);
    assert_eq!(
        copy[0].opacity_group.as_ref().unwrap().borrow().opacity,
        0.5
    );
    assert!(copy[0].paint_owner.as_ref().unwrap().borrow().opaque);
    assert!(Rc::ptr_eq(
        copy[0].opacity_group.as_ref().unwrap(),
        copy[1].opacity_group.as_ref().unwrap()
    ));
    assert!(Rc::ptr_eq(
        copy[0].paint_owner.as_ref().unwrap(),
        copy[1].paint_owner.as_ref().unwrap()
    ));
}

#[test]
fn storage_admission_checks_overflow_and_cancellation_before_pixels() {
    for format in [
        TextureFormat::Rgba,
        TextureFormat::Nrgba,
        TextureFormat::Alpha,
    ] {
        let channels = if format == TextureFormat::Alpha { 1 } else { 4 };
        let invalid = Texture::with_storage(2, 3, i64::MAX, format, vec![0; 2 * channels]);
        assert_eq!(
            invalid.validate_storage().unwrap_err(),
            "short native isometric texture storage"
        );
        let valid = Texture::with_storage(2, 1, i64::MAX, format, vec![0; 2 * channels]);
        assert!(valid.validate_storage().is_ok());
        assert_eq!(texture_sample(&valid, 0.5, 0.5), [0.0; 4]);
        let source = vec![Triangle {
            material: Some(material(Material {
                texture: Some(Rc::new(invalid)),
                ..Default::default()
            })),
            ..Default::default()
        }];
        let token = d2_rs_core::CancellationToken::new();
        token.cancel();
        let context = super::super::context::RenderContext::from_token(&token);
        assert_eq!(
            snapshot_triangles(&source, &context).unwrap_err(),
            "context canceled"
        );
        assert_eq!(
            render_mesh_with_dynamic(64, 64, &source, &[], Nrgba::default(), &context).unwrap_err(),
            "context canceled"
        );
    }
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_precision_and_compositor_contracts() {
    let cases: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/raster-api.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        for op in case["operations"].as_array().unwrap() {
            assert_same(
                &replay(&op["request"], Path::new("unused-raster-fixture-output")),
                &op["got"],
            );
        }
    }
}

// JSON encodes Go's float64(0) as 0 and Rust's as 0.0. Compare those
// numeric values exactly, as the source scoreboard does; no tolerance.
fn assert_same(a: &Value, b: &Value) {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => assert_eq!(a.as_f64(), b.as_f64()),
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert_same(a, b);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len());
            for (k, a) in a {
                assert_same(a, b.get(k).unwrap());
            }
        }
        _ => assert_eq!(a, b),
    }
}

/// Test-only bridge for original shadow construction and admission requests.
pub(crate) fn replay_fidelity(v: &Value) -> Value {
    let ts = triangles(&v["triangles"]).unwrap();
    let mut work = Work {
        context: context(v),
        ..Work::new()
    };
    if v["op"] == "validate" {
        return json!({"error":validate(&ts,&work).err().unwrap_or_default()});
    }
    work.remaining = integer(&v["remaining"]);
    let result = build_shadow_at(&ts, &mut work, None);
    match result {
        Ok(s) => {
            let c = s.camera;
            let array = |v: Vec| [v.x, v.y, v.z];
            let camera = json!({"right":array(c.right),"up":array(c.up),"direction":array(c.direction),"center_x":c.center_x,"center_y":c.center_y,"center_depth":c.center_depth,"scale":c.scale,"width":c.width,"height":c.height});
            let depth = s
                .depth
                .iter()
                .flat_map(|n| n.to_le_bytes())
                .collect::<std::vec::Vec<_>>();
            json!({"error":"","remaining":work.remaining,"shadow":{"camera":camera,"depth":base64::engine::general_purpose::STANDARD.encode(depth),"opacity":base64::engine::general_purpose::STANDARD.encode(s.opacity),"bias":s.bias}})
        }
        Err(e) => json!({"error":e,"remaining":work.remaining,"shadow":null}),
    }
}

// Original opacity prepared-draw and shadow calls; this transport is test-only.
pub(crate) fn replay_residual(v: &Value, directory: &Path) -> Value {
    let ts = triangles(&v["triangles"]).unwrap();
    let mut work = Work {
        context: context(v),
        ..Work::new()
    };
    if v["op"] == "validate" {
        return json!({"error":validate(&ts,&work).err().unwrap_or_default()});
    }
    work.remaining = integer(&v["remaining"]);
    if v["op"] == "shadow" {
        return match build_shadow_at(&ts, &mut work, None) {
            Ok(s) => {
                let depth = s
                    .depth
                    .iter()
                    .flat_map(|n| n.to_le_bytes())
                    .collect::<std::vec::Vec<_>>();
                let c = s.camera;
                let array = |v: Vec| [v.x, v.y, v.z];
                json!({"error":"","remaining":work.remaining,"shadow":{"camera":{"right":array(c.right),"up":array(c.up),"direction":array(c.direction),"center_x":c.center_x,"center_y":c.center_y,"center_depth":c.center_depth,"scale":c.scale,"width":c.width,"height":c.height},"depth":format!("{:x}",Sha256::digest(depth)),"opacity":format!("{:x}",Sha256::digest(s.opacity)),"samples":s.depth.len(),"bias":s.bias}})
            }
            Err(e) => json!({"error":e,"remaining":work.remaining,"shadow":null}),
        };
    }
    assert_eq!(v["op"], "draw");
    let (width, height) = (integer(&v["width"]), integer(&v["height"]));
    let shadow = Shadow::default();
    let raster = Raster {
        camera: camera(&v["camera"]).unwrap(),
        shadow: &shadow,
        ground_shadow: &shadow,
        ground: 0.,
        ground_bounds: Bounds::default(),
        background: Nrgba::default(),
        background_texture: None,
    };
    let plan = raster.prepare(&ts);
    let mut pixels = vec![0; (width * height * 4) as usize];
    let mut depth = vec![0.; (width * height) as usize];
    let error = raster
        .draw(
            &mut work,
            &mut pixels,
            &mut depth,
            Bounds {
                x0: 0,
                y0: 0,
                x1: width,
                y1: height,
            },
            &plan,
        )
        .err()
        .unwrap_or_default();
    json!({"error":error,"frame":frame(d2_rs_raster::NrgbaImage{pix:pixels,stride:(width*4) as usize,rect:d2_rs_raster::Rect::new(0,0,width,height)},directory)})
}
