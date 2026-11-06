// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Copyright 2022 Terrastruct Inc.
// Rust translation and modifications: Copyright 2026 Nelson Chen.

//! D2 route label placement shared by container fitting and target rendering.
//!
//! The input is the stored route polyline, including cubic control points for
//! curved connections. D2 does not sample the rendered Bezier path here.

use crate::{LabelPosition, Point, Size};

fn euclidean_distance(start: Point, end: Point) -> f64 {
    // D2 lib/geo/math.go, including the axis-aligned fast paths.
    if start.x == end.x {
        (start.y - end.y).abs()
    } else if start.y == end.y {
        (start.x - end.x).abs()
    } else {
        let dx = start.x - end.x;
        let dy = start.y - end.y;
        (dx * dx + dy * dy).sqrt()
    }
}

fn interpolate(start: Point, end: Point, t: f64) -> Point {
    // lib/geo/point.go:Point.Interpolate rounds the second product, then
    // fuses the first product on arm64. Near chopPrecision's float32
    // boundary, separate products can move a connection mask by a pixel.
    Point {
        x: crate::go_fmadd(start.x, 1.0 - t, end.x * t),
        y: crate::go_fmadd(start.y, 1.0 - t, end.y * t),
    }
}

fn point_at_distance(route: &[Point], distance: f64) -> Option<(Point, usize)> {
    if route.len() < 2 {
        return None;
    }
    let mut remaining = distance;
    for (index, segment) in route.windows(2).enumerate() {
        let length = euclidean_distance(segment[0], segment[1]);
        if remaining <= length {
            return Some((
                interpolate(segment[0], segment[1], remaining / length),
                index,
            ));
        }
        remaining -= length;
    }
    let index = route.len() - 2;
    let start = route[index];
    let end = route[index + 1];
    Some((
        interpolate(start, end, 1.0 + remaining / euclidean_distance(start, end)),
        index,
    ))
}

fn chop_precision(value: f64) -> f64 {
    // D2 lib/label/label.go:chopPrecision rounds to an integer after narrowing
    // the scaled value to float32. This is not four-decimal-place rounding.
    let result = (f64::from((value * 10000.0) as f32) / 10000.0).round();
    if result == 0.0 { 0.0 } else { result }
}

/// D2 v0.9.0 `Position.GetPointOnRoute` with the target stroke width and
/// unlocked label percentage. The caller resolves an unset position to
/// `InsideMiddleCenter`; positions unsupported for routes return `None`.
///
/// Routes shorter than two points return `None`. A zero-length segment keeps
/// upstream's NaN interpolation/normal behavior rather than being skipped.
pub fn top_left(
    route: &[Point],
    position: LabelPosition,
    stroke_width: f64,
    percentage: f64,
    size: Size,
) -> Option<Point> {
    let (fraction, offset) = match position {
        LabelPosition::InsideMiddleLeft => (0.25, None),
        LabelPosition::InsideMiddleCenter => (0.5, None),
        LabelPosition::InsideMiddleRight => (0.75, None),
        LabelPosition::OutsideTopLeft => (0.25, Some(true)),
        LabelPosition::OutsideTopCenter => (0.5, Some(true)),
        LabelPosition::OutsideTopRight => (0.75, Some(true)),
        LabelPosition::OutsideBottomLeft => (0.25, Some(false)),
        LabelPosition::OutsideBottomCenter => (0.5, Some(false)),
        LabelPosition::OutsideBottomRight => (0.75, Some(false)),
        LabelPosition::UnlockedTop => (percentage, Some(true)),
        LabelPosition::UnlockedMiddle => (percentage, None),
        LabelPosition::UnlockedBottom => (percentage, Some(false)),
        _ => return None,
    };
    let mut total_length = 0.0;
    for segment in route.windows(2) {
        total_length += euclidean_distance(segment[0], segment[1]);
    }
    let (mut center, index) = point_at_distance(route, fraction * total_length)?;
    if let Some(flip) = offset {
        let start = route[index];
        let end = route[index + 1];
        let length = euclidean_distance(start, end);
        let mut normal_x = (start.y - end.y) / length;
        let mut normal_y = (end.x - start.x) / length;
        if flip {
            normal_x *= -1.0;
            normal_y *= -1.0;
        }
        let offset_x = stroke_width / 2.0 + 5.0 + size.width / 2.0;
        let offset_y = stroke_width / 2.0 + 5.0 + size.height / 2.0;
        center.x += normal_x * offset_x;
        center.y += normal_y * offset_y;
    }
    Some(Point {
        x: chop_precision(center.x - size.width / 2.0),
        y: chop_precision(center.y - size.height / 2.0),
    })
}

#[cfg(test)]
mod tests {
    use super::{chop_precision, top_left};
    use crate::{LabelPosition, Point, Size};

    fn edge_label_top_left(route: &[Point], position: LabelPosition, size: Size) -> Option<Point> {
        top_left(route, position, 2.0, 0.0, size)
    }

    #[test]
    fn label_precision_narrows_scaled_value_before_integer_rounding() {
        assert_eq!(chop_precision(0.499_999_99), 1.0);
        assert_eq!(chop_precision(-0.499_999_99), -1.0);
        assert_eq!(chop_precision(1.2345), 1.0);
        assert_eq!(chop_precision(2.5), 3.0);
        assert_eq!(chop_precision(-2.5), -3.0);
        assert_eq!(chop_precision(-0.1).to_bits(), 0.0_f64.to_bits());
    }

    #[test]
    fn large_route_label_preserves_go_interpolation_before_chopping() {
        // feature-stress.py's themed/100, connection n73 -> n74. Pinned Go
        // lib/geo/point.go:Point.Interpolate fuses the first product on arm64;
        // lib/label/label.go:chopPrecision then narrows the scaled center.
        let route =
            [13716.5, 13765.2998046875, 13786.7001953125, 13823.5].map(|y| Point { x: 50.5, y });
        let origin = top_left(
            &route,
            LabelPosition::InsideMiddleCenter,
            2.0,
            0.0,
            Size {
                width: 8.0,
                height: 21.0,
            },
        )
        .unwrap();
        #[cfg(target_arch = "aarch64")]
        let y = 13760.0;
        #[cfg(not(target_arch = "aarch64"))]
        let y = 13759.0;
        assert_eq!(origin, Point { x: 47.0, y });
    }

    #[test]
    fn unlocked_labels_use_actual_stroke_and_allow_route_extrapolation() {
        let route = [Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 100.0 }];
        let size = Size {
            width: 20.0,
            height: 10.0,
        };
        assert_eq!(
            top_left(&route, LabelPosition::UnlockedTop, 8.0, 0.75, size),
            Some(Point { x: 9.0, y: 70.0 })
        );
        assert_eq!(
            top_left(&route, LabelPosition::UnlockedMiddle, 2.0, -0.25, size),
            Some(Point { x: -10.0, y: -30.0 })
        );
    }

    #[test]
    fn curve_labels_measure_the_control_polygon_without_bezier_sampling() {
        let route = [
            Point { x: 0.0, y: 0.0 },
            Point { x: 0.0, y: 100.0 },
            Point { x: 100.0, y: 100.0 },
            Point { x: 100.0, y: 0.0 },
        ];
        assert_eq!(
            top_left(
                &route,
                LabelPosition::InsideMiddleCenter,
                2.0,
                0.9,
                Size {
                    width: 20.0,
                    height: 10.0
                }
            ),
            Some(Point { x: 40.0, y: 95.0 })
        );
    }

    #[test]
    fn route_labels_follow_all_supported_positions() {
        let route = [Point { x: 0.0, y: 0.0 }, Point { x: 100.0, y: 0.0 }];
        let size = Size {
            width: 20.0,
            height: 10.0,
        };
        for (position, x, y) in [
            (LabelPosition::InsideMiddleLeft, 15.0, -5.0),
            (LabelPosition::InsideMiddleCenter, 40.0, -5.0),
            (LabelPosition::InsideMiddleRight, 65.0, -5.0),
            (LabelPosition::OutsideTopLeft, 15.0, -16.0),
            (LabelPosition::OutsideTopCenter, 40.0, -16.0),
            (LabelPosition::OutsideTopRight, 65.0, -16.0),
            (LabelPosition::OutsideBottomLeft, 15.0, 6.0),
            (LabelPosition::OutsideBottomCenter, 40.0, 6.0),
            (LabelPosition::OutsideBottomRight, 65.0, 6.0),
            (LabelPosition::UnlockedTop, -10.0, -16.0),
            (LabelPosition::UnlockedMiddle, -10.0, -5.0),
            (LabelPosition::UnlockedBottom, -10.0, 6.0),
        ] {
            assert_eq!(
                edge_label_top_left(&route, position, size),
                Some(Point { x, y }),
                "{position:?}"
            );
        }
        assert_eq!(
            edge_label_top_left(&route, LabelPosition::Unset, size),
            None
        );
        assert_eq!(
            edge_label_top_left(&route, LabelPosition::BorderTopCenter, size),
            None
        );
    }

    #[test]
    fn outside_label_normal_tracks_route_direction_and_segment_boundary() {
        let size = Size {
            width: 20.0,
            height: 10.0,
        };
        assert_eq!(
            edge_label_top_left(
                &[Point { x: 0.0, y: 0.0 }, Point { x: 0.0, y: 100.0 }],
                LabelPosition::OutsideTopCenter,
                size,
            ),
            Some(Point { x: 6.0, y: 45.0 })
        );
        assert_eq!(
            edge_label_top_left(
                &[Point { x: 0.0, y: 100.0 }, Point { x: 0.0, y: 0.0 }],
                LabelPosition::OutsideTopCenter,
                size,
            ),
            Some(Point { x: -26.0, y: 45.0 })
        );
        assert_eq!(
            edge_label_top_left(
                &[Point { x: 0.0, y: 0.0 }, Point { x: 3.0, y: 4.0 }],
                LabelPosition::OutsideTopCenter,
                size,
            ),
            Some(Point { x: 4.0, y: -10.0 })
        );
        // At an exact bend, GetPointAtDistance selects the preceding segment.
        assert_eq!(
            edge_label_top_left(
                &[
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 100.0, y: 0.0 },
                    Point { x: 100.0, y: 100.0 },
                ],
                LabelPosition::OutsideBottomCenter,
                size,
            ),
            Some(Point { x: 90.0, y: 6.0 })
        );
    }
}
