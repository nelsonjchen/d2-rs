// SPDX-License-Identifier: MPL-2.0
//! opacity_groups.go: mesh paint ownership (raster composition is separate).
use super::{geometry::MeshBuilder, mesh::PaintOwner};
use std::{cell::RefCell, rc::Rc};
impl MeshBuilder {
    pub fn mark_paint_owner(&mut self, first: usize) {
        if self.err.is_some() || first >= self.triangles.len() {
            return;
        }
        let (mut owner, mut opaque, mut partial) = (None, false, false);
        for t in &self.triangles[first..] {
            if t.opacity_group.is_some() {
                return;
            }
            if t.paint_owner.is_some() {
                owner = t.paint_owner.clone();
            }
            if let Some(m) = &t.material {
                let m = m.borrow();
                if !m.unlit && m.texture.is_none() && m.color.a > 0 {
                    opaque |= m.color.a == 255;
                    partial |= m.color.a < 255;
                }
            }
        }
        let owner = owner.unwrap_or_else(|| {
            Rc::new(RefCell::new(PaintOwner {
                opaque: opaque && !partial,
            }))
        });
        for t in &mut self.triangles[first..] {
            t.paint_owner = Some(owner.clone());
        }
    }
}
