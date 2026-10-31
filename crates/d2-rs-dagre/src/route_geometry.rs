// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Derived from Dagro v0.2.1 / Dagre, distributed under the MIT License.
// Copyright (c) 2012-2014 Chris Pettitt
// Copyright (c) 2026 Dagro contributors
// The applicable MIT permission notice is in ../THIRD_PARTY_NOTICES.md.
// D2 route label positioning: Copyright 2022 Terrastruct Inc., MPL-2.0.

//! Raw Dagro intersections and D2's route label geometry for container fitting.

use d2_rs_core::{LabelPosition, Point, Size};

fn js_truthy_number(value: f64) -> bool {
    value != 0.0 && !value.is_nan()
}

/// Dagro v0.2.1 `util.go:intersectRect`, preserving its arithmetic order.
///
/// The center and dimensions are Dagro's raw node attributes. Reconstructing
/// the center from a rounded public box changes endpoint geometry. `None`
/// represents upstream's "not possible to find intersection inside of the
/// rectangle" error, including its JavaScript-compatible NaN truthiness.
pub(super) fn dagro_intersect_rect(center: Point, size: Size, toward: Point) -> Option<Point> {
    let dx = toward.x - center.x;
    let dy = toward.y - center.y;
    let mut w = size.width / 2.0;
    let mut h = size.height / 2.0;
    if !js_truthy_number(dx) && !js_truthy_number(dy) {
        return (w == 0.0 || h == 0.0).then_some(center);
    }
    let (sx, sy) = if !js_truthy_number(dx) {
        if dy < 0.0 {
            h = -h;
        }
        (0.0, h)
    } else if !js_truthy_number(dy) {
        if dx < 0.0 {
            w = -w;
        }
        (w, 0.0)
    } else if dy.abs() * w > dx.abs() * h {
        if dy < 0.0 {
            h = -h;
        }
        (h * dx / dy, h)
    } else {
        if dx < 0.0 {
            w = -w;
        }
        (w, w * dy / dx)
    };
    Some(Point {
        x: center.x + sx,
        y: center.y + sy,
    })
}

/// D2 fitPadding uses a fixed 2px stroke and label percentage 0.
pub(super) fn edge_label_top_left(
    route: &[Point],
    position: LabelPosition,
    size: Size,
) -> Option<Point> {
    d2_rs_core::route_label::top_left(route, position, 2.0, 0.0, size)
}

#[cfg(test)]
mod tests {
    use super::dagro_intersect_rect;
    use d2_rs_core::{Point, Size};

    #[test]
    fn intersection_keeps_multiplication_before_division() {
        let center = Point { x: 0.0, y: 0.0 };
        let point = dagro_intersect_rect(
            center,
            Size {
                width: 47.0,
                height: 66.0,
            },
            Point { x: 13.0, y: 49.0 },
        )
        .unwrap();
        assert_eq!(point.x.to_bits(), 0x4021_829c_bc14_e5e1);
        assert_eq!(point.y.to_bits(), 33.0_f64.to_bits());
        // dx * (h / dy), used by a generic ray scale, loses one ULP here.
        assert_ne!(point.x.to_bits(), 0x4021_829c_bc14_e5e0);

        let transposed = dagro_intersect_rect(
            center,
            Size {
                width: 66.0,
                height: 47.0,
            },
            Point { x: 49.0, y: 13.0 },
        )
        .unwrap();
        assert_eq!(transposed.x.to_bits(), point.y.to_bits());
        assert_eq!(transposed.y.to_bits(), point.x.to_bits());
    }

    #[test]
    fn intersection_matches_go_127_raw_dagro_route_bits() {
        let point = dagro_intersect_rect(
            Point { x: 153.0, y: 303.0 },
            Size {
                width: 111.0,
                height: 66.0,
            },
            Point { x: 210.5, y: 396.5 },
        )
        .unwrap();

        // D2 v0.9.0's pinned Go 1.27.0 Dagro emits this as the first raw
        // route point for `Check PIN.Enter PIN -> Check PIN.choice` in
        // `glob_dimensions.d2` (`GO_DAGRO_EDGE_RAW`).
        assert_eq!(point.x.to_bits(), 0x4065_a969_6969_6969);
        assert_eq!(point.y.to_bits(), 0x4075_0000_0000_0000);
    }

    #[test]
    fn degenerate_rectangles_keep_the_upstream_axis_intersections() {
        // Dagro util_test.go:TestIntersectRectKeepsDegenerateRectanglesFinite.
        let center = Point { x: 3.0, y: 4.0 };
        let zero_width = Size {
            width: 0.0,
            height: 10.0,
        };
        assert_eq!(
            dagro_intersect_rect(center, zero_width, center),
            Some(center)
        );
        assert_eq!(
            dagro_intersect_rect(center, zero_width, Point { x: 3.0, y: 20.0 }),
            Some(Point { x: 3.0, y: 9.0 })
        );
        assert_eq!(
            dagro_intersect_rect(
                center,
                Size {
                    width: 10.0,
                    height: 0.0,
                },
                Point { x: -20.0, y: 4.0 },
            ),
            Some(Point { x: -2.0, y: 4.0 })
        );
    }

    #[test]
    fn same_center_errors_without_treating_small_deltas_as_zero() {
        let center = Point { x: 0.0, y: 0.0 };
        let size = Size {
            width: 10.0,
            height: 10.0,
        };
        assert_eq!(dagro_intersect_rect(center, size, center), None);
        assert_eq!(
            dagro_intersect_rect(center, size, Point { x: -0.0, y: -0.0 },),
            None
        );
        assert_eq!(
            dagro_intersect_rect(
                center,
                size,
                Point {
                    x: f64::MIN_POSITIVE,
                    y: 0.0,
                },
            ),
            Some(Point { x: 5.0, y: 0.0 })
        );
    }

    #[test]
    fn nan_deltas_are_falsy_and_infinite_deltas_are_truthy() {
        let center = Point { x: 0.0, y: 0.0 };
        let size = Size {
            width: 10.0,
            height: 10.0,
        };
        let both_nan = Point {
            x: f64::NAN,
            y: f64::NAN,
        };
        assert_eq!(dagro_intersect_rect(center, size, both_nan), None);
        assert_eq!(
            dagro_intersect_rect(
                center,
                Size {
                    width: 0.0,
                    height: 10.0,
                },
                both_nan,
            ),
            Some(center)
        );
        assert_eq!(
            dagro_intersect_rect(
                center,
                size,
                Point {
                    x: f64::NAN,
                    y: -9.0,
                },
            ),
            Some(Point { x: 0.0, y: -5.0 })
        );
        assert_eq!(
            dagro_intersect_rect(
                center,
                size,
                Point {
                    x: -9.0,
                    y: f64::NAN,
                },
            ),
            Some(Point { x: -5.0, y: 0.0 })
        );
        assert_eq!(
            dagro_intersect_rect(
                center,
                size,
                Point {
                    x: f64::INFINITY,
                    y: 1.0,
                },
            ),
            Some(Point { x: 5.0, y: 0.0 })
        );
    }
}
