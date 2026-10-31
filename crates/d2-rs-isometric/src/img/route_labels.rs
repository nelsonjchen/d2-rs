// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg route_labels.go and projection_clearance.go: route caption
//! placement against projected obstacles. Contraction sites are marked
//! `ARM64:`; products by 0.5 are exact.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use d2_rs_core::go_sort;
use d2_rs_render::go_float::{go_fmadd, go_fmsub, go_fnmsub};
use super::math as gomath;
use d2_rs_scene::{go_max, go_min};

use super::geometry::LabelSurface;
use super::mesh::{native_view_direction, nv, Material, Triangle, Vec};

const MAX_ROUTE_CAPTION_RECTS: usize = 19000;
const MAX_ROUTE_CAPTION_CANDIDATES: usize = 192;
const MAX_ROUTE_CAPTION_WORK: usize = 2_000_000;
const MAX_ROUTE_CAPTION_GRID_REFS: usize = 250_000;
const MAX_ROUTE_CAPTION_CELLS: i64 = 128;
pub const ROUTE_CAPTION_GAP: f64 = 0.055;
const MAX_ROUTE_CAPTION_HULL_VERTICES: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RouteCaptionPoint {
    pub x: f64,
    pub z: f64,
}

/// All obstacles share the camera's projection onto Y=0.
#[derive(Clone, Debug, Default)]
pub struct RouteCaptionRect {
    x: f64,
    z: f64,
    ux: f64,
    uz: f64,
    hw: f64,
    hd: f64,
    left: f64,
    right: f64,
    front: f64,
    back: f64,
    hard: bool,
    hull: std::vec::Vec<RouteCaptionPoint>,
}

#[derive(Default)]
pub struct RouteCaptionPlacer {
    rects: std::vec::Vec<RouteCaptionRect>,
    grid: HashMap<(i64, i64), std::vec::Vec<usize>>,
    wide: std::vec::Vec<usize>,
    work: usize,
    refs: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RouteCaptionLeg {
    pub a: Vec,
    pub b: Vec,
    pub start: f64,
    pub length: f64,
}

pub fn caption_finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite() && v.abs() <= 1e9)
}

/// Project parallel to the final orthographic camera onto one board plane.
pub fn caption_projection(p: Vec) -> RouteCaptionPoint {
    let d = native_view_direction();
    RouteCaptionPoint { x: p.x - p.y * d.x / d.y, z: p.z - p.y * d.z / d.y }
}

impl RouteCaptionPlacer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Avoid reserves an axis-aligned footprint.
    pub fn avoid(&mut self, center: Vec, width: f64, depth: f64) {
        if !caption_finite(&[center.x, center.y, center.z, width, depth]) || width <= 0.0 || depth <= 0.0 {
            return;
        }
        self.reserve(caption_rect(&LabelSurface { center, width, depth, ..Default::default() }, true));
    }

    /// Place keeps the supplied print dimensions and searches straight legs,
    /// along-leg offsets, then both sides of a route.
    pub fn place(&mut self, points: &[Vec], fraction: f64, width: f64, depth: f64) -> LabelSurface {
        if points.len() < 2 || !caption_finite(&[fraction, width, depth]) || width <= 0.0 || depth <= 0.0 {
            return LabelSurface::default();
        }
        if !caption_finite(&[points[0].y]) {
            return LabelSurface::default();
        }
        let fraction = go_max(0.0, go_min(1.0, fraction));
        let short;
        let points = if points.len() > 10000 {
            short = [points[0], points[points.len() - 1]];
            &short[..]
        } else {
            points
        };
        if self.rects.len() >= MAX_ROUTE_CAPTION_RECTS || self.work + points.len() > MAX_ROUTE_CAPTION_WORK {
            return caption_fallback(points, fraction, width, depth);
        }
        self.work += points.len();
        let (mut legs, total) = caption_legs(points);
        if legs.is_empty() {
            return LabelSurface::default();
        }
        let weight = if fraction != 0.5 { 3.0 } else { 0.35 };
        // ARM64 route_labels.go:83/90: target = total*fraction stays fused.
        let score = |l: &RouteCaptionLeg| go_fmsub(go_fmsub(total, fraction, l.start + l.length / 2.0).abs(), weight, l.length);
        legs.sort_by(|a, b| score(b).partial_cmp(&score(a)).unwrap_or(std::cmp::Ordering::Equal));
        let anchor = |l: &RouteCaptionLeg| {
            if fraction == 0.5 {
                return l.length / 2.0;
            }
            go_max(l.length * 0.15, go_min(l.length * 0.85, go_fnmsub(total, fraction, l.start)))
        };
        let normal_offset = go_max(0.25, depth / 2.0 + 0.075);
        let fallback = caption_on_leg(&legs[0], anchor(&legs[0]), normal_offset, width, depth, points[0].y);
        let (mut best, mut best_score) = (fallback, f64::INFINITY);
        let mut candidates = 0;
        for (leg_index, leg) in legs.iter().enumerate() {
            if leg_index >= 8 || candidates >= MAX_ROUTE_CAPTION_CANDIDATES || self.work >= MAX_ROUTE_CAPTION_WORK {
                break;
            }
            let inset = go_min(0.035, leg.length * 0.03);
            let (lo, hi) = (width / 2.0 + inset, leg.length - width / 2.0 - inset);
            if lo > hi {
                continue;
            }
            let preferred = go_max(lo, go_min(hi, anchor(leg)));
            let step = width + 2.0 * ROUTE_CAPTION_GAP;
            let positions = [preferred, preferred - step, preferred + step, preferred - 2.0 * step, preferred + 2.0 * step, lo, hi];
            for offset in [normal_offset, -normal_offset, normal_offset + depth + 0.1, -normal_offset - depth - 0.1] {
                for &along in &positions {
                    if along < lo || along > hi || candidates >= MAX_ROUTE_CAPTION_CANDIDATES {
                        continue;
                    }
                    // Endpoint labels remain in their end of the route.
                    let arc = (leg.start + along) / total;
                    if fraction < 0.25 && arc > 0.4 || fraction > 0.75 && arc < 0.6 {
                        continue;
                    }
                    let candidate = caption_on_leg(leg, along, offset, width, depth, points[0].y);
                    candidates += 1;
                    let (score, complete) = self.score(&caption_rect(&candidate, false));
                    if !complete {
                        self.reserve(caption_rect(&best, false));
                        return best;
                    }
                    if score < best_score {
                        best = candidate.clone();
                        best_score = score;
                    }
                    if score == 0.0 {
                        self.reserve(caption_rect(&candidate, false));
                        return candidate;
                    }
                }
            }
        }
        self.reserve(caption_rect(&best, false));
        best
    }

    pub fn reserve(&mut self, r: RouteCaptionRect) {
        if self.rects.len() >= MAX_ROUTE_CAPTION_RECTS {
            return;
        }
        let id = self.rects.len();
        let cells = caption_cells(&r);
        self.rects.push(r);
        let Some((x0, x1, z0, z1)) = cells else {
            self.wide.push(id);
            return;
        };
        if self.refs + ((x1 - x0 + 1) * (z1 - z0 + 1)) as usize > MAX_ROUTE_CAPTION_GRID_REFS {
            self.wide.push(id);
            return;
        }
        for x in x0..=x1 {
            for z in z0..=z1 {
                self.grid.entry((x, z)).or_default().push(id);
                self.refs += 1;
            }
        }
    }

    fn score(&mut self, r: &RouteCaptionRect) -> (f64, bool) {
        let mut seen: HashSet<usize> = HashSet::new();
        let mut score = 0.0;
        let mut check = |p: &mut RouteCaptionPlacer, id: usize| -> bool {
            if p.work >= MAX_ROUTE_CAPTION_WORK {
                return false;
            }
            p.work += 1;
            if !seen.insert(id) {
                return true;
            }
            let other = &p.rects[id];
            // Charge mesh silhouettes their actual narrow-phase size.
            if (!r.hull.is_empty() || !other.hull.is_empty())
                && r.right > other.left
                && other.right > r.left
                && r.back > other.front
                && other.back > r.front
            {
                let vertices = 4.max(r.hull.len()) + 4.max(other.hull.len());
                let cost = vertices * vertices;
                if p.work + cost > MAX_ROUTE_CAPTION_WORK {
                    return false;
                }
                p.work += cost;
            }
            let other = &p.rects[id];
            if caption_overlap(r, other) {
                score += if other.hard { 10000.0 } else { 1.0 };
            }
            true
        };
        let Some((x0, x1, z0, z1)) = caption_cells(r) else {
            for id in 0..self.rects.len() {
                if !check(self, id) {
                    return (score, false);
                }
            }
            return (score, true);
        };
        for i in 0..self.wide.len() {
            let id = self.wide[i];
            if !check(self, id) {
                return (score, false);
            }
        }
        for x in x0..=x1 {
            for z in z0..=z1 {
                if self.work >= MAX_ROUTE_CAPTION_WORK {
                    return (score, false);
                }
                self.work += 1;
                let ids = self.grid.get(&(x, z)).cloned().unwrap_or_default();
                for id in ids {
                    if !check(self, id) {
                        return (score, false);
                    }
                }
            }
        }
        (score, true)
    }

    /// AvoidMesh reserves a completed component's projected geometry: its
    /// structural silhouette and each printed surface separately.
    pub fn avoid_mesh(&mut self, triangles: &[Triangle]) {
        if self.rects.len() >= MAX_ROUTE_CAPTION_RECTS {
            return;
        }
        let mut body = vec![];
        let mut surfaces: std::vec::Vec<std::vec::Vec<RouteCaptionPoint>> = vec![];
        let mut indices: HashMap<*const std::cell::RefCell<Material>, usize> = HashMap::new();
        for t in triangles {
            let Some(m) = &t.material else { continue };
            if m.borrow().color.a == 0 {
                continue;
            }
            if t.v.iter().any(|v| !caption_finite(&[v.position.x, v.position.y, v.position.z])) {
                continue;
            }
            let points = t.v.map(|v| caption_projection(v.position));
            if m.borrow().texture.is_none() {
                body.extend(points);
                continue;
            }
            let next = surfaces.len();
            let index = *indices.entry(Rc::as_ptr(m)).or_insert(next);
            if index == next {
                surfaces.push(vec![]);
            }
            surfaces[index].extend(points);
        }
        self.avoid_projected_hull(body);
        // Iteration follows mesh/material creation order, never map order.
        for surface in surfaces {
            self.avoid_projected_hull(surface);
        }
    }

    fn avoid_projected_hull(&mut self, mut points: std::vec::Vec<RouteCaptionPoint>) {
        if points.len() < 3 || self.rects.len() >= MAX_ROUTE_CAPTION_RECTS {
            return;
        }
        let hull = caption_convex_hull(&mut points);
        if hull.len() < 3 {
            return;
        }
        let mut r = RouteCaptionRect {
            left: f64::INFINITY,
            front: f64::INFINITY,
            right: f64::NEG_INFINITY,
            back: f64::NEG_INFINITY,
            hard: true,
            ..Default::default()
        };
        for q in &hull {
            r.left = go_min(r.left, q.x);
            r.right = go_max(r.right, q.x);
            r.front = go_min(r.front, q.z);
            r.back = go_max(r.back, q.z);
        }
        r.hull = hull;
        // Bound narrow-phase work for complex imported artwork.
        if r.hull.len() > MAX_ROUTE_CAPTION_HULL_VERTICES {
            r.hull = vec![
                RouteCaptionPoint { x: r.left, z: r.front },
                RouteCaptionPoint { x: r.right, z: r.front },
                RouteCaptionPoint { x: r.right, z: r.back },
                RouteCaptionPoint { x: r.left, z: r.back },
            ];
        }
        r.left -= ROUTE_CAPTION_GAP;
        r.right += ROUTE_CAPTION_GAP;
        r.front -= ROUTE_CAPTION_GAP;
        r.back += ROUTE_CAPTION_GAP;
        self.reserve(r);
    }

    /// Automatic captions also avoid crossing other wires.
    pub fn avoid_route(&mut self, points: &[Vec], radius: f64) {
        if radius <= 0.0 || !caption_finite(&[radius]) {
            return;
        }
        let mut i = 1;
        while i < points.len() && self.rects.len() < MAX_ROUTE_CAPTION_RECTS / 2 {
            let (a, b) = (caption_projection(points[i - 1]), caption_projection(points[i]));
            i += 1;
            if !caption_finite(&[a.x, a.z, b.x, b.z]) {
                continue;
            }
            let length = gomath::hypot(b.x - a.x, b.z - a.z);
            if length <= 1e-9 {
                continue;
            }
            self.reserve(caption_rect(
                &LabelSurface {
                    center: nv((a.x + b.x) / 2.0, 0.0, (a.z + b.z) / 2.0),
                    width: length,
                    depth: 2.0 * radius,
                    angle: gomath::atan2(b.z - a.z, b.x - a.x),
                    ..Default::default()
                },
                true,
            ));
        }
    }
}

pub fn caption_legs(points: &[Vec]) -> (std::vec::Vec<RouteCaptionLeg>, f64) {
    let mut legs: std::vec::Vec<RouteCaptionLeg> = vec![];
    let mut total = 0.0;
    for i in 1..points.len() {
        let (a, b) = (points[i - 1], points[i]);
        if !caption_finite(&[a.x, a.y, a.z, b.x, b.y, b.z]) {
            continue;
        }
        let length = gomath::hypot(b.x - a.x, b.z - a.z);
        if length < 1e-9 {
            continue;
        }
        if let Some(last) = legs.last_mut() {
            let (x, z) = (last.b.x - last.a.x, last.b.z - last.a.z);
            let (x2, z2) = (b.x - a.x, b.z - a.z);
            // ARM64 route_labels.go:166.
            if gomath::hypot(last.b.x - a.x, last.b.z - a.z) < 1e-9
                && go_fmsub(z, x2, x * z2).abs() <= 1e-9 * last.length * length
                && go_fmadd(z, z2, x * x2) > 0.0
            {
                last.b = b;
                last.length += length;
                total += length;
                continue;
            }
        }
        legs.push(RouteCaptionLeg { a, b, start: total, length });
        total += length;
    }
    (legs, total)
}

fn caption_fallback(points: &[Vec], fraction: f64, width: f64, depth: f64) -> LabelSurface {
    let center = (points.len() as i64 - 2).min((fraction * (points.len() - 1) as f64) as i64);
    for step in 0..32i64 {
        let i = if step % 2 != 0 { center - (step + 1) / 2 } else { center + (step + 1) / 2 };
        if i < 0 || i + 1 >= points.len() as i64 {
            continue;
        }
        let (a, b) = (points[i as usize], points[i as usize + 1]);
        let length = gomath::hypot(b.x - a.x, b.z - a.z);
        if length > 1e-9 && caption_finite(&[a.x, a.z, b.x, b.z]) {
            let leg = RouteCaptionLeg { a, b, length, start: 0.0 };
            return caption_on_leg(&leg, length / 2.0, go_max(0.25, depth / 2.0 + 0.075), width, depth, points[0].y);
        }
    }
    LabelSurface::default()
}

pub fn caption_on_leg(l: &RouteCaptionLeg, along: f64, offset: f64, width: f64, depth: f64, y: f64) -> LabelSurface {
    let (mut ux, mut uz) = ((l.b.x - l.a.x) / l.length, (l.b.z - l.a.z) / l.length);
    // ARM64 route_labels.go:201.
    let (x, z) = (go_fmadd(ux, along, l.a.x), go_fmadd(uz, along, l.a.z));
    // Reversing an edge does not turn its text upside down.
    if ux < -1e-6 || ux.abs() < 1e-6 && uz < 0.0 {
        (ux, uz) = (-ux, -uz);
    }
    LabelSurface {
        // ARM64 route_labels.go:206.
        center: nv(go_fmadd(uz, offset, x), y + 0.006, go_fmsub(ux, offset, z)),
        width,
        depth,
        angle: gomath::atan2(uz, ux),
        ..Default::default()
    }
}

pub fn caption_rect(s: &LabelSurface, hard: bool) -> RouteCaptionRect {
    let center = caption_projection(s.center);
    let mut r = RouteCaptionRect {
        x: center.x,
        z: center.z,
        ux: gomath::cos(s.angle),
        uz: gomath::sin(s.angle),
        hw: s.width / 2.0 + ROUTE_CAPTION_GAP,
        hd: s.depth / 2.0 + ROUTE_CAPTION_GAP,
        hard,
        ..Default::default()
    };
    // ARM64 route_labels.go:212.
    let x = go_fmadd(r.uz.abs(), r.hd, r.ux.abs() * r.hw);
    let z = go_fmadd(r.ux.abs(), r.hd, r.uz.abs() * r.hw);
    (r.left, r.right, r.front, r.back) = (r.x - x, r.x + x, r.z - z, r.z + z);
    r
}

fn caption_overlap(a: &RouteCaptionRect, b: &RouteCaptionRect) -> bool {
    if a.right <= b.left || b.right <= a.left || a.back <= b.front || b.back <= a.front {
        return false;
    }
    if !a.hull.is_empty() || !b.hull.is_empty() {
        return caption_hull_overlap(a, b);
    }
    let (dx, dz) = (b.x - a.x, b.z - a.z);
    for axis in [[a.ux, a.uz], [-a.uz, a.ux], [b.ux, b.uz], [-b.uz, b.ux]] {
        // ARM64 route_labels.go:227.
        let project = |r: &RouteCaptionRect| {
            go_fmadd(r.hd, go_fmadd(r.ux, axis[1], -r.uz * axis[0]).abs(), r.hw * go_fmadd(r.uz, axis[1], r.ux * axis[0]).abs())
        };
        // ARM64 route_labels.go:229.
        if go_fmadd(dz, axis[1], dx * axis[0]).abs() >= project(a) + project(b) {
            return false;
        }
    }
    true
}

fn caption_cells(r: &RouteCaptionRect) -> Option<(i64, i64, i64, i64)> {
    if !caption_finite(&[r.left, r.right, r.front, r.back]) {
        return None;
    }
    let (x0, x1, z0, z1) =
        (gomath::floor(r.left) as i64, gomath::floor(r.right) as i64, gomath::floor(r.front) as i64, gomath::floor(r.back) as i64);
    let (nx, nz) = (x1 - x0 + 1, z1 - z0 + 1);
    (nx > 0 && nz > 0 && nx <= MAX_ROUTE_CAPTION_CELLS && nz <= MAX_ROUTE_CAPTION_CELLS && nx * nz <= MAX_ROUTE_CAPTION_CELLS)
        .then_some((x0, x1, z0, z1))
}

pub fn caption_convex_hull(points: &mut [RouteCaptionPoint]) -> std::vec::Vec<RouteCaptionPoint> {
    go_sort::sort_by(points, |a, b| a.x < b.x || a.x == b.x && a.z < b.z);
    // ARM64 projection_clearance.go:116.
    let turn = |a: RouteCaptionPoint, b: RouteCaptionPoint, c: RouteCaptionPoint| go_fmsub(b.z - a.z, c.x - a.x, (b.x - a.x) * (c.z - a.z));
    let mut hull: std::vec::Vec<RouteCaptionPoint> = std::vec::Vec::with_capacity(points.len().min(64));
    for &q in points.iter() {
        while hull.len() >= 2 && turn(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.0 {
            hull.pop();
        }
        hull.push(q);
    }
    let lower = hull.len();
    for i in (0..points.len().saturating_sub(1)).rev() {
        let q = points[i];
        while hull.len() > lower && turn(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.0 {
            hull.pop();
        }
        hull.push(q);
    }
    hull.pop();
    hull
}

fn caption_hull_corners(r: &RouteCaptionRect) -> std::vec::Vec<RouteCaptionPoint> {
    if !r.hull.is_empty() {
        return r.hull.clone();
    }
    // ARM64 projection_clearance.go:140: every corner sum fuses its products.
    let (ux, hw, uz, hd) = (r.ux, r.hw, r.uz, r.hd);
    let (xm, xp) = (go_fmsub(ux, hw, r.x), go_fmadd(ux, hw, r.x));
    let (zm, zp) = (go_fmsub(uz, hw, r.z), go_fmadd(uz, hw, r.z));
    vec![
        RouteCaptionPoint { x: go_fmsub(-uz, hd, xm), z: go_fmsub(ux, hd, zm) },
        RouteCaptionPoint { x: go_fmsub(-uz, hd, xp), z: go_fmsub(ux, hd, zp) },
        RouteCaptionPoint { x: go_fmadd(-uz, hd, xp), z: go_fmadd(ux, hd, zp) },
        RouteCaptionPoint { x: go_fmadd(-uz, hd, xm), z: go_fmadd(ux, hd, zm) },
    ]
}

fn caption_hull_overlap(a: &RouteCaptionRect, b: &RouteCaptionRect) -> bool {
    let (pa, pb) = (caption_hull_corners(a), caption_hull_corners(b));
    let interval = |points: &[RouteCaptionPoint], ax: f64, az: f64| {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for q in points {
            // ARM64 projection_clearance.go:148.
            let distance = go_fmadd(q.z, az, q.x * ax);
            lo = go_min(lo, distance);
            hi = go_max(hi, distance);
        }
        (lo, hi)
    };
    for points in [&pa, &pb] {
        for (i, q) in points.iter().enumerate() {
            let next = points[(i + 1) % points.len()];
            let (ax, az) = (next.z - q.z, q.x - next.x);
            let length = gomath::hypot(ax, az);
            if length <= 1e-12 {
                continue;
            }
            let (alo, ahi) = interval(&pa, ax, az);
            let (blo, bhi) = interval(&pb, ax, az);
            // Rectangles already include the print gap; mesh hulls need it.
            let mut gap = 0.0;
            if !a.hull.is_empty() {
                gap = go_fmadd(ROUTE_CAPTION_GAP, length, gap);
            }
            if !b.hull.is_empty() {
                gap = go_fmadd(ROUTE_CAPTION_GAP, length, gap);
            }
            if ahi + gap <= blo || bhi + gap <= alo {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
#[path = "caption_api_tests.rs"]
mod api_tests;

#[cfg(test)]
#[path = "contact_api_tests.rs"]
pub(crate) mod contact_api_tests;
