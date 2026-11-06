// SPDX-License-Identifier: MPL-2.0
//! Retain LayoutNested/grid/sequence's synthetic root outside obstacle boxes.
use d2_rs_core::{Graph, LabelPosition, Node, Point, Size, SyntheticRootState};

pub(super) fn grid_root(
    graph: &Graph,
    content: Size,
    horizontal: f64,
    vertical: f64,
) -> (SyntheticRootState, Point) {
    let attrs = graph.source_root.as_ref().and_then(|v| v.get("attributes"));
    let label = attrs
        .and_then(|v| v["label"]["value"].as_str())
        .is_some_and(|s| !s.is_empty());
    let icon = attrs
        .and_then(|v| v.get("icon"))
        .is_some_and(|v| !v.is_null());
    // Root label dimensions are zero: SetDimensions only measures Objects.
    // Spacing still reserves MAX_ICON_SIZE + 2*label.PADDING for an icon.
    let mut left = horizontal;
    let mut right = horizontal;
    let top = vertical.max(if icon { 74.0 } else { 0.0 });
    if icon && label {
        let side = 74.0_f64.max((158.0 - content.width) / 2.0);
        left = left.max(side);
        right = right.max(side);
    }
    let desired = Size {
        width: content.width + left + right,
        height: content.height + top + vertical,
    };
    let mut node = Node::new("", Size::default());
    if let Some(attrs) = attrs {
        if let Ok(shape) = serde_json::from_value(attrs["shape"]["value"].clone()) {
            node.shape = shape;
        }
        let width = attrs["width"]["value"]
            .as_str()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        let height = attrs["height"]["value"]
            .as_str()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        node.fixed_width = width > 0.0;
        node.fixed_height = height > 0.0;
        if node.fixed_width || node.fixed_height {
            node.declared_size = Some(Size { width, height });
        }
    }
    let size = super::grid::grid_shape_size(&node, desired, Size::default());
    let shift = super::grid::grid_shape_shift(node.shape, size, desired);
    (
        SyntheticRootState {
            top_left: Some(Point::default()),
            size,
            label_position: label.then_some(LabelPosition::InsideTopCenter),
            icon_position: icon.then_some(LabelPosition::InsideTopLeft),
        },
        Point {
            x: left - horizontal + shift.x,
            y: top - vertical + shift.y,
        },
    )
}
