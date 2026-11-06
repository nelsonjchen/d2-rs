// SPDX-License-Identifier: MPL-2.0
//! Test-only adapter; compares complete mesh records, without rounding.
use d2_rs_isometric::img::geometry_core::MeshGeometry;
use d2_rs_isometric::img::mesh::{Material, MaterialRef, Texture, Vec, material, nv};
use d2_rs_isometric::img::vector_surface::VectorSurface;
use d2_rs_scene::Nrgba;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::rc::Rc;

fn vector(v: &Value) -> Vec {
    nv(
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    )
}
fn color(v: &Value) -> Nrgba {
    Nrgba::new(
        v[0].as_u64().unwrap() as u8,
        v[1].as_u64().unwrap() as u8,
        v[2].as_u64().unwrap() as u8,
        v[3].as_u64().unwrap() as u8,
    )
}
fn vc(v: Vec) -> Value {
    json!([v.x, v.y, v.z])
}
fn cc(c: Nrgba) -> Value {
    json!([c.r, c.g, c.b, c.a])
}

pub fn replay(call: &Value) -> Result<Value, String> {
    let mut surfaces = HashMap::new();
    let materials: std::vec::Vec<_> = call["materials"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            material(Material {
                color: color(&m["color"]),
                roughness: m["roughness"].as_f64().unwrap(),
                metalness: m["metalness"].as_f64().unwrap(),
                emissive: color(&m["emissive"]),
                texture: m["texture"].as_array().map(|v| {
                    Rc::new(Texture::new(
                        v[0].as_u64().unwrap() as usize,
                        v[1].as_u64().unwrap() as usize,
                        vec![],
                    ))
                }),
                vector: m["vector"].as_u64().map(|id| {
                    surfaces
                        .entry(id)
                        .or_insert_with(|| Rc::new(VectorSurface::default()))
                        .clone()
                }),
                unlit: m["unlit"].as_bool().unwrap_or(false),
                multiply: m["multiply"].as_bool().unwrap_or(false),
                svg_contour: m["svgContour"].as_bool().unwrap_or(false),
                svg_solid_texture: m["svgSolidTexture"].as_bool().unwrap_or(false),
                gradient: m["gradient"].as_str().unwrap_or("").into(),
            })
        })
        .collect();
    let arg: std::vec::Vec<_> = call["materialArgs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|i| *i > 0)
                .map(|i| materials[i as usize - 1].clone())
        })
        .collect();
    let transparent = material(Material::default());
    let m = |i: usize| arg[i].as_ref().unwrap_or(&transparent);
    let mut geometry = MeshGeometry::with_prefix_count(call["prior"].as_u64().unwrap() as usize);
    geometry.err = call["error"]
        .as_str()
        .filter(|e| !e.is_empty())
        .map(str::to_owned);
    let a = &call["args"];
    match call["op"].as_str().ok_or("missing op")? {
        "box" => geometry.box_(
            vector(&a["center"]),
            vector(&a["size"]),
            m(0),
            a["radius"].as_f64().unwrap(),
        ),
        "sphere" => geometry.sphere(
            vector(&a["center"]),
            vector(&a["radii"]),
            m(0),
            a["segments"].as_i64().unwrap(),
            a["rings"].as_i64().unwrap(),
        ),
        "cylinder" => geometry.cylinder(
            vector(&a["center"]),
            a["rx"].as_f64().unwrap(),
            a["rz"].as_f64().unwrap(),
            a["height"].as_f64().unwrap(),
            a["topRatio"].as_f64().unwrap(),
            m(0),
            m(1),
            a["segments"].as_i64().unwrap(),
            a["horizontal"].as_bool().unwrap(),
        ),
        "flat" => geometry.flat(
            vector(&a["a"]),
            vector(&a["c"]),
            vector(&a["d"]),
            m(0),
            a["shadow"].as_bool().unwrap(),
        ),
        "extrudedProfile" => geometry.extruded_profile(
            &a["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(vector)
                .collect::<std::vec::Vec<_>>(),
            a["base"].as_f64().unwrap(),
            arg[0].as_ref(),
            m(1),
        ),
        op => return Err(format!("unsupported mesh operation: {op}")),
    }
    let mut material_ids = HashMap::new();
    let mut vector_ids = HashMap::new();
    let mut output_materials = vec![];
    let mut triangles = vec![];
    for t in &geometry.triangles {
        let id = if let Some(material) = &t.material {
            let ptr = Rc::as_ptr(material);
            if let Some(id) = material_ids.get(&ptr) {
                *id
            } else {
                let id = material_ids.len() + 1;
                material_ids.insert(ptr, id);
                output_materials.push(material_json(material, &mut vector_ids));
                id
            }
        } else {
            0
        };
        let mut out = json!({"material": id, "v": t.v.iter().map(|v| json!({"p": vc(v.position), "n": vc(v.normal), "u": v.u, "v": v.v})).collect::<std::vec::Vec<_>>()});
        if t.cast_shadow {
            out["castShadow"] = true.into();
        }
        triangles.push(out);
    }
    Ok(
        json!({"triangles": if triangles.is_empty() { Value::Null } else { triangles.into() },
        "materials": if output_materials.is_empty() { Value::Null } else { output_materials.into() },
        "error": geometry.err.unwrap_or_default()}),
    )
}
fn material_json(
    material: &MaterialRef,
    vectors: &mut HashMap<*const VectorSurface, usize>,
) -> Value {
    let m = material.borrow();
    let mut v = json!({"color": cc(m.color), "roughness": m.roughness, "metalness": m.metalness, "emissive": cc(m.emissive)});
    if let Some(texture) = &m.texture {
        v["texture"] = json!([texture.width, texture.height]);
    }
    if let Some(surface) = &m.vector {
        let next = vectors.len() + 1;
        v["vector"] = (*vectors.entry(Rc::as_ptr(surface)).or_insert(next)).into();
    }
    for (key, yes) in [
        ("unlit", m.unlit),
        ("multiply", m.multiply),
        ("svgContour", m.svg_contour),
        ("svgSolidTexture", m.svg_solid_texture),
    ] {
        if yes {
            v[key] = true.into();
        }
    }
    if !m.gradient.is_empty() {
        v["gradient"] = m.gradient.clone().into();
    }
    v
}
