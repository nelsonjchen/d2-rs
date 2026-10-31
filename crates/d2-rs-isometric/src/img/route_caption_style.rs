// SPDX-License-Identifier: MPL-2.0
//! route_caption_style.go: authored caption sizes, anchors and source metrics.
use super::{
    geometry::LabelSurface, geometry_routes::route_lengths, math as gomath, mesh::*,
    route_labels::caption_finite,
};
use crate::target::{Connection, Text};
use d2_rs_scene::{go_max, go_min};
use d2_rs_scenebuild::golib::{
    geo::Point,
    label::{PADDING, Position},
};
pub fn native_caption_on_edge(c: &Connection) -> bool {
    Position::from_string(&c.label_position).is_on_edge()
}
pub fn native_route_caption_size(style: &Text, icon: bool, scale: f64) -> (f64, f64) {
    let mut width = go_max(1.0, style.label_width as f64 * scale + 0.1);
    let lh = if style.label_height <= 0 {
        25
    } else {
        style.label_height
    };
    let mut depth = go_max(0.25, lh as f64 * scale + 0.06);
    if icon {
        if style.label.is_empty() {
            return (0.38, 0.38);
        }
        width = go_max(width, style.label_width as f64 * scale * 1.06 + 0.04) + 0.38 + 0.08;
        depth = go_max(depth, 0.38)
    }
    (width, depth)
}
pub fn connection_label_top_left(c: &Connection) -> Option<Point> {
    let route: Option<std::vec::Vec<_>> = c
        .route()
        .iter()
        .map(|p| p.map(|p| Point::new(p.x, p.y)))
        .collect();
    Position::from_string(&c.label_position)
        .try_get_point_on_route(
            &route?,
            c.stroke_width as f64,
            c.label_percentage,
            c.text.label_width as f64,
            c.text.label_height as f64,
        )
        .map(|p| p.0)
}
pub fn native_caption_source_center(points: &[Vec], c: &Connection, scale: f64) -> Option<Vec> {
    if c.route().len() < 2 || c.route().len() > 10000 {
        return None;
    }
    for p in c.route() {
        let p = p.as_ref()?;
        if !caption_finite(&[p.x, p.y]) || p.x.abs() > 1e9 || p.y.abs() > 1e9 {
            return None;
        }
    }
    let p = connection_label_top_left(c)?;
    if !caption_finite(&[p.x, p.y]) {
        return None;
    }
    let first = c.route()[0].as_ref()?;
    Some(nv(
        (p.x + c.text.label_width as f64 / 2.0 - first.x).mul_add(scale, points[0].x),
        0.0,
        (p.y + c.text.label_height as f64 / 2.0 - first.y).mul_add(scale, points[0].z),
    ))
}
pub fn native_connection_caption_surface(
    points: &[Vec],
    c: &Connection,
    width: f64,
    depth: f64,
    scale: f64,
) -> Option<LabelSurface> {
    use Position::*;
    let position = Position::from_string(&c.label_position);
    let fraction = match position {
        InsideMiddleLeft | OutsideTopLeft | OutsideBottomLeft => 0.25,
        InsideMiddleCenter | OutsideTopCenter | OutsideBottomCenter => 0.5,
        InsideMiddleRight | OutsideTopRight | OutsideBottomRight => 0.75,
        UnlockedTop | UnlockedMiddle | UnlockedBottom => {
            go_max(0.0, go_min(1.0, c.label_percentage))
        }
        _ => return None,
    };
    let side = match position {
        OutsideTopLeft | OutsideTopCenter | OutsideTopRight | UnlockedTop => -1.0,
        OutsideBottomLeft | OutsideBottomCenter | OutsideBottomRight | UnlockedBottom => 1.0,
        _ => 0.0,
    };
    if !caption_finite(&[fraction, width, depth, scale])
        || width <= 0.0
        || depth <= 0.0
        || scale <= 0.0
        || points.len() < 2
        || points.len() > 10000
    {
        return None;
    }
    let lengths = route_lengths(points);
    let total = lengths[lengths.len() - 1];
    if total <= 1e-9 || !caption_finite(&[total]) {
        return None;
    }
    let distance = fraction * total;
    for i in 1..points.len() {
        let length = lengths[i] - lengths[i - 1];
        if length <= 1e-9 || lengths[i] < distance {
            continue;
        }
        let mut direction = nunit(nsub(points[i], points[i - 1]));
        let mut center = nlerp(
            points[i - 1],
            points[i],
            (-fraction).mul_add(total, lengths[i - 1]) / -length,
        );
        let offset = side * (c.stroke_width as f64 / 2.0 + PADDING).mul_add(scale, depth / 2.0);
        center.x = (-direction.z).mul_add(offset, center.x);
        center.z = direction.x.mul_add(offset, center.z);
        center.y += 0.006;
        if direction.x < -1e-6 || direction.x.abs() < 1e-6 && direction.z < 0.0 {
            direction = nmul(direction, -1.0)
        }
        let mut angle = gomath::atan2(direction.z, direction.x);
        let (start, end) = native_caption_route_run(points, &lengths, i);
        if width + 2.0 * depth
            > 2.0
                * go_min(
                    fraction.mul_add(total, -start),
                    (-fraction).mul_add(total, end),
                )
                + 1e-9
        {
            angle = 0.0;
            if let Some(source) = native_caption_source_center(points, c, scale) {
                center.x = source.x;
                center.z = source.z
            }
        }
        return Some(LabelSurface {
            center,
            width,
            depth,
            angle,
            ..Default::default()
        });
    }
    None
}
fn native_caption_route_run(points: &[Vec], lengths: &[f64], index: usize) -> (f64, f64) {
    let direction = nunit(nsub(points[index], points[index - 1]));
    let aligned = |i: usize| {
        lengths[i] - lengths[i - 1] <= 1e-9
            || ndot(direction, nunit(nsub(points[i], points[i - 1]))) >= 3.0f64.sqrt() / 2.0
    };
    let (mut start, mut end) = (lengths[index - 1], lengths[index]);
    for i in (1..index).rev() {
        if !aligned(i) {
            break;
        }
        start = lengths[i - 1]
    }
    for i in index + 1..points.len() {
        if !aligned(i) {
            break;
        }
        end = lengths[i]
    }
    (start, end)
}
