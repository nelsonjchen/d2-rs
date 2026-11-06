// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg framing.go: projected extents, the final camera and the
//! output-density estimate. Contraction sites are marked `ARM64:`; products
//! by powers of two are exact.

use super::math as gomath;
use d2_rs_render::go_float::{go_fmadd, go_fmsub};
use d2_rs_scene::{go_max, go_min};

use super::geometry_routes::PacketRoute;
use super::hierarchy_boards::hierarchy_node_relief;
use super::mesh::{
    Camera, Triangle, Vec, nadd, native_view_direction, ncross, ndot, nlerp, nmul, nsub, nv,
    raster_fit, raster_unit,
};
use super::native_shapes::{NativeAnimatedNode, native_canonical_height, native_relief_symbol};
use super::native_solids::{native_solid_height, native_solid_node};
use super::native_structured::native_structured_node;
use super::svg::{RASTER_SHADOW_NORMAL_OFFSET, raster_ground_triangles, raster_shadow_ground};
use crate::scene::Scene;

#[derive(Clone, Debug, Default)]
pub struct NativeSceneOptions {
    pub fit_content: bool,
    pub defer_raster: bool,
    /// Retain solid cap coverage beneath vector face paint.
    pub vector: bool,
    /// Go's nativeVectorContext can retain documents independently of cap coverage.
    pub retain_vector_surfaces: bool,
    /// Final logical pixels, before supersampling.
    pub camera: Option<Camera>,
    pub output_density: f64,
    pub links: Option<d2_rs_scenebuild::LinkBudget>,
}

pub const NATIVE_TRAFFIC_RADIUS: f64 = 0.045;

pub fn raster_shadow_direction() -> Vec {
    nv(-1.0, 1.8, 1.0)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProjectedExtent {
    pub min_x: f64,
    pub min_y: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub max_z: f64,
    pub valid: bool,
}

impl ProjectedExtent {
    pub fn add(&mut self, p: Vec) {
        let c = native_camera_axes();
        let (x, y, z) = (ndot(p, c.right), ndot(p, c.up), ndot(p, c.direction));
        if !self.valid {
            *self = ProjectedExtent {
                min_x: x,
                min_y: y,
                min_z: z,
                max_x: x,
                max_y: y,
                max_z: z,
                valid: true,
            };
            return;
        }
        self.min_x = go_min(self.min_x, x);
        self.min_y = go_min(self.min_y, y);
        self.min_z = go_min(self.min_z, z);
        self.max_x = go_max(self.max_x, x);
        self.max_y = go_max(self.max_y, y);
        self.max_z = go_max(self.max_z, z);
    }

    pub fn mesh(&mut self, ts: &[Triangle]) {
        for t in ts {
            for v in &t.v {
                self.add(v.position);
            }
        }
    }

    pub fn union(&mut self, other: &ProjectedExtent) {
        if !self.valid {
            *self = *other;
            return;
        }
        self.min_x = go_min(self.min_x, other.min_x);
        self.min_y = go_min(self.min_y, other.min_y);
        self.min_z = go_min(self.min_z, other.min_z);
        self.max_x = go_max(self.max_x, other.max_x);
        self.max_y = go_max(self.max_y, other.max_y);
        self.max_z = go_max(self.max_z, other.max_z);
    }

    pub fn animated_mesh(&mut self, ts: &[Triangle], nodes: &[NativeAnimatedNode], scale: f64) {
        self.mesh(ts);
        for node in nodes {
            for t in &ts[node.first..node.last] {
                for v in &t.v {
                    let mut p = v.position;
                    p.z -= 4.0 * scale;
                    self.add(p);
                }
            }
        }
        self.ground_shadows(ts, nodes, scale, None);
    }

    /// The ground receiver is implicit raster content; project caster
    /// vertices onto it and reserve the shadow map's sampling footprint.
    pub fn ground_shadows(
        &mut self,
        ts: &[Triangle],
        nodes: &[NativeAnimatedNode],
        scale: f64,
        shadow_camera: Option<&Camera>,
    ) {
        if ts.is_empty() {
            return;
        }
        let ground = raster_shadow_ground(ts);
        let ts = raster_ground_triangles(ts, ground, native_view_direction());
        self.prepared_ground_shadows(&ts, nodes, scale, ground, shadow_camera);
    }

    pub fn prepared_ground_shadows(
        &mut self,
        ts: &[Triangle],
        nodes: &[NativeAnimatedNode],
        scale: f64,
        ground: f64,
        shadow_camera: Option<&Camera>,
    ) {
        let receiver = ground + RASTER_SHADOW_NORMAL_OFFSET;
        let light = match shadow_camera {
            Some(c) => *c,
            None => raster_fit(ts, raster_shadow_direction(), 2048, 2048, 1.08),
        };
        let mut shadow = ProjectedExtent::default();
        let cast = |shadow: &mut ProjectedExtent, p: Vec| {
            let distance = (p.y - receiver) / light.direction.y;
            let mut p = nsub(p, nmul(light.direction, distance));
            p.y = ground;
            shadow.add(p);
        };
        let triangle = |shadow: &mut ProjectedExtent, t: &Triangle, shift: f64, animated: bool| {
            let material = t.material.as_ref().map(|m| m.borrow());
            if material.as_ref().is_some_and(|m| m.color.a == 0) {
                return;
            }
            let mut casts = t.cast_shadow && t.shadow_opacity.is_none_or(|o| o > 0.0);
            if animated
                && !t.cast_shadow
                && material
                    .as_ref()
                    .is_none_or(|m| m.texture.is_none() || !t.no_depth_write)
            {
                casts = true; // frameRaster enables an authored animation's shadow
            }
            if !casts {
                return;
            }
            for i in 0..3 {
                let (mut a, mut b) = (t.v[i].position, t.v[(i + 1) % 3].position);
                a.z -= shift;
                b.z -= shift;
                if a.y >= receiver {
                    cast(shadow, a);
                }
                if (a.y < receiver) != (b.y < receiver) {
                    cast(shadow, nlerp(a, b, (receiver - a.y) / (b.y - a.y)));
                }
            }
        };
        for t in ts {
            triangle(&mut shadow, t, 0.0, false);
        }
        for node in nodes {
            for t in &ts[node.first..node.last] {
                triangle(&mut shadow, t, 0.0, true);
                triangle(&mut shadow, t, 4.0 * scale, true);
            }
        }
        if !shadow.valid {
            return;
        }
        // A caster sample can influence receiver samples up to 1.5 shadow
        // texels away.
        let u = nsub(
            light.right,
            nmul(light.direction, light.right.y / light.direction.y),
        );
        let v = nsub(
            light.up,
            nmul(light.direction, light.up.y / light.direction.y),
        );
        let (radius, camera) = (1.5 / light.scale, native_camera_axes());
        let expand = |axis: Vec| ndot(u, axis).abs() + ndot(v, axis).abs();
        let (ex, ey, ez) = (
            expand(camera.right),
            expand(camera.up),
            expand(camera.direction),
        );
        // ARM64 framing.go:135–137: radius*(...) fuses into each bound.
        shadow.min_x = go_fmsub(radius, ex, shadow.min_x);
        shadow.max_x = go_fmadd(radius, ex, shadow.max_x);
        shadow.min_y = go_fmsub(radius, ey, shadow.min_y);
        shadow.max_y = go_fmadd(radius, ey, shadow.max_y);
        shadow.min_z = go_fmsub(radius, ez, shadow.min_z);
        shadow.max_z = go_fmadd(radius, ez, shadow.max_z);
        self.union(&shadow);
    }

    /// Expanding the packets' projected union by the sphere radius contains
    /// every traffic phase.
    pub fn traffic_routes(&mut self, packets: &[PacketRoute]) {
        let mut traffic = ProjectedExtent::default();
        for packet in packets {
            for point in &packet.points {
                traffic.add(*point);
            }
        }
        if !traffic.valid {
            return;
        }
        traffic.min_x -= NATIVE_TRAFFIC_RADIUS;
        traffic.min_y -= NATIVE_TRAFFIC_RADIUS;
        traffic.min_z -= NATIVE_TRAFFIC_RADIUS;
        traffic.max_x += NATIVE_TRAFFIC_RADIUS;
        traffic.max_y += NATIVE_TRAFFIC_RADIUS;
        traffic.max_z += NATIVE_TRAFFIC_RADIUS;
        self.union(&traffic);
    }

    pub fn camera(&self, mut width: i64, mut height: i64, fit: bool) -> Camera {
        let e = if self.valid {
            *self
        } else {
            ProjectedExtent {
                min_x: -1.0,
                min_y: -1.0,
                min_z: -1.0,
                max_x: 1.0,
                max_y: 1.0,
                max_z: 1.0,
                valid: true,
            }
        };
        let (w, h) = (
            go_max(0.02, e.max_x - e.min_x),
            go_max(0.02, e.max_y - e.min_y),
        );
        if fit {
            // Preserve the maximum image budget and source geometry.
            let scale = go_min(width as f64 / w, height as f64 / h);
            width = 64.max(width.min(gomath::ceil(w * scale) as i64));
            height = 64.max(height.min(gomath::ceil(h * scale) as i64));
        }
        let mut c = native_camera_axes();
        c.width = width;
        c.height = height;
        c.center_x = (e.min_x + e.max_x) / 2.0;
        c.center_y = (e.min_y + e.max_y) / 2.0;
        c.center_depth = (e.min_z + e.max_z) / 2.0;
        c.scale = go_min(width as f64 / w, height as f64 / h) / 1.08;
        c
    }
}

pub fn native_camera_axes() -> Camera {
    let direction = raster_unit(native_view_direction());
    let right = raster_unit(ncross(nv(0.0, 1.0, 0.0), direction));
    Camera {
        direction,
        right,
        up: raster_unit(ncross(direction, right)),
        ..Default::default()
    }
}

pub fn camera_at_resolution(mut c: Camera, width: i64, height: i64) -> Camera {
    let (sx, sy) = (
        width as f64 / c.width as f64,
        height as f64 / c.height as f64,
    );
    c.scale *= go_min(sx, sy);
    c.width = width;
    c.height = height;
    c
}

/// Before textures exist, source geometry estimates their output sampling
/// rate.
pub fn scene_output_density(s: &Scene, width: i64, height: i64, camera: Option<&Camera>) -> f64 {
    if let Some(camera) = camera {
        return camera.scale;
    }
    let mut e = ProjectedExtent::default();
    for board in &s.boards {
        if board.kind == "ungrouped" {
            continue;
        }
        for x in [-0.5, 0.5] {
            for z in [-0.5, 0.5] {
                e.add(nadd(
                    board.position,
                    nv(x * board.size.x, 0.0, z * board.size.z),
                ));
            }
        }
    }
    for n in &s.nodes {
        if n.opacity <= 0.0 {
            continue;
        }
        let floor = n.position.y - n.size.y / 2.0;
        // ARM64 framing.go:228.
        let mut h = go_fmadd(go_max(0.3, n.size.y), 1.3, 0.2);
        if native_solid_node(n) {
            h = native_solid_height(n) + 0.002;
        } else if native_relief_symbol(n) || native_structured_node(n) {
            h = native_canonical_height(n, s.pixel_scale) + 0.002;
        }
        let relief = hierarchy_node_relief(n);
        // ARM64 framing.go:237: floor+h fuses h*relief.
        let top = go_fmadd(h, relief, floor);
        for x in [-0.5, 0.5] {
            for z in [-0.5, 0.5] {
                for y in [floor, top] {
                    e.add(nv(
                        n.position.x + x * n.size.x,
                        y,
                        n.position.z + z * n.size.z,
                    ));
                }
            }
        }
    }
    for edge in &s.edges {
        for p in edge.points.iter().flatten() {
            e.add(*p);
        }
    }
    e.camera(width, height, false).scale
}
