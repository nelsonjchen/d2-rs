// SPDX-License-Identifier: MPL-2.0
//! arrow_visibility.go: endpoint-owned caps and bounded swept marker queries.
use super::{
    geometry::{MeshBuilder, native_material, native_paint},
    geometry_routes::route_lengths,
    hierarchy_boards::{hierarchy_node_relief, hierarchy_surface_y},
    markdown_cards::{native_markdown_card, native_markdown_card_paint},
    mesh::*,
    native_shapes::{native_canonical_height, native_face_source, native_shape_profiles},
    native_solids::native_solid_node,
    native_structured::{native_structured_node, structured_rows},
    native_style::native_classic_node,
    route_labels::{RouteCaptionPoint, caption_convex_hull, caption_projection},
};
use crate::scene::{Board, Edge, Node};
use d2_rs_core::go_sort;
use d2_rs_scene::{go_max, go_min};
use std::collections::HashMap;
#[derive(Clone)]
pub struct NativeArrowOwner {
    first: usize,
    last: usize,
    node: Node,
    board: Option<Board>,
    caps: Option<std::vec::Vec<Triangle>>,
    obstacles: HashMap<u64, std::vec::Vec<Obstacle>>,
}
#[derive(Clone, Default)]
pub struct NativeArrowMarker {
    vertices: std::vec::Vec<Vec>,
    min_y: f64,
    reach: f64,
}
#[derive(Clone)]
struct Obstacle {
    points: std::vec::Vec<RouteCaptionPoint>,
    lo: RouteCaptionPoint,
    hi: RouteCaptionPoint,
}
#[derive(Clone, Copy)]
struct Interval {
    first: f64,
    last: f64,
}
fn hull(mut points: std::vec::Vec<RouteCaptionPoint>) -> std::vec::Vec<RouteCaptionPoint> {
    caption_convex_hull(&mut points)
}
fn bounds(points: &[RouteCaptionPoint]) -> (RouteCaptionPoint, RouteCaptionPoint) {
    let (mut lo, mut hi) = (
        RouteCaptionPoint {
            x: f64::INFINITY,
            z: f64::INFINITY,
        },
        RouteCaptionPoint {
            x: f64::NEG_INFINITY,
            z: f64::NEG_INFINITY,
        },
    );
    for p in points {
        lo.x = go_min(lo.x, p.x);
        lo.z = go_min(lo.z, p.z);
        hi.x = go_max(hi.x, p.x);
        hi.z = go_max(hi.z, p.z)
    }
    (lo, hi)
}
impl MeshBuilder {
    fn arrow_budget(&mut self, work: usize) -> bool {
        if self.err.is_some() {
            return false;
        }
        if let Err(error) = self.context.check_string() {
            self.fail(error);
            return false;
        }
        self.arrow_work += work;
        if self.arrow_work > 100000000 {
            self.fail("isometric endpoint visibility exceeds bounded query work");
            return false;
        }
        true
    }
    pub fn remember_arrow_owner(&mut self, node: &Node, board: Option<&Board>, first: usize) {
        if !node.sequence_role.is_empty() || node.opacity <= 0.0 {
            return;
        }
        self.arrow_owners.insert(
            node.id.clone(),
            NativeArrowOwner {
                first,
                last: self.triangles.len(),
                node: node.clone(),
                board: board.cloned(),
                caps: None,
                obstacles: HashMap::new(),
            },
        );
    }
    fn arrow_marker(&mut self, kind: &str, width: i64) -> NativeArrowMarker {
        let key = (kind.to_owned(), width);
        if let Some(m) = self.arrow_markers.get(&key) {
            return m.clone();
        }
        let mut q = MeshBuilder {
            scale: self.scale,
            arrow_background: self.arrow_background.clone(),
            ..Default::default()
        };
        q.context = self.context.clone();
        q.arrow_with_opacity(
            kind,
            Vec::default(),
            nv(1.0, 0.0, 0.0),
            &native_material("black", 0.0, 0.0, 1.0),
            width,
            1.0,
        );
        if let Some(e) = q.err.take() {
            self.fail(e)
        }
        let mut marker = NativeArrowMarker {
            min_y: f64::INFINITY,
            ..Default::default()
        };
        for t in &q.triangles {
            for v in t.v {
                marker.vertices.push(v.position);
                marker.min_y = go_min(
                    marker.min_y,
                    t.depth_bias
                        .mul_add(native_view_direction().y, v.position.y),
                );
                marker.reach = go_max(marker.reach, -v.position.x)
            }
        }
        if self.arrow_markers.len() < 64 {
            self.arrow_markers.insert(key, marker.clone());
        }
        marker
    }
    fn arrow_owner_caps(&mut self, o: &mut NativeArrowOwner) -> std::vec::Vec<Triangle> {
        if let Some(caps) = &o.caps {
            return caps.clone();
        }
        o.caps = Some(vec![]);
        let mut n = native_classic_node(&o.node);
        if native_markdown_card(&n) {
            n = native_markdown_card_paint(&n)
        }
        if o.board.is_none() && native_solid_node(&n) {
            return vec![];
        }
        let scale = if self.scale <= 0.0 { 0.01 } else { self.scale };
        let mut q = MeshBuilder::default();
        q.context = self.context.clone();
        let opacity = n.opacity;
        let no_side = material(Material::default());
        let mut add =
            |s: crate::target::Shape, x: f64, z: f64, w: f64, d: f64, y: f64, fill: &str| {
                if native_paint(fill, "#dce5eb").a == 0 || w <= 0.0 || d <= 0.0 {
                    return;
                }
                let ps = match native_shape_profiles(&s) {
                    Ok(p) => p,
                    Err(e) => {
                        self.fail(e);
                        return;
                    }
                };
                let m = native_material(fill, 0.0, 0.0, opacity);
                for p in ps {
                    let world: std::vec::Vec<_> = p
                        .iter()
                        .map(|v| {
                            nv(
                                x + v.x * w / s.width as f64,
                                y,
                                z + v.z * d / s.height as f64,
                            )
                        })
                        .collect();
                    q.extruded_profile(&world, y, Some(&m), &no_side);
                }
            };
        let floor = n.position.y - n.size.y / 2.0;
        let height = native_canonical_height(&n, scale);
        for copy in (0..=1).rev() {
            if copy == 1 && !n.metadata.original.multiple {
                continue;
            }
            let (mut dx, mut dz, dy) = (
                copy as f64 * 10.0 * scale,
                -(copy as f64) * 10.0 * scale,
                -(copy as f64) * go_min(0.045, height * 0.25),
            );
            if let Some(board) = &o.board {
                n.size = board.size;
                let s = native_face_source(&n, &n.fill);
                add(
                    s,
                    (-0.5f64).mul_add(board.size.x, board.position.x) + dx,
                    (-0.5f64).mul_add(board.size.z, board.position.z) + dz,
                    board.size.x,
                    board.size.z,
                    hierarchy_surface_y(board) - copy as f64 * 0.00001,
                    &n.fill,
                )
            } else if native_structured_node(&n) {
                let s = &n.metadata.original;
                let (sx, sz) = (n.size.x / s.width as f64, n.size.z / s.height as f64);
                dx = copy as f64 * 10.0 * sx;
                dz = -(copy as f64) * 10.0 * sz;
                let mut rail = |z: f64, depth: f64, top: f64, fill: &str| {
                    if depth <= 0.0 {
                        return;
                    }
                    let mut face = native_face_source(&n, fill);
                    face.type_ = "rectangle".into();
                    face.width = s.width;
                    face.height = 1.max(depth.round() as i64);
                    face.border_radius = s.border_radius.min(face.height / 2);
                    add(
                        face,
                        (-0.5f64).mul_add(n.size.x, n.position.x) + dx,
                        z.mul_add(sz, (-0.5f64).mul_add(n.size.z, n.position.z) + dz),
                        n.size.x,
                        depth * sz,
                        top + dy,
                        fill,
                    )
                };
                rail(0.0, s.height as f64, floor + 0.01, &n.fill);
                for (i, r) in structured_rows(s).iter().enumerate() {
                    let (top, fill) = if i == 0 {
                        (floor + height, &n.fill)
                    } else {
                        (floor + height * 0.32 / 0.46, &n.stroke)
                    };
                    rail(r.z + r.back, r.depth - r.back - r.front, top, fill)
                }
            } else {
                let s = native_face_source(&n, &n.fill);
                let y = (height + dy).mul_add(hierarchy_node_relief(&n), floor);
                add(
                    s,
                    (-0.5f64).mul_add(n.size.x, n.position.x) + dx,
                    (-0.5f64).mul_add(n.size.z, n.position.z) + dz,
                    n.size.x,
                    n.size.z,
                    y,
                    &n.fill,
                )
            }
        }
        if let Some(e) = q.err.take() {
            self.fail(e)
        }
        o.caps = Some(q.triangles.clone());
        q.geometry.triangles
    }
    fn arrow_obstacles(&mut self, id: &str, minimum_y: f64) -> std::vec::Vec<Obstacle> {
        let Some(mut o) = self.arrow_owners.remove(id) else {
            return vec![];
        };
        if let Some(p) = o.obstacles.get(&minimum_y.to_bits()) {
            let p = p.clone();
            self.arrow_owners.insert(id.into(), o);
            return p;
        }
        let caps = self.arrow_owner_caps(&mut o);
        let triangles: std::vec::Vec<_> = self.triangles[o.first..o.last]
            .iter()
            .cloned()
            .chain(caps)
            .collect();
        let mut polygons = vec![];
        let mut count = 0;
        for t in triangles {
            if !self.arrow_budget(1) {
                continue;
            }
            let Some(m) = &t.material else { continue };
            let m = m.borrow();
            if m.color.a == 0 || t.no_depth_write || t.svg_coverage_only {
                continue;
            }
            if m.texture.is_some()
                && (o.board.is_some()
                    || !native_solid_node(&o.node)
                    || m.unlit
                    || native_paint(&o.node.fill, "#dce5eb").a == 0)
            {
                continue;
            }
            let p = occluder(&t, minimum_y);
            if p.len() > 2 {
                let (lo, hi) = bounds(&p);
                count += p.len();
                polygons.push(Obstacle { points: p, lo, hi })
            }
        }
        if o.obstacles.len() < 8 && count <= 1000000 - self.arrow_cache_points {
            o.obstacles.insert(minimum_y.to_bits(), polygons.clone());
            self.arrow_cache_points += count;
        }
        self.arrow_owners.insert(id.into(), o);
        polygons
    }
    fn arrow_retreat(
        &mut self,
        id: &str,
        marker: &NativeArrowMarker,
        points: &[Vec],
        limit: f64,
    ) -> f64 {
        if marker.vertices.is_empty() || points.len() < 2 || limit <= 0.0 {
            return 0.0;
        }
        let obstacles = self.arrow_obstacles(id, points[0].y + marker.min_y);
        if obstacles.is_empty() {
            return 0.0;
        }
        let mut distance = 0.0;
        for i in 1..points.len() {
            let delta = nsub(points[i], points[i - 1]);
            let length = nlen(delta);
            if length < 1e-9 {
                continue;
            }
            if delta.y.abs() > 1e-7 || (points[i].y - points[0].y).abs() > 1e-7 {
                return distance;
            }
            let available = go_min(length, limit - distance);
            if available <= 0.0 {
                return distance;
            }
            let away = nmul(delta, 1.0 / length);
            let along = nmul(away, -1.0);
            let side = nv(-along.z, 0.0, along.x);
            let footprint = hull(
                marker
                    .vertices
                    .iter()
                    .map(|v| {
                        let mut p = nadd(points[i - 1], nadd(nmul(along, v.x), nmul(side, v.z)));
                        p.y += v.y;
                        caption_projection(p)
                    })
                    .collect(),
            );
            let (mut lo, mut hi) = bounds(&footprint);
            lo.x += go_min(0.0, away.x * available);
            lo.z += go_min(0.0, away.z * available);
            hi.x += go_max(0.0, away.x * available);
            hi.z += go_max(0.0, away.z * available);
            let mut blocked = vec![];
            for p in &obstacles {
                if !self.arrow_budget(1) {
                    return 0.0;
                }
                if lo.x > p.hi.x || hi.x < p.lo.x || lo.z > p.hi.z || hi.z < p.lo.z {
                    continue;
                }
                let count = footprint.len() + p.points.len();
                if !self.arrow_budget(count * count) {
                    return 0.0;
                }
                if let Some(v) = overlap(
                    &footprint,
                    &p.points,
                    RouteCaptionPoint {
                        x: away.x,
                        z: away.z,
                    },
                    available,
                ) {
                    blocked.push(v)
                }
            }
            go_sort::sort_by(&mut blocked, |a, b| a.first < b.first);
            let mut clear = 0.0;
            for v in blocked {
                if v.first > clear + 0.0008 {
                    break;
                }
                clear = go_max(clear, v.last + 0.0008)
            }
            if clear <= available {
                return distance + clear;
            }
            distance += available;
            if available < length || distance >= limit {
                return distance;
            }
        }
        distance
    }
    pub fn visible_arrow_range(&mut self, e: &Edge, points: &[Vec]) -> (f64, f64) {
        if self.arrow_owners.is_empty() || points.len() < 2 || !self.arrow_budget(1) {
            return (0.0, 1.0);
        }
        let (source, target) = (
            self.arrow_marker(&e.source_arrow, e.stroke_width),
            self.arrow_marker(&e.target_arrow, e.stroke_width),
        );
        if source.vertices.is_empty() && target.vertices.is_empty() {
            return (0.0, 1.0);
        }
        let lengths = route_lengths(points);
        let total = lengths[lengths.len() - 1];
        let limit = go_max(0.0, total - source.reach - target.reach - 0.002);
        let mut start = self.arrow_retreat(&e.source, &source, points, limit);
        let reversed: std::vec::Vec<_> = points.iter().rev().copied().collect();
        let mut end = self.arrow_retreat(&e.target, &target, &reversed, limit);
        if start + end > limit && start + end > 0.0 {
            let factor = limit / (start + end);
            start *= factor;
            end *= factor
        }
        if start <= 1e-9 && end <= 1e-9 {
            return (0.0, 1.0);
        }
        (start / total, 1.0 - end / total)
    }
}
fn occluder(t: &Triangle, minimum_y: f64) -> std::vec::Vec<RouteCaptionPoint> {
    let minimum_y = (-t.depth_bias).mul_add(native_view_direction().y, minimum_y);
    let p = t.v.map(|v| v.position);
    let mut out = vec![];
    for (i, &a) in p.iter().enumerate() {
        let c = p[(i + 1) % 3];
        let (ain, cin) = (a.y > minimum_y + 1e-6, c.y > minimum_y + 1e-6);
        if ain {
            out.push(a)
        }
        if ain != cin {
            out.push(nlerp(a, c, (minimum_y + 1e-6 - a.y) / (c.y - a.y)))
        }
    }
    if out.len() < 3 {
        return vec![];
    }
    hull(out.into_iter().map(caption_projection).collect())
}
fn overlap(
    marker: &[RouteCaptionPoint],
    obstacle: &[RouteCaptionPoint],
    velocity: RouteCaptionPoint,
    length: f64,
) -> Option<Interval> {
    let mut interval = Interval {
        first: 0.0,
        last: length,
    };
    for poly in [marker, obstacle] {
        for (i, a) in poly.iter().enumerate() {
            let c = poly[(i + 1) % poly.len()];
            let axis = RouteCaptionPoint {
                x: -(c.z - a.z),
                z: c.x - a.x,
            };
            if axis.x.abs() + axis.z.abs() < 1e-12 {
                continue;
            }
            let bounds = |points: &[RouteCaptionPoint]| {
                let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
                for p in points {
                    let v = p.z.mul_add(axis.z, p.x * axis.x);
                    lo = go_min(lo, v);
                    hi = go_max(hi, v)
                }
                (lo, hi)
            };
            let (al, ah) = bounds(marker);
            let (bl, bh) = bounds(obstacle);
            let speed = velocity.z.mul_add(axis.z, velocity.x * axis.x);
            if speed.abs() < 1e-12 {
                if ah < bl || bh < al {
                    return None;
                }
                continue;
            }
            let (mut enter, mut exit) = ((bl - ah) / speed, (bh - al) / speed);
            if enter > exit {
                std::mem::swap(&mut enter, &mut exit)
            }
            interval.first = go_max(interval.first, enter);
            interval.last = go_min(interval.last, exit);
            if interval.first > interval.last {
                return None;
            }
        }
    }
    Some(interval)
}

#[cfg(test)]
#[path = "arrow_visibility_api_tests.rs"]
mod api_tests;
