// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg native_page.go: the physical page fold.

use std::rc::Rc;

use d2_rs_render::go_float::{go_fmadd, go_fmsub};
use d2_rs_scene::{go_min, Node as SceneNode, Primitive};

use super::geometry::{native_material, LabelSurface, MeshBuilder};
use super::mesh::{material, ncross, nsub, nunit, nv, MaterialRef, Texture, Vec, Vertex};
use crate::scene::Node;

impl MeshBuilder {
    /// The source page reserves its upper-right corner for a fold; give that
    /// corner a real inclined surface within the same source footprint.
    #[allow(clippy::too_many_arguments)]
    pub fn native_page_fold(
        &mut self,
        n: &Node,
        face: &LabelSurface,
        texture: Option<&Rc<Texture>>,
        source: &MaterialRef,
        dx: f64,
        dz: f64,
        top: f64,
        profile: &[Vec],
    ) {
        let (x, z) = (n.position.x + n.size.x / 2.0 + dx, n.position.z - n.size.z / 2.0 + dz);
        let mut hinge: &[Vec] = &[];
        let mut start: Option<usize> = None;
        for (i, p) in profile.iter().enumerate() {
            if (p.z - z).abs() < 1e-8 {
                start = Some(i);
            }
            if let Some(s) = start {
                if i > s && (p.x - x).abs() < 1e-8 {
                    hinge = &profile[s..=i];
                    break;
                }
            }
        }
        if hinge.len() < 2 {
            return;
        }
        let (cw, cd) = (x - hinge[0].x, hinge[hinge.len() - 1].z - z);
        // ARM64 native_page.go:37.
        let tip = nv(go_fmsub(cw, 0.90, x), go_fmadd(go_min(cw, cd), 0.62, top), go_fmadd(cd, 0.90, z));
        let mat = match texture {
            Some(texture) => {
                let mut m = native_material("white", 0.68, 0.0, n.opacity).borrow().clone();
                m.texture = Some(texture.clone());
                m.vector = self.vector_for_texture(texture);
                material(m)
            }
            None => material(source.borrow().clone()),
        };
        for i in 1..hinge.len() {
            let (a, c) = (hinge[i - 1], hinge[i]);
            let normal = nunit(ncross(nsub(tip, a), nsub(c, a)));
            let vertex = |at: Vec| Vertex {
                position: at,
                normal,
                u: (at.x - face.center.x) / face.width + 0.5,
                v: (at.z - face.center.z) / face.depth + 0.5,
            };
            self.triangle(vertex(a), vertex(tip), vertex(c), &mat, true);
        }
    }
}

/// The second source Page path draws the flat fold and repeats an inset
/// perimeter; the physical fold replaces that glyph.
pub fn native_remove_page_glyph(root: &mut SceneNode) {
    fn visit(n: &mut SceneNode, path_index: &mut usize) {
        if let Some(Primitive::Path(p)) = &mut n.primitive {
            if p.stroke.is_some() {
                if *path_index == 1 {
                    p.stroke = None;
                }
                *path_index += 1;
            }
        }
        for child in n.children.iter_mut().flatten() {
            visit(child, path_index);
        }
    }
    visit(root, &mut 0);
}
