// SPDX-License-Identifier: MPL-2.0
//! gradient_paints.go: bounded textures and shared geometric paint coordinates.
use super::{
    framing::native_camera_axes,
    geometry::{MeshBuilder, css_color},
    mesh::{Texture, nmul, nsub},
};
use d2_rs_scene::{Box as SceneBox, Document, Node, Nrgba, Paint, Primitive, Rect, go_max, go_min};
use std::{collections::HashMap, rc::Rc};
pub fn native_paint_document(paint: Paint) -> Document {
    let bx = SceneBox {
        width: 256.0,
        height: 256.0,
        ..Default::default()
    };
    Document::new(
        bx,
        Node::new(Some(Primitive::Rect(Rect {
            r#box: bx,
            fill: Some(paint),
            ..Default::default()
        }))),
    )
}
pub fn native_gradient_summary(raw: &str) -> Option<Nrgba> {
    let paint = d2_rs_scenebuild::gradient_paint(raw).ok()?;
    let stops = match paint {
        Paint::LinearGradient(p) => p.stops,
        Paint::RadialGradient(p) => p.stops,
        _ => return None,
    };
    if stops.is_empty() {
        return None;
    }
    let (mut sums, mut opaque, mut clear) = ([0.0; 4], true, true);
    for s in &stops {
        let c = s.color;
        for (i, v) in [c.r, c.g, c.b, c.a].into_iter().enumerate() {
            sums[i] += v as f64;
        }
        opaque &= c.a == 255;
        clear &= c.a == 0;
    }
    for s in &mut sums {
        *s /= stops.len() as f64;
    }
    let mut a = sums[3].round() as u8;
    if !opaque && a == 255 {
        a = 254;
    }
    if !clear && a == 0 {
        a = 1;
    }
    Some(Nrgba::new(
        sums[0].round() as u8,
        sums[1].round() as u8,
        sums[2].round() as u8,
        a,
    ))
}
pub fn native_gradient_opacity(raw: &str, opacity: f64) -> Result<String, String> {
    let (kind, direction, stops) = super::backends::gradient_parts(raw)?;
    let mut parts = vec![];
    if !direction.is_empty() {
        parts.push(direction);
    }
    for (color, position) in stops {
        let mut c = css_color(&color).ok_or_else(|| format!("invalid color {color:?}"))?;
        c.a = (c.a as f64 * opacity).round() as u8;
        let mut value = super::rich_labels::rich_color(c);
        if !position.is_empty() {
            value.push(' ');
            value.push_str(&position);
        }
        parts.push(value);
    }
    Ok(format!("{kind}-gradient({})", parts.join(",")))
}

impl MeshBuilder {
    pub fn gradient_texture(&mut self, raw: &str) -> Option<Rc<Texture>> {
        if self.err.is_some() {
            return None;
        }
        if let Some(t) = self.gradient_textures.get(raw) {
            return Some(t.clone());
        }
        if self.gradient_textures.len() >= 128 {
            self.fail("isometric gradients exceed 8388608 texture pixels");
            return None;
        }
        let result = d2_rs_scenebuild::gradient_paint(raw).and_then(|p| {
            let doc = native_paint_document(p);
            let t = Rc::new(Texture::new(
                256,
                256,
                super::backends::raster_surface(&doc, 256, 256)?,
            ));
            self.retain_vector_surface(&t, &doc)?;
            Ok(t)
        });
        match result {
            Ok(t) => {
                self.gradient_textures.insert(raw.into(), t.clone());
                Some(t)
            }
            Err(e) => {
                self.fail(e);
                None
            }
        }
    }
    pub fn prepare_gradient_materials(&mut self) {
        let mut bounds = HashMap::new();
        let view = native_camera_axes().direction;
        let point = |p: super::mesh::Vec| nsub(p, nmul(view, p.y / view.y));
        for t in &self.triangles {
            let Some(m) = &t.material else { continue };
            if m.borrow().gradient.is_empty() || m.borrow().texture.is_some() {
                continue;
            }
            let e = bounds.entry(Rc::as_ptr(m) as usize).or_insert((
                m.clone(),
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
            ));
            for v in t.v {
                let p = point(v.position);
                e.1[0] = go_min(e.1[0], p.x);
                e.1[1] = go_min(e.1[1], p.z);
                e.1[2] = go_max(e.1[2], p.x);
                e.1[3] = go_max(e.1[3], p.z);
            }
        }
        for (m, _) in bounds.values() {
            let raw = m.borrow().gradient.clone();
            let texture = self.gradient_texture(&raw);
            if self.err.is_some() {
                return;
            }
            let vector = texture.as_ref().and_then(|t| self.vector_for_texture(t));
            let mut m = m.borrow_mut();
            m.texture = texture;
            m.vector = vector;
        }
        for t in &mut self.geometry.triangles {
            let Some(m) = &t.material else { continue };
            let Some((_, e)) = bounds.get(&(Rc::as_ptr(m) as usize)) else {
                continue;
            };
            for v in &mut t.v {
                let p = point(v.position);
                v.u = (p.x - e[0]) / go_max(1e-9, e[2] - e[0]);
                v.v = (p.z - e[1]) / go_max(1e-9, e[3] - e[1]);
            }
        }
    }
}
