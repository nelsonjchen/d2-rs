// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Runtime LoopOffsets content. Go map traversal and alias identity are
//! separate contracts; this ordered representation preserves keys and bits.
use super::*;

pub(super) type LoopOffsetsMap = d2_rs_core::TalaLoopOffsets;

/// Go geo.Orientation numbering differs from this Rust enum's declaration.
pub(super) fn loop_orientation_key(orientation: Orientation) -> i64 {
    match orientation {
        Orientation::TopLeft => 0,
        Orientation::TopRight => 1,
        Orientation::BottomLeft => 2,
        Orientation::BottomRight => 3,
        Orientation::Top => 4,
        Orientation::Right => 5,
        Orientation::Bottom => 6,
        Orientation::Left => 7,
        Orientation::None => 8,
    }
}

pub(super) fn loop_offset_toward(offsets: Option<&LoopOffsetsMap>, side: Orientation) -> f64 {
    offsets
        .and_then(|map| map.get(&loop_orientation_key(side)))
        .copied()
        .unwrap_or(0.0)
}

pub(super) fn loop_cardinal_extents(offsets: Option<&LoopOffsetsMap>) -> [f64; 4] {
    [
        Orientation::Top,
        Orientation::Right,
        Orientation::Bottom,
        Orientation::Left,
    ]
    .map(|side| loop_offset_toward(offsets, side))
}

// Pinned Go 1.27 darwin/arm64 int conversion returns MinInt64 for NaN,
// infinities and values outside the signed 64-bit range. Rust's `as i64`
// saturates instead. Ordinary finite offsets truncate independently.
pub(super) fn loop_spacing_units(value: f64) -> i64 {
    if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value) {
        value as i64
    } else {
        i64::MIN
    }
}

pub(super) fn loop_spacing_gap(
    first: Option<&LoopOffsetsMap>,
    second: Option<&LoopOffsetsMap>,
    orientation: Orientation,
) -> f64 {
    20_i64
        .wrapping_add(loop_spacing_units(loop_offset_toward(
            first,
            orientation.opposite(),
        )))
        .wrapping_add(loop_spacing_units(loop_offset_toward(second, orientation))) as f64
}

/// loops.computeOffsets publishes all eight keys after envelope extension,
/// including zero cardinal values. Empty envelope equality is checked first.
pub(super) fn computed_loop_offsets([top, right, bottom, left]: [f64; 4]) -> LoopOffsetsMap {
    LoopOffsetsMap::from([
        (0, top.max(left)),
        (1, top.max(right)),
        (2, bottom.max(left)),
        (3, bottom.max(right)),
        (4, top),
        (5, right),
        (6, bottom),
        (7, left),
    ])
}

/// Node.BoundingBox(nil) immediately after computeOffsets replaces the map.
/// Keep raw top-left coordinates; only bottom-right dimensions are rounded.
pub(super) fn loop_base_bounds_without_offsets(node: &ArenaNode, rect: Rect) -> (Point, Point) {
    let (modifier_x, modifier_y) = bounds::modifier_element_adjustments(node);
    (
        Point {
            x: rect.origin.x,
            y: rect.origin.y - modifier_y,
        },
        Point {
            x: rect.right().round() + modifier_x,
            y: rect.bottom().round(),
        },
    )
}

impl ArenaGraph {
    /// loops.UpdateOffsets preserves a nil TopLeft while routing from zero,
    /// replaces the map, and discards the precomputation's absolute routes.
    pub(super) fn update_loop_offsets(&mut self, node: NodeId) {
        let offsets = self.compute_loop_offsets(node);
        for edge_index in loops::ordered_self_loop_edges(self, node) {
            let edge = &mut self.edge_records[edge_index];
            edge.route.replace(RoutePoints::nil());
            if let Some(label) = &mut edge.label {
                label.position = LabelPosition::OutsideTopCenter;
            }
        }
        self.nodes[node.0 as usize].loop_offsets = Some(offsets);
    }

    pub(super) fn compute_loop_offsets(&self, node: NodeId) -> LoopOffsetsMap {
        let value = &self.nodes[node.0 as usize];
        let rect = Rect {
            // Go's loops.UpdateOffsets routes against the node's current
            // TopLeft.  Keeping that absolute origin matters because
            // GetPointOnRoute deliberately performs arithmetic on the
            // absolute coordinates before chopping to float32 precision;
            // translating routes to (0,0) can change a half-pixel round.
            origin: value.position.unwrap_or(Point { x: 0.0, y: 0.0 }),
            size: value.rect.size,
        };
        let routes = crate::internal::routing::self_loop_routes(self, node, rect);
        if routes.is_empty() {
            return LoopOffsetsMap::new();
        }

        // TALA computes each loop's complete edge bounding box at the origin,
        // including the outside label and arrowhead labels, then unions those
        // boxes with the node box.  The rounded side differences are cached as
        // LoopOffsets and consumed by every later placement/bounds operation.
        // computeOffsets first replaces the old map, then calls
        // Node.BoundingBox(nil): modifiers count, outside labels/icons do not.
        let (top_left, bottom_right) = loop_base_bounds_without_offsets(value, rect);
        let mut loop_top_left = top_left;
        let mut loop_bottom_right = bottom_right;
        for (edge_index, points, _) in routes {
            let edge = &self.edge_records[edge_index];
            let mut edge_top_left = Point {
                x: f64::INFINITY,
                y: f64::INFINITY,
            };
            let mut edge_bottom_right = Point {
                x: f64::NEG_INFINITY,
                y: f64::NEG_INFINITY,
            };
            for point in &points {
                edge_top_left.x = edge_top_left.x.min(point.x);
                edge_top_left.y = edge_top_left.y.min(point.y);
                edge_bottom_right.x = edge_bottom_right.x.max(point.x);
                edge_bottom_right.y = edge_bottom_right.y.max(point.y);
            }
            if let Some(label) = &edge.label
                && let Some(top_left) = crate::internal::labels::route_label_top_left(
                    &points,
                    LabelPosition::OutsideTopCenter,
                    0.0,
                    label.size.width,
                    label.size.height,
                    3.0,
                )
            {
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_LOOP_COMPONENTS") {
                    eprintln!(
                        "LOOP_COMPONENT_RUST node={} edge={} kind=main-label rect={:?}->{:?} size={:?}",
                        value.tala_id,
                        edge_index,
                        top_left,
                        Point {
                            x: top_left.x + label.size.width,
                            y: top_left.y + label.size.height
                        },
                        label.size,
                    );
                }
                edge_top_left.x = edge_top_left.x.min(top_left.x);
                edge_top_left.y = edge_top_left.y.min(top_left.y);
                edge_bottom_right.x = edge_bottom_right.x.max(top_left.x + label.size.width);
                edge_bottom_right.y = edge_bottom_right.y.max(top_left.y + label.size.height);
            }
            for target in [false, true] {
                if let Some(label_rect) =
                    crate::internal::labels::arrowhead_label_rect_for_route(edge, &points, target)
                {
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_LOOP_COMPONENTS") {
                        eprintln!(
                            "LOOP_COMPONENT_RUST node={} edge={} kind=arrow target={} rect={:?} size={:?} arrows={:?},{:?}",
                            value.tala_id,
                            edge_index,
                            target,
                            label_rect,
                            if target {
                                edge.target_arrowhead_label.as_ref().map(|l| l.size)
                            } else {
                                edge.source_arrowhead_label.as_ref().map(|l| l.size)
                            },
                            edge.source_arrowhead,
                            edge.target_arrowhead,
                        );
                    }
                    edge_top_left.x = edge_top_left.x.min(label_rect.origin.x);
                    edge_top_left.y = edge_top_left.y.min(label_rect.origin.y);
                    edge_bottom_right.x = edge_bottom_right.x.max(label_rect.right());
                    edge_bottom_right.y = edge_bottom_right.y.max(label_rect.bottom());
                }
            }
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_LOOP_COMPONENTS") {
                eprintln!(
                    "LOOP_COMPONENT_RUST node={} edge={} kind=route bbox={:?}->{:?}",
                    value.tala_id,
                    edge_index,
                    points.iter().fold(
                        Point {
                            x: f64::INFINITY,
                            y: f64::INFINITY
                        },
                        |mut p, q| {
                            p.x = p.x.min(q.x);
                            p.y = p.y.min(q.y);
                            p
                        }
                    ),
                    points.iter().fold(
                        Point {
                            x: f64::NEG_INFINITY,
                            y: f64::NEG_INFINITY
                        },
                        |mut p, q| {
                            p.x = p.x.max(q.x);
                            p.y = p.y.max(q.y);
                            p
                        }
                    ),
                );
            }
            edge_top_left.x = edge_top_left.x.round();
            edge_top_left.y = edge_top_left.y.round();
            edge_bottom_right.x = edge_bottom_right.x.round();
            edge_bottom_right.y = edge_bottom_right.y.round();
            if crate::internal::trace_env_enabled("WEFTAN_TRACE_LOOP_BOUNDS") {
                eprintln!(
                    "LOOP_BOUNDS_RUST node={} edge={} points={:?} bbox={:?}->{:?} label={:?}",
                    value.tala_id,
                    edge_index,
                    points,
                    edge_top_left,
                    edge_bottom_right,
                    edge.label.as_ref().map(|label| &label.size),
                );
            }
            loop_top_left.x = loop_top_left.x.min(edge_top_left.x);
            loop_top_left.y = loop_top_left.y.min(edge_top_left.y);
            loop_bottom_right.x = loop_bottom_right.x.max(edge_bottom_right.x);
            loop_bottom_right.y = loop_bottom_right.y.max(edge_bottom_right.y);
        }
        if top_left == loop_top_left && bottom_right == loop_bottom_right {
            return LoopOffsetsMap::new();
        }
        computed_loop_offsets([
            (top_left.y - loop_top_left.y).abs().round(),
            (loop_bottom_right.x - bottom_right.x).abs().round(),
            (loop_bottom_right.y - bottom_right.y).abs().round(),
            (top_left.x - loop_top_left.x).abs().round(),
        ])
    }

    /// Cardinal projection for rectangle math. A nil map is not recomputed:
    /// NewNode, decoding and full Graph.Clone deliberately start nil.
    pub(super) fn loop_spacing_extents(&self, node: NodeId) -> Option<[f64; 4]> {
        self.nodes[node.0 as usize]
            .loop_offsets
            .as_ref()
            .map(|offsets| loop_cardinal_extents(Some(offsets)))
    }

    pub(super) fn loop_extent_toward(&self, node: NodeId, orientation: Orientation) -> f64 {
        loop_offset_toward(
            self.nodes[node.0 as usize].loop_offsets.as_ref(),
            orientation,
        )
    }
}
