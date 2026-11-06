// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Arrowhead-label anchor calculations.

use crate::{Point, Size};

/// Computes a stable top-left anchor near a route endpoint.
///
/// D2's renderer owns the final arrowhead geometry.  This source-shaped seam
/// keeps TALA's bounds code independent of renderer internals and preserves
/// the integer truncation of measured label dimensions at the boundary.
pub(crate) fn arrowhead_top_left(
    route: &[Point],
    is_target: bool,
    width: f64,
    height: f64,
) -> Option<Point> {
    let endpoint = if is_target {
        route.last().copied()?
    } else {
        route.first().copied()?
    };
    let _label_size = Size {
        width: width.trunc(),
        height: height.trunc(),
    };
    Some(Point {
        x: endpoint.x - width.trunc() / 2.0,
        y: endpoint.y - height.trunc() / 2.0,
    })
}

#[cfg(test)]
mod tests {
    use super::arrowhead_top_left;
    use crate::Point;

    #[test]
    fn uses_the_selected_endpoint_and_integer_dimensions() {
        let route = [Point { x: 0.0, y: 10.0 }, Point { x: 100.0, y: 40.0 }];
        assert_eq!(
            arrowhead_top_left(&route, false, 21.9, 11.9),
            Some(Point { x: -10.5, y: 4.5 })
        );
        assert_eq!(
            arrowhead_top_left(&route, true, 21.9, 11.9),
            Some(Point { x: 89.5, y: 34.5 })
        );
        assert_eq!(arrowhead_top_left(&[], true, 10.0, 10.0), None);
    }
}
