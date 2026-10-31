// SPDX-License-Identifier: MPL-2.0
//! Per-board debugging adapter for Go's TestD2rsMeshDump. This example-only
//! code is absent from native libraries and normal/release render paths.
use d2_rs_isometric::img::{mesh::*, svg::NativeScene, vector_surface::VectorSurface};
use serde_json::{Value, json};
use std::{collections::HashMap, rc::Rc};
fn vc(v: Vec) -> Value {
    json!([v.x, v.y, v.z])
}
fn cc(c: d2_rs_scene::Nrgba) -> Value {
    json!([c.r, c.g, c.b, c.a])
}
fn surface(s: &VectorSurface) -> Value {
    let mut v = json!({"document":&*s.document.borrow()});
    if let Some(ink) = &*s.ink_coverage.borrow() {
        v["inkCoverage"] = surface(ink);
    }
    if s.coverage_opacity.get() != 0.0 {
        v["coverageOpacity"] = s.coverage_opacity.get().into();
    }
    if let Some(c) = s.cap_background {
        v["capBackground"] = cc(c);
    }
    if let Some(p) = &s.cap_paint {
        v["capPaint"] = json!(p);
    }
    v
}
pub fn dump(s: &NativeScene) -> Value {
    let c = s.camera;
    let mut out = json!({"width":s.width,"height":s.height,"background":cc(s.background),"camera":{"right":vc(c.right),"up":vc(c.up),"direction":vc(c.direction),"centerX":c.center_x,"centerY":c.center_y,"centerDepth":c.center_depth,"scale":c.scale,"width":c.width,"height":c.height}});
    let (mut material_ids, mut vector_ids, mut group_ids, mut owner_ids) = (
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
    );
    let (mut materials, mut surfaces, mut groups, mut owners, mut triangles) =
        (vec![], vec![], vec![], vec![], vec![]);
    for t in &s.triangles {
        let mut row = json!({"v":t.v.iter().map(|v|json!({"p":vc(v.position),"n":vc(v.normal),"u":v.u,"v":v.v})).collect::<std::vec::Vec<_>>(),"material":0});
        if let Some(m) = &t.material {
            let next = material_ids.len() + 1;
            let ptr = Rc::as_ptr(m) as usize;
            let id=*material_ids.entry(ptr).or_insert_with(||{
                let m=m.borrow();let mut entry=json!({"color":cc(m.color),"roughness":m.roughness,"metalness":m.metalness,"emissive":cc(m.emissive)});
                if let Some(tex)=&m.texture{entry["texture"]=json!([tex.width,tex.height]);}
                if let Some(vector)=&m.vector{let next=vector_ids.len()+1;let id=*vector_ids.entry(Rc::as_ptr(vector) as usize).or_insert_with(||{surfaces.push(surface(vector));next});entry["vector"]=id.into();}
                for(k,yes)in[("unlit",m.unlit),("multiply",m.multiply),("svgContour",m.svg_contour),("svgSolidTexture",m.svg_solid_texture)]{if yes{entry[k]=true.into();}}
                if !m.gradient.is_empty(){entry["gradient"]=m.gradient.clone().into();}materials.push(entry);next
            });
            row["material"] = id.into();
        }
        for (k, yes) in [
            ("castShadow", t.cast_shadow),
            ("noDepthWrite", t.no_depth_write),
            ("coverageOnly", t.svg_coverage_only),
        ] {
            if yes {
                row[k] = true.into();
            }
        }
        for (k, value) in [
            ("shadowOpacity", t.shadow_opacity),
            ("shadowGround", t.shadow_ground),
        ] {
            if let Some(v) = value {
                row[k] = v.into();
            }
        }
        if let Some(a) = t.shadow_fill_alpha {
            row["shadowFillAlpha"] = a.into();
        }
        if t.depth_bias != 0.0 {
            row["depthBias"] = t.depth_bias.into();
        }
        if let Some(g) = &t.opacity_group {
            let next = group_ids.len() + 1;
            let id = *group_ids.entry(Rc::as_ptr(g) as usize).or_insert_with(|| {
                groups.push(g.borrow().opacity);
                next
            });
            row["opacityGroup"] = id.into();
        }
        if let Some(o) = &t.paint_owner {
            let next = owner_ids.len() + 1;
            let id = *owner_ids.entry(Rc::as_ptr(o) as usize).or_insert_with(|| {
                owners.push(o.borrow().opaque);
                next
            });
            row["paintOwner"] = id.into();
        }
        triangles.push(row);
    }
    out["materials"] = if materials.is_empty() {
        Value::Null
    } else {
        json!(materials)
    };
    out["triangles"] = if triangles.is_empty() {
        Value::Null
    } else {
        json!(triangles)
    };
    out["groups"] = if groups.is_empty() {
        Value::Null
    } else {
        json!(groups)
    };
    out["owners"] = if owners.is_empty() {
        Value::Null
    } else {
        json!(owners)
    };
    if !surfaces.is_empty() {
        out["surfaces"] = json!(surfaces);
    }
    if let Some(bg) = &s.background_paint {
        out["backgroundPaint"] = true.into();
        out["backgroundSurface"] = surface(bg);
    }
    if !s.links.is_empty() {
        out["links"]=json!(s.links.iter().map(|l|{let mut v=json!({"points":l.points.iter().map(|p|vc(*p)).collect::<std::vec::Vec<_>>()});for(k,s)in[("url",&l.url),("tooltip",&l.tooltip),("target",&l.target)]{if !s.is_empty(){v[k]=s.clone().into();}}v}).collect::<std::vec::Vec<_>>());
    }
    out
}
