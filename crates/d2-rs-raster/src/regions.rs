// SPDX-License-Identifier: MPL-2.0
//! d2raster/mask_regions.go and stroke_regions.go: tighter band-rendering
//! bounds for luminance masks with an opaque white backdrop and for
//! stroke-only effect layers.

use d2_rs_scene::{go_max, go_min, BlendMode, LineCap, LineJoin, MaskType, Matrix, Point};

use crate::fma::{finite, finite_point, go_fmadd, gomath, mpoint};
use crate::geometry::{stroke_extent, PreparedStroke};
use crate::paint::PaintKind;
use crate::pixels::{union_rect, Rect};
use crate::raster::{PreparedMask, PreparedNode, PreparedPrimitive};

pub fn mask_render_bounds(mask: &PreparedMask, destination: Rect, reference_coordinates: bool) -> Rect {
    if !reference_coordinates || mask.kind != MaskType::LUMINANCE || !plain_mask_node(&mask.root) || mask.root.primitive.is_some() || mask.root.children.is_empty() {
        return destination;
    }
    let base = &mask.root.children[0];
    if !plain_mask_node(base) || !base.children.is_empty() {
        return destination;
    }
    let Some(primitive) = &base.primitive else {
        return destination;
    };
    let Some(fill) = &primitive.fill else {
        return destination;
    };
    if fill.kind != PaintKind::Solid
        || fill.solid.r != 255
        || fill.solid.g != 255
        || fill.solid.b != 255
        || fill.solid.a != 255
        || primitive.stroke.is_some()
        || primitive.image.is_some()
        || primitive.vector.is_some()
        || primitive.subpaths.len() != 1
    {
        return destination;
    }
    if (primitive.reference_bounds.min.x as f64).abs() > (1u64 << 20) as f64
        || (primitive.reference_bounds.min.y as f64).abs() > (1u64 << 20) as f64
        || !bounded_region_point(Point::new(primitive.transform.e, primitive.transform.f))
    {
        return destination;
    }
    let points = &primitive.subpaths[0].points;
    if points.len() != 4 {
        return destination;
    }
    let mut corners = [Point::default(); 4];
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let limit = (1u64 << 20) as f64;
    for (index, point) in points.iter().enumerate() {
        let point = mpoint(&primitive.transform, *point);
        if !finite_point(point) || point.x.abs() > limit || point.y.abs() > limit {
            return destination;
        }
        corners[index] = point;
        min_x = go_min(min_x, point.x);
        min_y = go_min(min_y, point.y);
        max_x = go_max(max_x, point.x);
        max_y = go_max(max_y, point.y);
    }
    let mut seen_corners = 0u8;
    for index in 0..4 {
        let point = corners[index];
        let next = corners[(index + 1) % 4];
        if point.x != min_x && point.x != max_x || point.y != min_y && point.y != max_y || point == next || (point.x == next.x) == (point.y == next.y) {
            return destination;
        }
        let mut corner = 0u8;
        if point.x == max_x {
            corner |= 1;
        }
        if point.y == max_y {
            corner |= 2;
        }
        seen_corners |= 1 << corner;
    }
    if seen_corners != 0xf {
        return destination;
    }
    let interior = Rect::new(min_x.ceil() as i64 + 1, min_y.ceil() as i64 + 1, max_x.floor() as i64 - 1, max_y.floor() as i64 - 1);
    if !destination.is_in(interior) {
        return destination;
    }
    let mut changed = Rect::default();
    for child in &mask.root.children[1..] {
        if child.opacity != 0.0 {
            changed = union_rect(changed, child.bounds.intersect(destination));
        }
    }
    changed
}

fn plain_mask_node(node: &PreparedNode) -> bool {
    node.opacity == 1.0 && node.blend == BlendMode::NORMAL && !node.isolated && node.filters.is_empty() && node.clip.is_none() && node.mask.is_none()
}

pub fn effect_render_bounds(node: &PreparedNode, destination: Rect) -> Rect {
    if node.opacity == 0.0 {
        return Rect::default();
    }
    let visible = node.bounds.intersect(destination);
    if visible.empty() || !node.filters.is_empty() {
        return visible;
    }
    let mut bounds = Rect::default();
    if let Some(primitive) = &node.primitive {
        if let Some(vector) = &primitive.vector {
            bounds = effect_render_bounds(vector, visible);
        } else if primitive.fill.is_none() && primitive.stroke.is_some() && primitive.image.is_none() {
            bounds = stroke_region_bounds(primitive, visible);
        } else {
            bounds = primitive.bounds.intersect(visible);
        }
    }
    for child in &node.children {
        bounds = union_rect(bounds, effect_render_bounds(child, visible));
    }
    bounds.intersect(visible)
}

fn stroke_region_bounds(primitive: &PreparedPrimitive, destination: Rect) -> Rect {
    let fallback = primitive.bounds.intersect(destination);
    let limit = (1u64 << 20) as f64;
    if fallback.empty()
        || (primitive.reference_bounds.min.x as f64).abs() > limit
        || (primitive.reference_bounds.min.y as f64).abs() > limit
        || !bounded_region_point(Point::new(primitive.transform.e, primitive.transform.f))
    {
        return fallback;
    }
    let stroke = primitive.stroke.as_ref().unwrap();
    let scale = primitive.transform.max_scale();
    let expansion = stroke_region_extent(stroke, &primitive.transform) + 1.0;
    if !finite(expansion) || expansion > limit {
        return fallback;
    }
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let lower_y = destination.min.y as f64 - expansion;
    let upper_y = destination.max.y as f64 + expansion;
    for run in &primitive.stroke_runs {
        let mut count = run.points.len() as i64 - 1;
        if run.closed {
            count += 1;
        }
        for index in 0..count.max(0) as usize {
            let local_from = run.points[index];
            let local_to = run.points[(index + 1) % run.points.len()];
            if !bounded_stroke_local_point(local_from, scale) || !bounded_stroke_local_point(local_to, scale) {
                return fallback;
            }
            let from = mpoint(&primitive.transform, local_from);
            let to = mpoint(&primitive.transform, local_to);
            if !bounded_region_point(from) || !bounded_region_point(to) {
                return fallback;
            }
            if go_max(from.y, to.y) < lower_y || go_min(from.y, to.y) > upper_y {
                continue;
            }
            let (mut first, mut last) = (0.0, 1.0);
            let (dx, dy) = (to.x - from.x, to.y - from.y);
            if dy != 0.0 {
                let (t0, t1) = ((lower_y - from.y) / dy, (upper_y - from.y) / dy);
                first = go_max(0.0, go_min(t0, t1));
                last = go_min(1.0, go_max(t0, t1));
            }
            // ARM64 stroke_regions.go:100–101.
            let (x0, x1) = (go_fmadd(first, dx, from.x), go_fmadd(last, dx, from.x));
            let (y0, y1) = (go_fmadd(first, dy, from.y), go_fmadd(last, dy, from.y));
            min_x = go_min(go_min(min_x, x0), x1);
            min_y = go_min(go_min(min_y, y0), y1);
            max_x = go_max(go_max(max_x, x0), x1);
            max_y = go_max(go_max(max_y, y0), y1);
        }
    }
    if min_x == f64::INFINITY {
        return Rect::default();
    }
    let min_x = go_max(min_x - expansion, destination.min.x as f64);
    let min_y = go_max(min_y - expansion, destination.min.y as f64);
    let max_x = go_min(max_x + expansion, destination.max.x as f64);
    let max_y = go_min(max_y + expansion, destination.max.y as f64);
    if min_x >= max_x || min_y >= max_y {
        return Rect::default();
    }
    Rect::new(min_x.floor() as i64, min_y.floor() as i64, max_x.ceil() as i64, max_y.ceil() as i64).intersect(destination)
}

pub fn bounded_region_point(point: Point) -> bool {
    let limit = (1u64 << 20) as f64;
    finite_point(point) && point.x.abs() <= limit && point.y.abs() <= limit
}

fn stroke_region_extent(stroke: &PreparedStroke, transform: &Matrix) -> f64 {
    let mut extent = stroke_extent(stroke, transform);
    if stroke.cap == LineCap::ROUND || stroke.join == LineJoin::ROUND {
        const CIRCLE_CONTROL: f64 = 0.5522847498307936;
        extent = go_max(extent, stroke.width / 2.0 * transform.max_scale() * gomath::hypot(1.0, CIRCLE_CONTROL));
    }
    extent
}

fn bounded_stroke_local_point(point: Point, scale: f64) -> bool {
    finite_point(point) && go_max(point.x.abs(), point.y.abs()) * scale <= (1u64 << 40) as f64
}
