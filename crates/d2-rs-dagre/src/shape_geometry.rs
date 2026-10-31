// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Derived from D2 v0.9.0 lib/shape/shape.go and lib/geo/{ellipse,point,vector}.go.
// Copyright 2022 Terrastruct Inc.

//! Final circle/oval endpoint tracing, after D2's rectangular route chop.

use d2_rs_core::{Point, Rect, ShapeKind};

/// `Point.DistanceToLine` measures the finite segment, including its ends.
#[cfg(test)]
fn distance_to_segment(point: Point, start: Point, end: Point) -> f64 {
    let a = point.x - start.x;
    let b = point.y - start.y;
    let c = end.x - start.x;
    let d = end.y - start.y;
    let dot = a * c + b * d;
    let length_squared = c * c + d * d;
    let parameter = if length_squared != 0.0 {
        dot / length_squared
    } else {
        -1.0
    };
    let closest = if parameter < 0.0 {
        start
    } else if parameter > 1.0 {
        end
    } else {
        Point {
            x: start.x + parameter * c,
            y: start.y + parameter * d,
        }
    };
    let dx = point.x - closest.x;
    let dy = point.y - closest.y;
    (dx * dx + dy * dy).sqrt()
}

/// D2 `geo.Ellipse.Intersections`, retaining root order and finite-segment
/// filtering. Circle and oval perimeters use the same independent half-axes.
#[cfg(test)]
fn ellipse_intersections(rect: Rect, start: Point, end: Point) -> Vec<Point> {
    let center = rect.center();
    let a = rect.size.width / 2.0;
    let b = rect.size.height / 2.0;
    let a2 = a * a;
    let b2 = b * b;
    if a <= 0.0 || b <= 0.0 {
        return Vec::new();
    }
    let start = Point {
        x: start.x - center.x,
        y: start.y - center.y,
    };
    let end = Point {
        x: end.x - center.x,
        y: end.y - center.y,
    };
    let mut intersections = Vec::new();
    if start.x == end.x {
        let first = Point {
            x: start.x,
            y: b * (a2 - start.x * start.x).sqrt() / a,
        };
        let second = Point {
            x: first.x,
            y: -first.y,
        };
        let on_segment = |point: Point| {
            // The three-value insertion sort mirrors Go sort.Slice's `<`
            // comparator, including non-intersection NaNs from the square root.
            let mut ys = [point.y, start.y, end.y];
            for index in 1..ys.len() {
                let mut current = index;
                while current > 0 && ys[current] < ys[current - 1] {
                    ys.swap(current, current - 1);
                    current -= 1;
                }
            }
            ys[1] == point.y
        };
        if on_segment(first) {
            intersections.push(first);
        }
        if second.y != 0.0 && on_segment(second) {
            intersections.push(second);
        }
    } else {
        let slope = (end.y - start.y) / (end.x - start.x);
        let intercept = start.y - slope * start.x;
        let denominator = a2 * slope * slope + b2;
        let root = (a2 * b2 * (denominator - intercept * intercept)).sqrt();
        let first = Point {
            x: (-slope * intercept * a2 + root) / denominator,
            y: (intercept * b2 + slope * root) / denominator,
        };
        let second = Point {
            x: (-slope * intercept * a2 - root) / denominator,
            y: (intercept * b2 - slope * root) / denominator,
        };
        // PrecisionCompare(distance, 0, geo.PRECISION) == 0.
        if distance_to_segment(first, start, end).abs() < 0.0001 {
            intersections.push(first);
        }
        if first != second && distance_to_segment(second, start, end).abs() < 0.0001 {
            intersections.push(second);
        }
    }
    for point in &mut intersections {
        point.x += center.x;
        point.y += center.y;
    }
    intersections
}

/// D2 `shape.TraceToShapeBorder` for circle and oval perimeters. The caller
/// first intersects the final route segment with the object's rectangle and
/// skips shape tracing when an outside label or icon already clipped it.
pub(super) fn trace_to_shape_border(
    shape: ShapeKind,
    rect: Rect,
    rect_border_point: Point,
    previous_point: Point,
) -> Point {
    d2_rs_core::shape_geometry::trace_shape_border(shape, rect, rect_border_point, previous_point)
}

#[cfg(test)]
mod tests {
    use super::{ellipse_intersections, trace_to_shape_border};
    use d2_rs_core::{Point, Rect, ShapeKind, Size};

    fn ellipse(center: Point, rx: f64, ry: f64) -> Rect {
        Rect {
            origin: Point {
                x: center.x - rx,
                y: center.y - ry,
            },
            size: Size {
                width: rx * 2.0,
                height: ry * 2.0,
            },
        }
    }

    #[test]
    fn upstream_ellipse_intersections_keep_root_order_and_segment_bounds() {
        // D2 lib/geo/ellipse_test.go: vertical crossing and short segments.
        let rect = ellipse(Point::default(), 11.0, 11.0);
        assert_eq!(
            ellipse_intersections(rect, Point { x: 0.0, y: 20.0 }, Point { x: 0.0, y: -20.0 }),
            [Point { x: 0.0, y: 11.0 }, Point { x: 0.0, y: -11.0 }]
        );
        assert!(
            ellipse_intersections(rect, Point { x: 0.0, y: 2.0 }, Point { x: 0.0, y: -2.0 })
                .is_empty()
        );
        assert!(
            ellipse_intersections(rect, Point { x: 2.0, y: 2.0 }, Point { x: 5.0, y: 5.0 })
                .is_empty()
        );
        // Go's untyped math.Sqrt2/2*11 constant rounds once to binary64.
        // Computing the same expression with Rust f64 intermediates is one
        // ULP higher. Pinned Go 1.27: 7.7781745930520225.
        let point = f64::from_bits(0x401f_1cd9_ccee_f239);
        assert_eq!(
            ellipse_intersections(rect, Point { x: 2.0, y: 2.0 }, Point { x: 50.0, y: 50.0 }),
            [Point { x: point, y: point }]
        );
    }

    #[test]
    fn upstream_translated_ellipse_tangents_do_not_duplicate_roots() {
        let rect = ellipse(Point { x: 100.0, y: 200.0 }, 21.0, 21.0);
        assert_eq!(
            ellipse_intersections(
                rect,
                Point { x: 0.0, y: 221.0 },
                Point { x: 200.0, y: 221.0 }
            ),
            [Point { x: 100.0, y: 221.0 }]
        );
        assert_eq!(
            ellipse_intersections(
                rect,
                Point { x: 121.0, y: 100.0 },
                Point { x: 121.0, y: 300.0 }
            ),
            [Point { x: 121.0, y: 200.0 }]
        );
        assert!(
            ellipse_intersections(
                rect,
                Point { x: 122.0, y: 100.0 },
                Point { x: 122.0, y: 300.0 }
            )
            .is_empty()
        );
    }

    #[test]
    fn shape_trace_chooses_the_near_perimeter_and_preserves_oval_axes() {
        let rect = ellipse(Point::default(), 100.0, 50.0);
        assert_eq!(
            trace_to_shape_border(
                ShapeKind::Oval,
                rect,
                Point { x: 100.0, y: 50.0 },
                Point { x: 200.0, y: 100.0 }
            ),
            Point { x: 71.0, y: 35.0 }
        );
        let circle = ellipse(Point::default(), 100.0, 100.0);
        assert_eq!(
            trace_to_shape_border(
                ShapeKind::Circle,
                circle,
                Point {
                    x: -100.0,
                    y: -100.0
                },
                Point {
                    x: -200.0,
                    y: -200.0
                }
            ),
            Point { x: -71.0, y: -71.0 }
        );
    }

    #[test]
    fn shape_trace_rounds_after_float32_truncation() {
        let rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 100.999998,
                height: 80.0,
            },
        };
        let border = Point {
            x: 50.499999,
            y: 0.0,
        };
        assert_eq!(
            trace_to_shape_border(
                ShapeKind::Oval,
                rect,
                border,
                Point {
                    x: border.x,
                    y: -100.0
                }
            ),
            Point { x: 51.0, y: 0.0 }
        );
        assert_eq!(
            trace_to_shape_border(ShapeKind::Rectangle, rect, border, Point::default()),
            border
        );
    }

    #[test]
    fn sequence_circle_endpoints_match_pinned_go_final_trace() {
        // Captured fit→final endpoints from the pinned v0.9.0 oracle for
        // sequence-trace-order.d2. The second target is a compound, but the
        // source circle still receives its ordinary rectangle then shape pass.
        let rect = Rect {
            origin: Point { x: 745.0, y: 100.0 },
            size: Size {
                width: 131.0,
                height: 131.0,
            },
        };
        for (fit, neighbor, expected) in [
            (
                Point {
                    x: 876.0,
                    y: 181.50264410364886,
                },
                Point {
                    x: 1283.25,
                    y: 281.0,
                },
                Point { x: 874.0, y: 181.0 },
            ),
            (
                Point {
                    x: 745.0,
                    y: 194.21062618595826,
                },
                Point { x: 547.0, y: 281.0 },
                Point { x: 750.0, y: 192.0 },
            ),
        ] {
            let rectangle_point = super::super::go_box_intersections(rect, neighbor, fit)[0];
            let actual = trace_to_shape_border(ShapeKind::Circle, rect, rectangle_point, neighbor);
            assert_eq!(actual.x.to_bits(), expected.x.to_bits());
            assert_eq!(actual.y.to_bits(), expected.y.to_bits());
        }
    }
}
