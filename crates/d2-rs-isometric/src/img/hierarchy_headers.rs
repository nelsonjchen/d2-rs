// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg hierarchy_headers.go: container titles placed at their
//! compiled label position without being occluded by descendants.
//! Contraction sites are marked `ARM64:`; products by 0.5 are exact.

use std::collections::HashMap;

use d2_rs_core::go_sort;
use d2_rs_render::go_float::go_fmadd;
use d2_rs_scene::{go_max, go_min};
use d2_rs_scenebuild::golib::geo;
use d2_rs_scenebuild::golib::label::{self, Position};

use super::geometry::{LabelSurface, native_paint};
use super::hierarchy_boards::{
    hierarchy_node_relief, hierarchy_physical_plate, hierarchy_surface_y,
};
use super::hierarchy_support::hierarchy_support_offsets;
use super::markdown_cards::{
    MARKDOWN_CARD_PAPER_DEPTH, native_markdown_card, native_markdown_card_inset,
    native_plain_markdown_card,
};
use super::mesh::native_view_direction;
use super::native_shapes::native_relief_symbol;
use super::native_solids::native_solid_node;
use super::native_structured::native_structured_node;
use crate::scene::{Board, MAX_NODES, Node, SCENE_SCALE};

/// Places the existing print area at the compiled container label position.
pub fn hierarchy_board_header_surface(
    surface: LabelSurface,
    board: &Board,
    owner: &Node,
    nodes: &[Node],
    pixel_scale: f64,
    boards: Option<&[Board]>,
) -> LabelSurface {
    hierarchy_board_header_placement(surface, board, owner, nodes, pixel_scale, boards).0
}

#[derive(Clone, Copy, Debug)]
struct HierarchyHeaderObstacle {
    x0: f64,
    x1: f64,
    z0: f64,
    z1: f64,
}

/// The bool reports whether the full print area has an unoccluded placement.
pub fn hierarchy_board_header_placement(
    mut surface: LabelSurface,
    board: &Board,
    owner: &Node,
    nodes: &[Node],
    pixel_scale: f64,
    boards: Option<&[Board]>,
) -> (LabelSurface, bool) {
    if !positive(board.size.x)
        || !positive(board.size.z)
        || !positive(surface.width)
        || !positive(surface.depth)
    {
        return (surface, false);
    }
    let pixel_scale = if positive(pixel_scale) {
        pixel_scale
    } else {
        SCENE_SCALE
    };
    let (pad_x, pad_z) = (
        go_min(0.05, board.size.x * 0.02),
        go_min(0.05, board.size.z * 0.02),
    );
    let (available_x, available_z) = (board.size.x - 2.0 * pad_x, board.size.z - 2.0 * pad_z);
    let fit = go_min(
        1.0,
        go_min(available_x / surface.width, available_z / surface.depth),
    );
    surface.width *= fit;
    surface.depth *= fit;
    let mut position = Position::from_string(&owner.metadata.original.label_position);
    if position == Position::Unset {
        position = Position::InsideTopCenter;
    }
    let bx = geo::Box::new(
        geo::Point::new(0.0, 0.0),
        board.size.x / pixel_scale,
        board.size.z / pixel_scale,
    );
    let tl = position.get_point_on_box(
        &bx,
        label::PADDING,
        surface.width / pixel_scale,
        surface.depth / pixel_scale,
    );
    let (left, front) = (
        board.position.x - board.size.x / 2.0,
        board.position.z - board.size.z / 2.0,
    );
    // ARM64 hierarchy_headers.go:44/48–49.
    let source_x = go_fmadd(tl.x, pixel_scale, left) + surface.width / 2.0;
    let source_z = go_fmadd(tl.y, pixel_scale, front) + surface.depth / 2.0;
    let (x0, x1) = (
        left + pad_x + surface.width / 2.0,
        left + board.size.x - pad_x - surface.width / 2.0,
    );
    let (z0, z1) = (
        front + pad_z + surface.depth / 2.0,
        front + board.size.z - pad_z - surface.depth / 2.0,
    );
    let clamp = |v: f64, lo: f64, hi: f64| go_max(lo, go_min(hi, v));
    surface.center.x = clamp(source_x, x0, x1);
    surface.center.z = clamp(source_z, z0, z1);
    let mut obstacles: std::vec::Vec<HierarchyHeaderObstacle> = vec![];
    let mut markdown_child = false;
    let supports = boards.map(hierarchy_support_offsets).unwrap_or_default();
    let view = native_view_direction();
    let surface_y = surface.center.y;
    let (sw, sd) = (surface.width, surface.depth);
    let append_box = |obstacles: &mut std::vec::Vec<HierarchyHeaderObstacle>,
                      node: &Node,
                      half_x: f64,
                      half_z: f64,
                      bottom: f64,
                      top: f64| {
        if top < surface_y {
            return;
        }
        let bottom = go_max(bottom, surface_y);
        let (dx0, dx1) = (
            (bottom - surface_y) * view.x / view.y,
            (top - surface_y) * view.x / view.y,
        );
        let (dz0, dz1) = (
            (bottom - surface_y) * view.z / view.y,
            (top - surface_y) * view.z / view.y,
        );
        let obstacle = HierarchyHeaderObstacle {
            x0: node.position.x - half_x - go_max(dx0, dx1) - 0.005,
            x1: node.position.x + half_x - go_min(dx0, dx1) + 0.005,
            z0: node.position.z - half_z - go_max(dz0, dz1) - 0.005,
            z1: node.position.z + half_z - go_min(dz0, dz1) + 0.005,
        };
        let inside = obstacle.x1 >= left
            && obstacle.x0 <= left + board.size.x
            && obstacle.z1 >= front
            && obstacle.z0 <= front + board.size.z;
        let outside = position.is_outside()
            && obstacle.x1 >= source_x - sw / 2.0
            && obstacle.x0 <= source_x + sw / 2.0
            && obstacle.z1 >= source_z - sd / 2.0
            && obstacle.z0 <= source_z + sd / 2.0;
        if inside || outside {
            obstacles.push(obstacle);
        }
    };
    for node in &nodes[..nodes.len().min(MAX_NODES)] {
        if node.container || node.opacity <= 0.0 || !positive(node.size.x) || !positive(node.size.z)
        {
            continue;
        }
        markdown_child = markdown_child || native_markdown_card(node) && node.board_id == board.id;
        let relief = hierarchy_node_relief(node);
        if native_plain_markdown_card(node) {
            let (floor, top) = (
                node.position.y - node.size.y / 2.0,
                node.position.y + node.size.y / 2.0,
            );
            let inset = native_markdown_card_inset(node);
            append_box(
                &mut obstacles,
                node,
                node.size.x / 2.0 - inset,
                node.size.z / 2.0 - inset,
                floor,
                top - MARKDOWN_CARD_PAPER_DEPTH,
            );
            append_box(
                &mut obstacles,
                node,
                node.size.x / 2.0,
                node.size.z / 2.0,
                top - MARKDOWN_CARD_PAPER_DEPTH,
                top + 0.002,
            );
            continue;
        }
        if native_solid_node(node)
            || native_relief_symbol(node)
            || native_structured_node(node)
            || native_markdown_card(node)
        {
            if native_structured_node(node)
                || native_markdown_card(node)
                || !(node.fill_explicit && native_paint(&node.fill, "#edf1f7").a == 0)
            {
                let floor = node.position.y - node.size.y / 2.0;
                // ARM64 hierarchy_headers.go:98.
                append_box(
                    &mut obstacles,
                    node,
                    node.size.x / 2.0,
                    node.size.z / 2.0,
                    floor,
                    go_fmadd(0.002, relief, floor + node.size.y),
                );
            }
            continue;
        }
        let floor = node.position.y - node.size.y / 2.0;
        let trim = go_min(1.0, go_min(node.size.x / 0.6, node.size.z / 0.5));
        // ARM64 hierarchy_headers.go:109.
        append_box(
            &mut obstacles,
            node,
            go_fmadd(0.055, trim, node.size.x / 2.0),
            go_fmadd(0.05, trim, node.size.z / 2.0),
            floor,
            go_fmadd(0.09 * trim, relief, floor),
        );
        if !(node.fill_explicit && native_paint(&node.fill, "#edf1f7").a == 0) {
            let drop = supports.get(&node.board_id).copied().unwrap_or(0.0);
            let mut h = go_max(0.3 * relief, node.size.y + drop);
            let (half_x, half_z) = (node.size.x / 2.0, node.size.z / 2.0);
            if node.type_ == "diamond" {
                h *= 1.28;
            }
            h -= drop;
            // ARM64 hierarchy_headers.go:119.
            append_box(
                &mut obstacles,
                node,
                half_x,
                half_z,
                floor,
                go_fmadd(0.12, relief, floor + h),
            );
        }
    }
    if let Some(boards) = boards {
        let owners: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        for candidate in boards {
            let Some(node) = owners.get(candidate.source_id.as_str()).copied() else {
                continue;
            };
            if candidate.id == board.id
                || node.opacity <= 0.0
                || !hierarchy_physical_plate(
                    candidate,
                    Some(node),
                    native_paint(&node.fill, "transparent").a,
                )
            {
                continue;
            }
            let depth = if candidate.kind == "platform" {
                0.13
            } else {
                candidate.size.y
            };
            // Include the raised cap's centered source outline and its wall.
            let cap = hierarchy_surface_y(candidate);
            if cap <= surface_y + 0.001 {
                continue;
            }
            let margin = node.stroke_width as f64 * pixel_scale / 2.0;
            let mut copy = node.clone();
            copy.position = candidate.position;
            append_box(
                &mut obstacles,
                &copy,
                candidate.size.x / 2.0 + margin,
                candidate.size.z / 2.0 + margin,
                cap - depth,
                cap,
            );
        }
    }
    let name = position.string();
    let vertical = name.contains("_LEFT_")
        || name.contains("_RIGHT_")
        || name == "INSIDE_MIDDLE_LEFT"
        || name == "INSIDE_MIDDLE_RIGHT";
    if let Some(placed) = hierarchy_header_strip(&surface, &obstacles, x0, x1, z0, z1, vertical) {
        return (placed, true);
    }
    // At most eleven additional interval searches.
    for fraction in [1.0, 0.0, 0.5, 0.25, 0.75, 0.125, 0.875, 0.375, 0.625] {
        let mut candidate = surface.clone();
        // ARM64 hierarchy_headers.go:157.
        candidate.center.z = go_fmadd(z1 - z0, fraction, z0);
        if let Some(placed) = hierarchy_header_strip(&candidate, &obstacles, x0, x1, z0, z1, false)
        {
            return (placed, true);
        }
    }
    for x in [x0, x1] {
        let mut candidate = surface.clone();
        candidate.center.x = x;
        if let Some(placed) = hierarchy_header_strip(&candidate, &obstacles, x0, x1, z0, z1, true) {
            return (placed, true);
        }
    }
    if markdown_child {
        // A source title can use the existing bottom margin right up to a
        // clear border.
        let pad = go_max(
            2.0 * pixel_scale,
            (owner.stroke_width as f64 / 2.0 + 1.0) * pixel_scale,
        );
        if pad < pad_z {
            let mut bottom = surface.clone();
            bottom.center.z = front + board.size.z - pad - bottom.depth / 2.0;
            let z = bottom.center.z;
            if let Some(placed) = hierarchy_header_strip(&bottom, &obstacles, x0, x1, z, z, false) {
                return (placed, true);
            }
        }
    }
    if markdown_child && position.is_outside() {
        // Retain the authored outside anchor.
        let mut outside = surface.clone();
        outside.center.x = source_x;
        outside.center.z = source_z;
        if let Some(placed) = hierarchy_header_strip(
            &outside, &obstacles, source_x, source_x, source_z, source_z, false,
        ) {
            return (placed, true);
        }
    }
    (surface, false)
}

fn hierarchy_header_strip(
    surface: &LabelSurface,
    obstacles: &[HierarchyHeaderObstacle],
    x0: f64,
    x1: f64,
    z0: f64,
    z1: f64,
    vertical: bool,
) -> Option<LabelSurface> {
    let (mut axis, mut cross, mut span, mut cross_span) = (
        surface.center.x,
        surface.center.z,
        surface.width,
        surface.depth,
    );
    let (mut low, mut high) = (x0, x1);
    if vertical {
        (axis, cross, span, cross_span) = (
            surface.center.z,
            surface.center.x,
            surface.depth,
            surface.width,
        );
        (low, high) = (z0, z1);
    }
    let mut blocked: std::vec::Vec<(f64, f64)> = vec![];
    for o in obstacles {
        let (mut a, mut b, mut c, mut d) = (o.x0, o.x1, o.z0, o.z1);
        if vertical {
            (a, b, c, d) = (c, d, a, b);
        }
        if cross + cross_span / 2.0 < c || cross - cross_span / 2.0 > d {
            continue;
        }
        (a, b) = (a - span / 2.0, b + span / 2.0);
        if b >= low && a <= high {
            blocked.push((go_max(low, a), go_min(high, b)));
        }
    }
    go_sort::sort_by(
        &mut blocked,
        |x, y| if x.0 != y.0 { x.0 < y.0 } else { x.1 < y.1 },
    );
    let (mut best, mut distance, mut cursor) = (axis, f64::INFINITY, low);
    let mut consider = |a: f64, b: f64| {
        if a > b {
            return;
        }
        let candidate = go_max(a, go_min(b, axis));
        let d = (candidate - axis).abs();
        if d < distance {
            best = candidate;
            distance = d;
        }
    };
    for o in &blocked {
        consider(cursor, o.0 - 0.001);
        cursor = go_max(cursor, o.1 + 0.001);
    }
    consider(cursor, high);
    if distance.is_infinite() {
        return None;
    }
    let mut surface = surface.clone();
    if vertical {
        surface.center.z = best;
    } else {
        surface.center.x = best;
    }
    Some(surface)
}

fn positive(v: f64) -> bool {
    v > 0.0 && v.is_finite()
}
