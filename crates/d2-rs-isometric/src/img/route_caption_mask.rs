// SPDX-License-Identifier: MPL-2.0
//! route_caption_mask.go: subtract the caption from the actual dashed ink.
use super::{
    geometry::{LabelSurface, MeshBuilder},
    math as gomath,
    mesh::*,
    route_labels::caption_projection,
};
impl MeshBuilder {
    pub fn route_caption_knockout(&mut self, first: usize, s: &LabelSurface) {
        if s.width <= 0.0 || s.depth <= 0.0 || first == self.triangles.len() {
            return;
        }
        let wire = self.geometry.triangles.split_off(first);
        let (cos, sin) = (gomath::cos(s.angle), gomath::sin(s.angle));
        let center = caption_projection(s.center);
        for t in wire {
            if self.err.is_some() {
                return;
            }
            let mut remaining = t.v.to_vec();
            for (side, bound) in [-s.width / 2.0, s.width / 2.0, -s.depth / 2.0, s.depth / 2.0]
                .into_iter()
                .enumerate()
            {
                if remaining.len() < 3 {
                    break;
                }
                let distance = |v: Vertex| {
                    let p = caption_projection(v.position);
                    let (dx, dz) = (p.x - center.x, p.z - center.z);
                    // ARM64 route_caption_mask.go:39 rounds dz*sin,
                    // then fuses dx*cos; z rounds its first product.
                    let x = dx.mul_add(cos, dz * sin);
                    let z = dz.mul_add(cos, -dx * sin);
                    match side {
                        0 => x - bound,
                        1 => bound - x,
                        2 => z - bound,
                        _ => bound - z,
                    }
                };
                let (mut interior, mut exterior) = (vec![], vec![]);
                let mut previous = *remaining.last().unwrap();
                let mut pd = distance(previous);
                for current in remaining {
                    let cd = distance(current);
                    if (pd >= 0.0) != (cd >= 0.0) {
                        let t = pd / (pd - cd);
                        let cut = Vertex {
                            position: nlerp(previous.position, current.position, t),
                            normal: nlerp(previous.normal, current.normal, t),
                            u: (current.u - previous.u).mul_add(t, previous.u),
                            v: (current.v - previous.v).mul_add(t, previous.v),
                        };
                        if pd != 0.0 && cd != 0.0 {
                            interior.push(cut)
                        }
                        exterior.push(cut)
                    }
                    if cd >= 0.0 {
                        interior.push(current)
                    } else {
                        exterior.push(current)
                    }
                    previous = current;
                    pd = cd;
                }
                for i in 1..exterior.len().saturating_sub(1) {
                    if nlen(ncross(
                        nsub(exterior[i].position, exterior[0].position),
                        nsub(exterior[i + 1].position, exterior[0].position),
                    )) > 1e-18
                    {
                        let first = self.triangles.len();
                        if let Some(m) = &t.material {
                            self.triangle(
                                exterior[0],
                                exterior[i],
                                exterior[i + 1],
                                m,
                                t.cast_shadow,
                            );
                            if self.triangles.len() > first {
                                let mut piece = t.clone();
                                piece.v = self.triangles[first].v;
                                self.geometry.triangles[first] = piece
                            }
                        }
                    }
                }
                remaining = interior;
            }
        }
    }
}
