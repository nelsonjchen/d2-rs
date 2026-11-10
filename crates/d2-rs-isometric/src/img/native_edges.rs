// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg native_edges.go and native_ink_joins.go: classic outline
//! ink traced from a component's physical mesh. Go rounds every product here
//! explicitly (float64(...)), so there are no contraction sites.

use d2_rs_core::stable_sort::StableSort;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use d2_rs_core::go_sort;
use d2_rs_render::go_float::gomath;
use d2_rs_scene::{go_max, go_min};
use d2_rs_scenebuild::golib::label::Position;
use d2_rs_scenebuild::golib::svgpath::get_stroke_dash_attributes;

use super::context::RenderContext;
use super::framing::native_camera_axes;
use super::geometry::{MeshBuilder, native_material, native_paint};
use super::mesh::{
    Camera, MaterialRef, Triangle, Vec, Vertex, nadd, ncross, ndot, nlen, nlerp, nmul, nsub, nunit,
    nv,
};
use super::native_solids::native_solid_node;
use super::native_style::native_classic_node;
use super::route_labels::caption_finite;
use crate::scene::Node;
use crate::target::{
    SHAPE_CLASS, SHAPE_CODE, SHAPE_IMAGE, SHAPE_QUEUE, SHAPE_SQL_TABLE, SHAPE_TEXT,
};

/// A plain contour is owned by geometry. Decorative source borders retain
/// their texture.
pub fn native_classic_rim(n: &Node) -> bool {
    let s = &n.metadata.original;
    if n.opacity < 1.0
        || native_paint(&n.stroke, "#263c4e").a < 255
        || s.double_border
        || n.stroke_dash != 0.0
        || !s.text.label.is_empty() && Position::from_string(&s.label_position).is_border()
    {
        return false;
    }
    if matches!(
        n.type_.as_str(),
        SHAPE_CLASS | SHAPE_SQL_TABLE | SHAPE_TEXT | SHAPE_CODE | SHAPE_IMAGE
    ) {
        return false;
    }
    // A transparent body has no physical walls from which to recover its rim.
    !n.fill.eq_ignore_ascii_case("none") && native_paint(&n.fill, "#dce5eb").a != 0
}

const CLASSIC_INK_WELD: f64 = 1e-6;
const MAX_CLASSIC_INK_TRIANGLES: usize = 100_000;
const MAX_CLASSIC_INK_SEGMENTS: usize = 100_000;
const MAX_CLASSIC_INK_SUPPORT_FACES: usize = 256;
const MAX_CLASSIC_INK_SUPPORT_WORK: usize = 5_000_000;
const CLASSIC_INK_DEPTH_BIAS: f64 = 0.001;

pub type ClassicInkKey = [i64; 3];
type ClassicInkPair = (ClassicInkKey, ClassicInkKey);

#[derive(Clone, Copy)]
struct ClassicInkFace {
    normal: Vec,
    third: Vec,
    cap: bool,
    edge_normals: [Vec; 2],
    interpolated: bool,
}

struct ClassicInkEdge {
    a: Vec,
    c: Vec,
    faces: std::vec::Vec<ClassicInkFace>,
}

#[derive(Clone)]
pub struct ClassicInkSegment {
    pub a: Vec,
    pub c: Vec,
    pub support: std::vec::Vec<Rc<ClassicInkFacet>>,
}

/// Projected finite supporting faces keep centered ink on its own surface.
pub struct ClassicInkFacet {
    pub points: [Vec; 3],
    pub bias: f64,
    pub keys: [ClassicInkKey; 3],
    pub normal: Vec,
}

impl ClassicInkFacet {
    pub fn depth_at(&self, p: Vec) -> Option<f64> {
        let [a, c, d] = self.points;
        let denom = (c.y - d.y) * (a.x - d.x) + (d.x - c.x) * (a.y - d.y);
        if denom.abs() < 1e-14 {
            return None;
        }
        let u = ((c.y - d.y) * (p.x - d.x) + (d.x - c.x) * (p.y - d.y)) / denom;
        let v = ((d.y - a.y) * (p.x - d.x) + (a.x - d.x) * (p.y - d.y)) / denom;
        let w = 1.0 - u - v;
        if u < -1e-8 || v < -1e-8 || w < -1e-8 {
            return None;
        }
        Some(u * a.z + v * c.z + w * d.z + self.bias)
    }
}

pub fn classic_ink_key_of(p: Vec) -> ClassicInkKey {
    [
        (p.x / CLASSIC_INK_WELD).round() as i64,
        (p.y / CLASSIC_INK_WELD).round() as i64,
        (p.z / CLASSIC_INK_WELD).round() as i64,
    ]
}

pub fn classic_ink_less(a: &ClassicInkKey, c: &ClassicInkKey) -> bool {
    a < c
}

fn max3(a: f64, b: f64, c: f64) -> f64 {
    go_max(go_max(a, b), c)
}
fn min3(a: f64, b: f64, c: f64) -> f64 {
    go_min(go_min(a, b), c)
}

/// Weld only positions: lighting-normal and UV seams must not become ink.
pub fn classic_ink_segments(
    n: &Node,
    triangles: &[Triangle],
    ink_radius: f64,
) -> Result<std::vec::Vec<ClassicInkSegment>, String> {
    classic_ink_segments_with_context(&RenderContext::default(), n, triangles, ink_radius)
}

/// Source context checks are part of outline extraction, including support
/// adjacency walks. Keep the background convenience entry for direct callers.
pub fn classic_ink_segments_with_context(
    ctx: &RenderContext,
    n: &Node,
    triangles: &[Triangle],
    ink_radius: f64,
) -> Result<std::vec::Vec<ClassicInkSegment>, String> {
    if triangles.len() > MAX_CLASSIC_INK_TRIANGLES {
        return Err(format!(
            "isometric outline exceeds {MAX_CLASSIC_INK_TRIANGLES} input triangles"
        ));
    }
    let camera = native_camera_axes();
    let view = camera.direction;
    let mut edges: HashMap<ClassicInkPair, ClassicInkEdge> = HashMap::new();
    // Go iterates this map in random order before an unstable sort; the
    // sorted keys only differ from Go's order on exact key ties.
    let mut contours: BTreeMap<ClassicInkPair, ClassicInkSegment> = BTreeMap::new();
    let mut supports: HashMap<ClassicInkKey, std::vec::Vec<Rc<ClassicInkFacet>>> = HashMap::new();
    let mut contour_anchors: HashMap<ClassicInkPair, [ClassicInkKey; 3]> = HashMap::new();
    let mut caps: std::vec::Vec<(f64, Rc<ClassicInkFacet>)> = vec![];
    let project = |p: Vec| nv(ndot(p, camera.right), ndot(p, camera.up), ndot(p, view));
    let solid = native_solid_node(n);
    for (i, t) in triangles.iter().enumerate() {
        if i % 1024 == 0 {
            ctx.check_string()?;
        }
        let Some(m) = &t.material else { continue };
        let m = m.borrow();
        if m.color.a == 0 || t.no_depth_write || !t.cast_shadow {
            continue;
        }
        let (a, c, d) = (t.v[0].position, t.v[1].position, t.v[2].position);
        let mut normal = ncross(nsub(c, a), nsub(d, a));
        if nlen(normal) < 1e-12 {
            continue;
        }
        normal = nunit(normal);
        if ndot(
            normal,
            nadd(nadd(t.v[0].normal, t.v[1].normal), t.v[2].normal),
        ) < 0.0
        {
            normal = nmul(normal, -1.0);
        }
        let facet = Rc::new(ClassicInkFacet {
            points: [project(a), project(c), project(d)],
            bias: t.depth_bias,
            keys: [
                classic_ink_key_of(a),
                classic_ink_key_of(c),
                classic_ink_key_of(d),
            ],
            normal,
        });
        for p in [a, c, d] {
            supports
                .entry(classic_ink_key_of(p))
                .or_default()
                .push(facet.clone());
        }
        // Canonical caps are transparent texture quads; their open sidewall
        // boundary supplies the exact outline.
        if !solid && m.texture.is_some() && normal.y.abs() > 0.999999 {
            caps.push((a.y, facet));
            continue;
        }
        if t.depth_bias != 0.0 {
            continue;
        }
        let cap = solid
            && ((n.type_ == SHAPE_QUEUE && normal.x.abs() > 0.999999)
                || (n.type_ != SHAPE_QUEUE && normal.y > 0.999999));
        let points = [a, c, d];
        let interpolated = ndot(t.v[0].normal, t.v[1].normal) < 1.0 - 1e-8
            && ndot(t.v[0].normal, t.v[2].normal) < 1.0 - 1e-8
            && ndot(t.v[1].normal, t.v[2].normal) < 1.0 - 1e-8;
        if interpolated {
            let mut crossings: std::vec::Vec<Vec> = vec![];
            let add = |crossings: &mut std::vec::Vec<Vec>, p: Vec| {
                if crossings
                    .iter()
                    .any(|q| classic_ink_key_of(p) == classic_ink_key_of(*q))
                {
                    return;
                }
                crossings.push(p);
            };
            for (j, p) in points.iter().enumerate() {
                let k = (j + 1) % 3;
                let (f, g) = (ndot(t.v[j].normal, view), ndot(t.v[k].normal, view));
                if f.abs() < 1e-10 {
                    add(&mut crossings, *p);
                }
                if f * g < 0.0 {
                    add(&mut crossings, nlerp(*p, points[k], f / (f - g)));
                }
            }
            if crossings.len() == 2 {
                let (mut p, mut q) = (crossings[0], crossings[1]);
                let (mut pk, mut qk) = (classic_ink_key_of(p), classic_ink_key_of(q));
                if classic_ink_less(&qk, &pk) {
                    std::mem::swap(&mut p, &mut q);
                    std::mem::swap(&mut pk, &mut qk);
                }
                if pk != qk {
                    contours.insert(
                        (pk, qk),
                        ClassicInkSegment {
                            a: p,
                            c: q,
                            support: vec![],
                        },
                    );
                    contour_anchors.insert(
                        (pk, qk),
                        [
                            classic_ink_key_of(a),
                            classic_ink_key_of(c),
                            classic_ink_key_of(d),
                        ],
                    );
                }
            }
        }
        for (j, p) in points.iter().enumerate() {
            let (mut p, mut q) = (*p, points[(j + 1) % 3]);
            let (mut pn, mut qn) = (t.v[j].normal, t.v[(j + 1) % 3].normal);
            let (mut pk, mut qk) = (classic_ink_key_of(p), classic_ink_key_of(q));
            if pk == qk {
                continue;
            }
            if classic_ink_less(&qk, &pk) {
                std::mem::swap(&mut pk, &mut qk);
                std::mem::swap(&mut p, &mut q);
                std::mem::swap(&mut pn, &mut qn);
            }
            let e = edges.entry((pk, qk)).or_insert_with(|| ClassicInkEdge {
                a: p,
                c: q,
                faces: vec![],
            });
            // Nonmanifold/overlaid faces are not additional structural edges.
            if e.faces.len() < 3 {
                e.faces.push(ClassicInkFace {
                    normal,
                    third: points[(j + 2) % 3],
                    cap,
                    edge_normals: [pn, qn],
                    interpolated,
                });
            }
        }
    }
    let mut keys: std::vec::Vec<ClassicInkPair> = edges.keys().copied().collect();
    keys.stable_sort();
    let full_rim = native_classic_rim(n);
    let radius = if ink_radius > 0.0 && caption_finite(&[ink_radius]) {
        ink_radius
    } else {
        0.0
    };
    let mut work = 0usize;
    let mut support_err: Option<String> = None;
    let cos30 = gomath::cos(std::f64::consts::PI / 6.0) - 1e-8;
    let mut support_for = |keys: &[ClassicInkKey],
                           top_y: f64,
                           a: Vec,
                           c: Vec,
                           support_err: &mut Option<String>|
     -> Option<std::vec::Vec<Rc<ClassicInkFacet>>> {
        let mut seen: HashSet<*const ClassicInkFacet> = HashSet::new();
        let mut faces: std::vec::Vec<Rc<ClassicInkFacet>> = vec![];
        let add = |faces: &mut std::vec::Vec<Rc<ClassicInkFacet>>,
                   seen: &mut HashSet<*const ClassicInkFacet>,
                   f: &Rc<ClassicInkFacet>|
         -> bool {
            if seen.contains(&Rc::as_ptr(f)) {
                return true;
            }
            if faces.len() >= MAX_CLASSIC_INK_SUPPORT_FACES {
                return false;
            }
            seen.insert(Rc::as_ptr(f));
            faces.push(f.clone());
            true
        };
        for key in keys {
            for f in supports.get(key).into_iter().flatten() {
                if !add(&mut faces, &mut seen, f) {
                    return None;
                }
            }
        }
        // Walk only the local, finite, smoothly connected surface within the
        // ribbon's bounds.
        if radius > 0.0 {
            let (p, q) = (project(a), project(c));
            let (lo_x, lo_y) = (
                go_min(p.x, q.x) - 2.0 * radius,
                go_min(p.y, q.y) - 2.0 * radius,
            );
            let (hi_x, hi_y) = (
                go_max(p.x, q.x) + 2.0 * radius,
                go_max(p.y, q.y) + 2.0 * radius,
            );
            let mut at = 0;
            while at < faces.len() {
                let face = faces[at].clone();
                for key in &face.keys {
                    for next in supports.get(key).into_iter().flatten() {
                        work += 1;
                        if work.is_multiple_of(1024) {
                            if let Err(error) = ctx.check_string() {
                                *support_err = Some(error);
                                return None;
                            }
                        }
                        if work > MAX_CLASSIC_INK_SUPPORT_WORK {
                            *support_err = Some(format!(
                                "isometric outline support exceeds {MAX_CLASSIC_INK_SUPPORT_WORK} adjacency checks"
                            ));
                            return None;
                        }
                        if seen.contains(&Rc::as_ptr(next))
                            || ndot(face.normal, next.normal) < cos30
                        {
                            continue;
                        }
                        let [u, v, w] = next.points;
                        if max3(u.x, v.x, w.x) < lo_x
                            || min3(u.x, v.x, w.x) > hi_x
                            || max3(u.y, v.y, w.y) < lo_y
                            || min3(u.y, v.y, w.y) > hi_y
                        {
                            continue;
                        }
                        if !add(&mut faces, &mut seen, next) {
                            return None;
                        }
                    }
                }
                at += 1;
            }
        }
        for (y, facet) in &caps {
            if *y >= top_y - 1e-8 && *y <= top_y + 0.00051 && !add(&mut faces, &mut seen, facet) {
                return None;
            }
        }
        Some(faces)
    };
    let too_many = || {
        format!("isometric outline exceeds {MAX_CLASSIC_INK_SUPPORT_FACES} local supporting faces")
    };
    let mut result: std::vec::Vec<ClassicInkSegment> = vec![];
    for (i, k) in keys.iter().enumerate() {
        if i % 1024 == 0 {
            ctx.check_string()?;
        }
        let e = &edges[k];
        if e.faces.len() > 2 {
            continue;
        }
        let (mut front, mut back, mut painted_rim) = (false, false, false);
        for f in &e.faces {
            let dot = ndot(f.normal, view);
            front = front || dot > 1e-7;
            back = back || dot < -1e-7;
            painted_rim = painted_rim || f.cap;
        }
        let top_boundary = e.faces.len() == 1
            && (e.a.y - e.c.y).abs() < CLASSIC_INK_WELD
            && e.faces[0].third.y < e.a.y - CLASSIC_INK_WELD;
        if !full_rim && (top_boundary || painted_rim) {
            continue;
        }
        let smooth = e.faces.len() == 2
            && ndot(e.faces[0].edge_normals[0], e.faces[1].edge_normals[0]) > 1.0 - 1e-6
            && ndot(e.faces[0].edge_normals[1], e.faces[1].edge_normals[1]) > 1.0 - 1e-6;
        let interpolated = e.faces.len() == 2 && e.faces[0].interpolated && e.faces[1].interpolated;
        let mut visible = (e.faces.len() == 1 && (front || top_boundary))
            || front && back && !(smooth && interpolated);
        if e.faces.len() == 2
            && !smooth
            && front
            && ndot(e.faces[0].normal, e.faces[1].normal) < cos30
        {
            visible = true;
        }
        if visible {
            let top_y = if top_boundary { e.a.y } else { f64::INFINITY };
            let Some(support) = support_for(&[k.0, k.1], top_y, e.a, e.c, &mut support_err) else {
                return Err(support_err.unwrap_or_else(too_many));
            };
            result.push(ClassicInkSegment {
                a: e.a,
                c: e.c,
                support,
            });
            if result.len() > MAX_CLASSIC_INK_SEGMENTS {
                return Err(format!(
                    "isometric outline exceeds {MAX_CLASSIC_INK_SEGMENTS} segments"
                ));
            }
        }
    }
    for (key, mut s) in contours {
        let anchors = contour_anchors[&key];
        let Some(support) = support_for(&anchors, f64::INFINITY, s.a, s.c, &mut support_err) else {
            return Err(support_err.unwrap_or_else(too_many));
        };
        s.support = support;
        result.push(s);
    }
    if result.len() > MAX_CLASSIC_INK_SEGMENTS {
        return Err(format!(
            "isometric outline exceeds {MAX_CLASSIC_INK_SEGMENTS} segments"
        ));
    }
    go_sort::sort_by(&mut result, |x, y| {
        let (a, c) = (classic_ink_key_of(x.a), classic_ink_key_of(y.a));
        if a != c {
            return classic_ink_less(&a, &c);
        }
        classic_ink_less(&classic_ink_key_of(x.c), &classic_ink_key_of(y.c))
    });
    Ok(result)
}

/// Trace deterministic chains so a dash never restarts at a triangulation
/// vertex.
pub fn classic_ink_paths(
    segments: &[ClassicInkSegment],
) -> std::vec::Vec<std::vec::Vec<ClassicInkSegment>> {
    let mut adjacent: HashMap<ClassicInkKey, std::vec::Vec<usize>> = HashMap::new();
    for (i, s) in segments.iter().enumerate() {
        adjacent.entry(classic_ink_key_of(s.a)).or_default().push(i);
        adjacent.entry(classic_ink_key_of(s.c)).or_default().push(i);
    }
    let mut vertices: std::vec::Vec<ClassicInkKey> = adjacent.keys().copied().collect();
    vertices.stable_sort();
    let mut used = vec![false; segments.len()];
    let mut paths = vec![];
    let mut walk = |start: ClassicInkKey, first: usize, used: &mut std::vec::Vec<bool>| {
        let mut path = vec![];
        let (mut at, mut index) = (start, first);
        while !used[index] {
            used[index] = true;
            let mut s = segments[index].clone();
            if classic_ink_key_of(s.a) != at {
                std::mem::swap(&mut s.a, &mut s.c);
            }
            at = classic_ink_key_of(s.c);
            path.push(s);
            let next = &adjacent[&at];
            if next.len() != 2 {
                break;
            }
            index = next[0];
            if used[index] {
                index = next[1];
            }
        }
        paths.push(path);
    };
    for p in &vertices {
        if adjacent[p].len() == 2 {
            continue;
        }
        for &i in &adjacent[p] {
            if !used[i] {
                walk(*p, i, &mut used);
            }
        }
    }
    for (i, s) in segments.iter().enumerate() {
        if !used[i] {
            walk(classic_ink_key_of(s.a), i, &mut used);
        }
    }
    paths
}

impl MeshBuilder {
    /// Invoke after hierarchy transforms; `first` starts the component's
    /// triangles (Go passes b.triangles[first:] as of the call).
    pub fn classic_ink_edges(&mut self, n: &Node, first: usize) {
        if self.err.is_some() {
            return;
        }
        let triangles = self.triangles[first..].to_vec();
        // Source captions remain the final triangles of a node. Insert the
        // ink before those decals without changing the physical mesh order.
        let before = self.triangles.len();
        let trailing = triangles
            .iter()
            .rev()
            .take_while(|t| {
                t.no_depth_write
                    && t.material
                        .as_ref()
                        .is_some_and(|m| m.borrow().texture.is_some())
            })
            .count();
        self.classic_ink_edges_body(n, &triangles);
        if trailing == 0 || self.triangles.len() == before {
            return;
        }
        // tail := triangles[before-trailing:before]; shift the ink down.
        self.triangles[before - trailing..].rotate_left(trailing);
    }

    fn classic_ink_edges_body(&mut self, n: &Node, triangles: &[Triangle]) {
        let n = native_classic_node(n);
        // A translucent source object is one compositing group.
        if n.stroke_width <= 0 || n.opacity < 1.0 || n.stroke.eq_ignore_ascii_case("none") {
            return;
        }
        let mut paint = native_paint(&n.stroke, "#263c4e");
        paint.a = (paint.a as f64 * go_min(1.0, n.opacity)).round() as u8;
        if paint.a < 255 {
            return;
        }
        let mut scale = self.scale;
        if scale <= 0.0 {
            scale = 0.01;
            if n.metadata.original.width > 0 {
                scale = n.size.x / n.metadata.original.width as f64;
            }
        }
        let radius = n.stroke_width as f64 * scale / 2.0;
        let segments = match classic_ink_segments_with_context(&self.context, &n, triangles, radius)
        {
            Ok(s) => s,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let material = native_material(&n.stroke, 1.0, 0.0, n.opacity);
        {
            let mut m = material.borrow_mut();
            m.unlit = true;
            m.svg_contour = true;
        }
        let (mut dash, mut gap) = (0.0, 0.0);
        if n.stroke_dash > 0.0 {
            (dash, gap) = get_stroke_dash_attributes(n.stroke_width as f64, n.stroke_dash);
            (dash, gap) = (dash * scale, gap * scale);
            if !caption_finite(&[dash, gap]) || dash <= 0.0 || gap <= 0.0 {
                (dash, gap) = (radius * 4.0, radius * 4.0);
            }
        }
        let mut ends: std::vec::Vec<ClassicInkSegment> = vec![];
        let ribbon = |b: &mut MeshBuilder,
                      path: &[ClassicInkSegment],
                      ends: &mut std::vec::Vec<ClassicInkSegment>| {
            if path.is_empty() {
                return;
            }
            b.classic_ink_ribbon(path, radius, &material);
            ends.push(path[0].clone());
            let last = &path[path.len() - 1];
            ends.push(ClassicInkSegment {
                a: last.c,
                c: last.a,
                support: last.support.clone(),
            });
        };
        let mut count = 0;
        for path in classic_ink_paths(&segments) {
            if dash == 0.0 {
                ribbon(self, &path, &mut ends);
                continue;
            }
            let mut phase = 0.0;
            let mut part: std::vec::Vec<ClassicInkSegment> = vec![];
            for s in &path {
                let delta = nsub(s.c, s.a);
                let view = native_camera_axes().direction;
                let length = nlen(nsub(delta, nmul(view, ndot(delta, view))));
                if length < 1e-10 {
                    continue;
                }
                let mut at = 0.0;
                while at < length - 1e-10 {
                    count += 1;
                    if count > MAX_CLASSIC_INK_SEGMENTS {
                        self.fail(format!(
                            "isometric dashed outline exceeds {MAX_CLASSIC_INK_SEGMENTS} segments"
                        ));
                        return;
                    }
                    let on = phase < dash;
                    let remaining = if on { dash - phase } else { dash + gap - phase };
                    let next = go_min(length, at + remaining);
                    if on {
                        part.push(ClassicInkSegment {
                            a: nlerp(s.a, s.c, at / length),
                            c: nlerp(s.a, s.c, next / length),
                            support: s.support.clone(),
                        });
                    }
                    phase += next - at;
                    at = next;
                    if on && phase >= dash - 1e-10 {
                        ribbon(self, &part, &mut ends);
                        part.clear();
                        phase = dash;
                    }
                    if phase >= dash + gap - 1e-10 {
                        phase = 0.0;
                    }
                }
            }
            ribbon(self, &part, &mut ends);
        }
        self.classic_ink_junctions(&segments, &ends, radius, &material);
    }

    pub fn classic_ink_ribbon(
        &mut self,
        path: &[ClassicInkSegment],
        radius: f64,
        material: &MaterialRef,
    ) {
        if path.is_empty() || self.err.is_some() {
            return;
        }
        let camera = native_camera_axes();
        let view = camera.direction;
        let sides: std::vec::Vec<Vec> = path
            .iter()
            .map(|s| nunit(ncross(view, nsub(s.c, s.a))))
            .collect();
        let closed = classic_ink_key_of(path[0].a) == classic_ink_key_of(path[path.len() - 1].c);
        let mut left = vec![Vertex::default(); path.len() + 1];
        let mut right = left.clone();
        for i in 0..left.len() {
            let (mut previous, mut next) = (i.saturating_sub(1), i.min(path.len() - 1));
            if closed && (i == 0 || i == path.len()) {
                (previous, next) = (path.len() - 1, 0);
            }
            let mut miter = nadd(sides[previous], sides[next]);
            if nlen(miter) < 1e-8 {
                miter = sides[next];
            } else {
                miter = nunit(miter);
            }
            let offset = nmul(miter, radius / go_max(0.5, ndot(miter, sides[next])));
            let mut at = path[i.min(path.len() - 1)].a;
            if i == path.len() {
                at = path[path.len() - 1].c;
            }
            let vertex = |offset: Vec| {
                // Keep the exposed half at the edge's height.
                let mut p = nadd(at, offset);
                if view.y.abs() > 1e-8 {
                    p = nsub(p, nmul(view, offset.y / view.y));
                }
                Vertex {
                    position: p,
                    normal: view,
                    ..Default::default()
                }
            };
            left[i] = vertex(offset);
            right[i] = vertex(nmul(offset, -1.0));
        }
        for i in 0..path.len() {
            self.classic_ink_face(
                [left[i], right[i], right[i + 1]],
                &path[i].support,
                material,
                &camera,
            );
            self.classic_ink_face(
                [left[i], right[i + 1], left[i + 1]],
                &path[i].support,
                material,
                &camera,
            );
        }
    }

    /// Intersect the ribbon with the finite supporting faces.
    pub fn classic_ink_face(
        &mut self,
        vertices: [Vertex; 3],
        support: &[Rc<ClassicInkFacet>],
        material: &MaterialRef,
        camera: &Camera,
    ) {
        if self.err.is_some() {
            return;
        }
        let emit = |b: &mut MeshBuilder, a: Vertex, c: Vertex, d: Vertex| {
            let first = b.triangles.len();
            b.triangle(a, c, d, material, false);
            if b.triangles.len() > first {
                b.triangles[first].depth_bias = CLASSIC_INK_DEPTH_BIAS;
            }
        };
        emit(self, vertices[0], vertices[1], vertices[2]);
        let mut projected = [Vec::default(); 3];
        let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for (i, v) in vertices.iter().enumerate() {
            let p = v.position;
            projected[i] = nv(
                ndot(p, camera.right),
                ndot(p, camera.up),
                ndot(p, camera.direction),
            );
            lo_x = go_min(lo_x, projected[i].x);
            lo_y = go_min(lo_y, projected[i].y);
            hi_x = go_max(hi_x, projected[i].x);
            hi_y = go_max(hi_y, projected[i].y);
        }
        fn clip(polygon: &[Vec], distance: impl Fn(Vec) -> f64) -> std::vec::Vec<Vec> {
            if polygon.is_empty() {
                return vec![];
            }
            let mut out = vec![];
            let mut previous = polygon[polygon.len() - 1];
            let mut pd = distance(previous);
            for &at in polygon {
                let d = distance(at);
                if (d >= 0.0) != (pd >= 0.0) {
                    out.push(nlerp(previous, at, pd / (pd - d)));
                }
                if d >= 0.0 {
                    out.push(at);
                }
                previous = at;
                pd = d;
            }
            out
        }
        for face in support {
            let [a, c, d] = face.points;
            if max3(a.x, c.x, d.x) < lo_x
                || min3(a.x, c.x, d.x) > hi_x
                || max3(a.y, c.y, d.y) < lo_y
                || min3(a.y, c.y, d.y) > hi_y
            {
                continue;
            }
            let area = (c.x - a.x) * (d.y - a.y) - (c.y - a.y) * (d.x - a.x);
            if area.abs() < 1e-14 {
                continue;
            }
            let sign = if area < 0.0 { -1.0 } else { 1.0 };
            let mut polygon = projected.to_vec();
            for i in 0..3 {
                let (p, q) = (face.points[i], face.points[(i + 1) % 3]);
                polygon = clip(&polygon, |at| {
                    sign * ((q.x - p.x) * (at.y - p.y) - (q.y - p.y) * (at.x - p.x))
                });
            }
            // A supported patch replaces only the part that the face occludes.
            polygon = clip(&polygon, |p| {
                face.depth_at(p).unwrap_or(0.0) - p.z - CLASSIC_INK_DEPTH_BIAS
            });
            if polygon.len() < 3 {
                continue;
            }
            let vertex = |p: Vec| Vertex {
                position: nadd(
                    nadd(nmul(camera.right, p.x), nmul(camera.up, p.y)),
                    nmul(camera.direction, face.depth_at(p).unwrap_or(0.0)),
                ),
                normal: camera.direction,
                ..Default::default()
            };
            // A stable fan anchor keeps equivalent loops from changing diagonals.
            let anchor = classic_ink_fan_anchor(&polygon);
            let first = vertex(polygon[anchor]);
            for i in 1..polygon.len() - 1 {
                emit(
                    self,
                    first,
                    vertex(polygon[(anchor + i) % polygon.len()]),
                    vertex(polygon[(anchor + i + 1) % polygon.len()]),
                );
            }
        }
    }

    /// native_ink_joins.go: one shared exterior join where a silhouette turns
    /// a corner between separately dashed paths.
    pub fn classic_ink_junctions(
        &mut self,
        segments: &[ClassicInkSegment],
        ends: &[ClassicInkSegment],
        radius: f64,
        material: &MaterialRef,
    ) {
        if self.err.is_some() || radius <= 0.0 {
            return;
        }
        let mut degree: HashMap<ClassicInkKey, usize> = HashMap::new();
        for s in segments {
            *degree.entry(classic_ink_key_of(s.a)).or_default() += 1;
            *degree.entry(classic_ink_key_of(s.c)).or_default() += 1;
        }
        let mut junctions: BTreeMap<ClassicInkKey, std::vec::Vec<ClassicInkSegment>> =
            BTreeMap::new();
        for s in ends {
            let key = classic_ink_key_of(s.a);
            if degree.get(&key).copied().unwrap_or(0) > 2 {
                junctions.entry(key).or_default().push(s.clone());
            }
        }
        let camera = native_camera_axes();
        let view = camera.direction;
        for incident in junctions.values() {
            if incident.len() < 2 {
                continue;
            }
            let mut rays: std::vec::Vec<(Vec, f64)> = vec![];
            let mut support: std::vec::Vec<Rc<ClassicInkFacet>> = vec![];
            let mut seen: HashSet<*const ClassicInkFacet> = HashSet::new();
            for s in incident {
                let mut delta = nsub(s.c, s.a);
                delta = nsub(delta, nmul(view, ndot(delta, view)));
                if nlen(delta) < 1e-10 {
                    continue;
                }
                let direction = nunit(delta);
                rays.push((
                    direction,
                    gomath::atan2(ndot(direction, camera.up), ndot(direction, camera.right)),
                ));
                for face in &s.support {
                    if seen.insert(Rc::as_ptr(face)) {
                        support.push(face.clone());
                        if support.len() > MAX_CLASSIC_INK_SUPPORT_FACES {
                            self.fail(format!("isometric outline join exceeds {MAX_CLASSIC_INK_SUPPORT_FACES} local supporting faces"));
                            return;
                        }
                    }
                }
            }
            if rays.len() < 2 {
                continue;
            }
            go_sort::sort_by(&mut rays, |a, b| a.1 < b.1);
            let at = incident[0].a;
            let vertex = |offset: Vec| {
                let mut p = nadd(at, offset);
                if view.y.abs() > 1e-8 {
                    p = nsub(p, nmul(view, offset.y / view.y));
                }
                Vertex {
                    position: p,
                    normal: view,
                    ..Default::default()
                }
            };
            for i in 0..rays.len() {
                let (a, c) = (rays[i], rays[(i + 1) % rays.len()]);
                let mut angle = c.1 - a.1;
                if i == rays.len() - 1 {
                    angle += 2.0 * std::f64::consts::PI;
                }
                if angle <= std::f64::consts::PI + 1e-8
                    || angle >= 2.0 * std::f64::consts::PI - 1e-8
                {
                    continue;
                }
                // Fill the outer wedge with a sharp miter, or a bounded bevel.
                let u = ncross(view, a.0);
                let v = nmul(ncross(view, c.0), -1.0);
                let (l, r) = (nmul(u, radius), nmul(v, radius));
                let miter = nmul(nadd(u, v), radius / go_max(1e-12, 1.0 + ndot(u, v)));
                let origin = vertex(Vec::default());
                if nlen(miter) <= 2.0 * radius {
                    self.classic_ink_face(
                        [origin, vertex(l), vertex(miter)],
                        &support,
                        material,
                        &camera,
                    );
                    self.classic_ink_face(
                        [origin, vertex(miter), vertex(r)],
                        &support,
                        material,
                        &camera,
                    );
                } else {
                    self.classic_ink_face(
                        [origin, vertex(l), vertex(r)],
                        &support,
                        material,
                        &camera,
                    );
                }
            }
        }
    }
}

pub fn classic_ink_fan_anchor(polygon: &[Vec]) -> usize {
    // These keys only choose a starting vertex.
    let precision = (1u64 << 32) as f64;
    let key = |p: Vec| -> ClassicInkKey {
        [
            (p.x * precision).round() as i64,
            (p.y * precision).round() as i64,
            (p.z * precision).round() as i64,
        ]
    };
    let mut first = 0;
    for i in 1..polygon.len() {
        if classic_ink_less(&key(polygon[i]), &key(polygon[first])) {
            first = i;
        }
    }
    first
}

#[cfg(test)]
#[path = "ink_api_tests.rs"]
mod api_tests;

#[cfg(test)]
#[path = "composition_api_tests.rs"]
mod composition_api_tests;

#[cfg(test)]
#[path = "residual_api_tests.rs"]
mod residual_api_tests;
