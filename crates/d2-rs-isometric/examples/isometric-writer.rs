// SPDX-License-Identifier: MPL-2.0
//! Rebuilds a native scene from a Go mesh dump (<board>.mesh.json) and runs
//! the SVG writer. --surfaces loads retained documents and serializes them
//! natively; otherwise the legacy writer tier substitutes the marker `§`.
//! scripts/isometric-scoreboard.py compares each tier with the Go golden.

use std::cell::RefCell;
use std::rc::Rc;

use d2_rs_isometric::img::mesh::{
    Camera, Material, OpacityGroup, PaintOwner, Texture, Triangle, Vertex, material, nv,
};
use d2_rs_isometric::img::svg::{NativeLink, NativeScene, native_scene_svg};
use d2_rs_isometric::img::vector_surface::VectorSurface;
use d2_rs_scene::Nrgba;

fn vec3(v: &serde_json::Value) -> d2_rs_isometric::img::mesh::Vec {
    nv(
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    )
}

fn rgba(v: &serde_json::Value) -> Nrgba {
    let c = |i: usize| v[i].as_u64().unwrap() as u8;
    Nrgba::new(c(0), c(1), c(2), c(3))
}

fn surface(v: &serde_json::Value) -> Result<Rc<VectorSurface>, Box<dyn std::error::Error>> {
    let mut document: d2_rs_scene::Document = serde_json::from_value(v["document"].clone())?;
    document.load_asset_files()?;
    Ok(Rc::new(VectorSurface {
        document: Rc::new(RefCell::new(document)),
        ink_coverage: RefCell::new(v.get("inkCoverage").map(surface).transpose()?),
        coverage_opacity: std::cell::Cell::new(v["coverageOpacity"].as_f64().unwrap_or(0.0)),
        cap_background: v.get("capBackground").map(rgba),
        cap_paint: v
            .get("capPaint")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?,
    }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (input, output) = (args.next().ok_or("input")?, args.next().ok_or("output")?);
    let full = args.next().as_deref() == Some("--surfaces");
    let dump: serde_json::Value = serde_json::from_slice(&std::fs::read(&input)?)?;
    let mut surfaces: std::collections::BTreeMap<u64, Rc<VectorSurface>> = Default::default();
    if full {
        for (i, value) in dump["surfaces"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            surfaces.insert(i as u64 + 1, surface(value)?);
        }
    }
    let materials: Vec<_> = dump["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let texture = m["texture"].as_array().map(|t| {
                Rc::new(Texture::new(
                    t[0].as_u64().unwrap() as usize,
                    t[1].as_u64().unwrap() as usize,
                    Vec::new(),
                ))
            });
            let vector = m["vector"].as_u64().map(|id| {
                surfaces
                    .entry(id)
                    .or_insert_with(|| Rc::new(VectorSurface::default()))
                    .clone()
            });
            material(Material {
                color: rgba(&m["color"]),
                roughness: m["roughness"].as_f64().unwrap(),
                metalness: m["metalness"].as_f64().unwrap(),
                emissive: rgba(&m["emissive"]),
                texture,
                vector,
                unlit: m["unlit"].as_bool().unwrap_or(false),
                multiply: m["multiply"].as_bool().unwrap_or(false),
                svg_contour: m["svgContour"].as_bool().unwrap_or(false),
                svg_solid_texture: m["svgSolidTexture"].as_bool().unwrap_or(false),
                gradient: m["gradient"].as_str().unwrap_or("").to_owned(),
            })
        })
        .collect();
    let groups: Vec<_> = dump["groups"]
        .as_array()
        .map(|g| {
            g.iter()
                .map(|o| {
                    Rc::new(RefCell::new(OpacityGroup {
                        opacity: o.as_f64().unwrap(),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    let owners: Vec<_> = dump["owners"]
        .as_array()
        .map(|g| {
            g.iter()
                .map(|o| {
                    Rc::new(RefCell::new(PaintOwner {
                        opaque: o.as_bool().unwrap(),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    let triangles = dump["triangles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let mut v = [Vertex::default(); 3];
            for (i, vertex) in t["v"].as_array().unwrap().iter().enumerate() {
                v[i] = Vertex {
                    position: vec3(&vertex["p"]),
                    normal: vec3(&vertex["n"]),
                    u: vertex["u"].as_f64().unwrap(),
                    v: vertex["v"].as_f64().unwrap(),
                };
            }
            let index = |key: &str| t[key].as_u64().filter(|i| *i > 0).map(|i| i as usize - 1);
            Triangle {
                v,
                material: index("material").map(|i| materials[i].clone()),
                cast_shadow: t["castShadow"].as_bool().unwrap_or(false),
                shadow_opacity: t["shadowOpacity"].as_f64(),
                shadow_fill_alpha: t["shadowFillAlpha"].as_u64().map(|a| a as u8),
                shadow_ground: t["shadowGround"].as_f64(),
                depth_bias: t["depthBias"].as_f64().unwrap_or(0.0),
                no_depth_write: t["noDepthWrite"].as_bool().unwrap_or(false),
                opacity_group: index("opacityGroup").map(|i| groups[i].clone()),
                paint_owner: index("paintOwner").map(|i| owners[i].clone()),
                svg_coverage_only: t["coverageOnly"].as_bool().unwrap_or(false),
            }
        })
        .collect();
    let c = &dump["camera"];
    let camera = Camera {
        right: vec3(&c["right"]),
        up: vec3(&c["up"]),
        direction: vec3(&c["direction"]),
        center_x: c["centerX"].as_f64().unwrap(),
        center_y: c["centerY"].as_f64().unwrap(),
        center_depth: c["centerDepth"].as_f64().unwrap(),
        scale: c["scale"].as_f64().unwrap(),
        width: c["width"].as_i64().unwrap(),
        height: c["height"].as_i64().unwrap(),
    };
    let links = dump["links"]
        .as_array()
        .map(|links| {
            links
                .iter()
                .map(|l| NativeLink {
                    points: l["points"].as_array().unwrap().iter().map(vec3).collect(),
                    url: l["url"].as_str().unwrap_or("").to_owned(),
                    tooltip: l["tooltip"].as_str().unwrap_or("").to_owned(),
                    target: l["target"].as_str().unwrap_or("").to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut scene = NativeScene {
        triangles,
        camera,
        width: dump["width"].as_i64().unwrap(),
        height: dump["height"].as_i64().unwrap(),
        background: rgba(&dump["background"]),
        background_paint: if full {
            dump.get("backgroundSurface").map(surface).transpose()?
        } else {
            dump["backgroundPaint"]
                .as_bool()
                .unwrap_or(false)
                .then(|| Rc::new(VectorSurface::default()))
        },
        links,
        ..Default::default()
    };
    let stub = |_: &VectorSurface, _: &str| -> Result<String, String> { Ok("§".to_owned()) };
    let writer: &d2_rs_isometric::img::svg::SurfaceWriter = if full {
        &d2_rs_isometric::img::vector_surface_svg::native_surface_svg
    } else {
        &stub
    };
    match native_scene_svg(&mut scene, writer) {
        Ok(svg) => std::fs::write(output, svg)?,
        Err(error) => std::fs::write(format!("{output}.err"), error)?,
    }
    Ok(())
}
