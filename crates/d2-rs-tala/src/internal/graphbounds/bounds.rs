// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bounds, shape modifiers, and layout extents.
//!
//! These helpers keep 3D/multiple-shape protrusions and aggregate geometry
//! consistent across fitting, packing, routing, and normalization.

use super::owner_median::source_math_max as source_max;
use super::owner_sizeless_geometry::source_min;
use super::*;

/// Source `Edge.boundingBoxValues`: route and positioned labels, with each
/// edge corner rounded before it contributes to any graph envelope.
pub(super) fn routed_edge_bounds(edge: &ArenaEdge) -> Option<(Point, Point)> {
    if edge.route.is_empty() {
        return None;
    }
    let mut top_left = Point {
        x: f64::INFINITY,
        y: f64::INFINITY,
    };
    let mut bottom_right = Point {
        x: f64::NEG_INFINITY,
        y: f64::NEG_INFINITY,
    };
    for point in edge.route.read().iter().copied() {
        top_left.x = source_min(top_left.x, point.x);
        top_left.y = source_min(top_left.y, point.y);
        bottom_right.x = source_max(bottom_right.x, point.x);
        bottom_right.y = source_max(bottom_right.y, point.y);
    }

    if let Some(label) = edge.label.as_ref()
        && let Some(label_top_left) = labels::edge_label_top_left(
            edge,
            label.position,
            label.percentage,
            label.size.width,
            label.size.height,
        )
    {
        top_left.x = source_min(top_left.x, label_top_left.x);
        top_left.y = source_min(top_left.y, label_top_left.y);
        bottom_right.x = source_max(bottom_right.x, label_top_left.x + label.size.width);
        bottom_right.y = source_max(bottom_right.y, label_top_left.y + label.size.height);
    }

    for is_target in [false, true] {
        if let Some(label_rect) =
            labels::arrowhead_label_rect_for_route(edge, &edge.route.read(), is_target)
        {
            top_left.x = source_min(top_left.x, label_rect.origin.x);
            top_left.y = source_min(top_left.y, label_rect.origin.y);
            bottom_right.x =
                source_max(bottom_right.x, label_rect.origin.x + label_rect.size.width);
            bottom_right.y =
                source_max(bottom_right.y, label_rect.origin.y + label_rect.size.height);
        }
    }

    // Edge.getBoundingBox rounds each returned corner before Graph folds
    // it into the aggregate bounds.
    Some((
        Point {
            x: top_left.x.round(),
            y: top_left.y.round(),
        },
        Point {
            x: bottom_right.x.round(),
            y: bottom_right.y.round(),
        },
    ))
}

fn outside_side_alignment(
    position: LabelPosition,
) -> Option<(ExternalSide, crate::ExternalAlignment)> {
    use crate::ExternalAlignment::{Center, End, Start};
    Some(match position {
        LabelPosition::OutsideTopLeft => (ExternalSide::Top, Start),
        LabelPosition::OutsideTopCenter => (ExternalSide::Top, Center),
        LabelPosition::OutsideTopRight => (ExternalSide::Top, End),
        LabelPosition::OutsideLeftTop => (ExternalSide::Left, Start),
        LabelPosition::OutsideLeftMiddle => (ExternalSide::Left, Center),
        LabelPosition::OutsideLeftBottom => (ExternalSide::Left, End),
        LabelPosition::OutsideRightTop => (ExternalSide::Right, Start),
        LabelPosition::OutsideRightMiddle => (ExternalSide::Right, Center),
        LabelPosition::OutsideRightBottom => (ExternalSide::Right, End),
        LabelPosition::OutsideBottomLeft => (ExternalSide::Bottom, Start),
        LabelPosition::OutsideBottomCenter => (ExternalSide::Bottom, Center),
        LabelPosition::OutsideBottomRight => (ExternalSide::Bottom, End),
        _ => return None,
    })
}

pub(super) fn external_label_position(label: ExternalLabel) -> LabelPosition {
    use crate::ExternalAlignment::{Center, End, Start};
    use ExternalSide::{Bottom, Left, Right, Top};
    match (label.side, label.alignment) {
        (Top, Start) => LabelPosition::OutsideTopLeft,
        (Top, Center) => LabelPosition::OutsideTopCenter,
        (Top, End) => LabelPosition::OutsideTopRight,
        (Left, Start) => LabelPosition::OutsideLeftTop,
        (Left, Center) => LabelPosition::OutsideLeftMiddle,
        (Left, End) => LabelPosition::OutsideLeftBottom,
        (Right, Start) => LabelPosition::OutsideRightTop,
        (Right, Center) => LabelPosition::OutsideRightMiddle,
        (Right, End) => LabelPosition::OutsideRightBottom,
        (Bottom, Start) => LabelPosition::OutsideBottomLeft,
        (Bottom, Center) => LabelPosition::OutsideBottomCenter,
        (Bottom, End) => LabelPosition::OutsideBottomRight,
    }
}

/// Recovered `Node.getModifierElementAdjustments`.
///
/// Multiple and 3D shapes extend to the right and above their ordinary shape
/// box. Bounds, inside placement, and edge tracing all consume this node-level
/// geometry carrier.
pub(super) fn modifier_element_adjustments(node: &ArenaNode) -> (f64, f64) {
    if node.is_3d {
        (
            15.0,
            if node.shape == ShapeKind::Hexagon {
                7.5
            } else {
                15.0
            },
        )
    } else if node.is_multiple {
        (10.0, 10.0)
    } else {
        (0.0, 0.0)
    }
}

/// Nodes.fixedOrigin: the first least-nested container wins, then the first
/// positioned fixed child belonging directly to that container supplies origin.
/// The caller supplies ancestry in the same view as the observed node boxes.
fn selected_fixed_origin<T, K: Copy + Eq>(
    nodes: &[T],
    ancestry: impl Fn(&T) -> (Option<K>, usize),
    node_origin: impl Fn(&T) -> Option<Point>,
) -> Option<Point> {
    let mut container = None;
    let mut minimum_level = usize::MAX;
    for node in nodes {
        let (parent, level) = ancestry(node);
        if parent.is_none() {
            container = None;
            break;
        }
        if level < minimum_level {
            minimum_level = level;
            container = parent;
        }
    }
    nodes.iter().find_map(|node| {
        if ancestry(node).0 != container {
            return None;
        }
        node_origin(node)
    })
}

impl ArenaGraph {
    /// Source Graph.boundingBox folds Graph.Edges into the fixed node box.
    /// Edges with no points contribute no bounds, even if they carry labels.
    pub(super) fn include_graph_edge_bounds(&self, low: Point, high: Point) -> (Point, Point) {
        self.include_selected_graph_edge_bounds(low, high, &self.edge_order.read())
    }

    pub(super) fn include_selected_graph_edge_bounds(
        &self,
        mut low: Point,
        mut high: Point,
        edges: &[EdgeId],
    ) -> (Point, Point) {
        for edge_id in edges {
            if let Some((edge_low, edge_high)) =
                routed_edge_bounds(&self.edge_records[edge_id.0 as usize])
            {
                if edge_low.x.is_infinite() {
                    continue;
                }
                low.x = source_min(low.x, edge_low.x);
                low.y = source_min(low.y, edge_low.y);
                high.x = source_max(high.x, edge_high.x);
                high.y = source_max(high.y, edge_high.y);
            }
        }
        (low, high)
    }

    /// Return the outside label represented by the node's current placement.
    ///
    /// Automatic labels are initially materialized with a shape-preferred
    /// side (for example, a person starts at the bottom).  `PlaceLabels` can
    /// then select another side, while the adapter's `external_label` remains
    /// the original carrier.  Go's `Node.boundingBoxValues` reads the live
    /// `Label.Position`, so bounds must follow that position after selection.
    pub(super) fn selected_external_label(node: &ArenaNode) -> Option<ExternalLabel> {
        // Node.boundingBoxValues uses the live Label.Position regardless of
        // the adapter's initial spacing policy. An undersized image can have
        // an unreserved carrier and still acquire an outside label in
        // PlaceLabels. Inside labels remain excluded by outside_side_alignment.
        let selected = node.label_size.and_then(|size| {
            outside_side_alignment(node.label_position).map(|(side, alignment)| ExternalLabel {
                size,
                side,
                alignment,
                automatic: false,
                reserve_space: true,
            })
        });
        selected.or_else(|| node.external_label.filter(|label| label.reserve_space))
    }

    /// `Nodes.getFixedBoundingBox` over nodes retained only through a
    /// temporary graph's shared `Containers` map.
    pub(super) fn fixed_external_node_bounds(nodes: &[ArenaNode]) -> Option<(Point, Point)> {
        let (low, high) = Self::fixed_external_node_corners(nodes);
        low.zip(high)
    }

    /// `Nodes.getBoundingBox` over nodes retained only through a temporary
    /// graph's shared `Containers` map. Unlike `getFixedBoundingBox`, this
    /// keeps the bounds' actual top-left even when a node is fixed.
    pub(super) fn external_node_bounds(nodes: &[ArenaNode]) -> Option<(Point, Point)> {
        let boxes = nodes
            .iter()
            .map(|node| Some((node, node.position?)))
            .collect::<Option<Vec<_>>>()?;
        Some(Self::positioned_node_bounds(&boxes, true))
    }

    /// NodeBoundingBox(node, nil): body corner rounding, then modifier and
    /// loop extents. Outside labels/icons belong to the separate contextual
    /// bounds pass and must not affect packing hierarchy intrusion corners.
    pub(super) fn positioned_node_body_bounds(
        node: &ArenaNode,
        position: Point,
        round_dimensions: bool,
    ) -> (Point, Point) {
        let size = node.rect.size;
        let mut top_left = position;
        // Node.boundingBoxValues rounds the body corner before
        // adding modifier/loop extents and testing outside labels.
        // Keeping the raw corner in a max would turn 450.4 into
        // 451 at the later shape fit instead of the source's 450.
        let mut bottom_right = Point {
            x: position.x + size.width,
            y: position.y + size.height,
        };
        if round_dimensions {
            bottom_right.x = bottom_right.x.round();
            bottom_right.y = bottom_right.y.round();
        }
        let (modifier_x, modifier_y) = modifier_element_adjustments(node);
        let [loop_top, loop_right, loop_bottom, loop_left] =
            loop_cardinal_extents(node.loop_offsets.as_ref());
        if modifier_x != 0.0 || modifier_y != 0.0 {
            top_left.y -= modifier_y;
            bottom_right.x += modifier_x;
        }
        top_left.x -= loop_left;
        top_left.y -= loop_top;
        bottom_right.x += loop_right;
        bottom_right.y += loop_bottom;
        (top_left, bottom_right)
    }

    /// Nodes.boundingBox, shared by rounded/unrounded arena and projected views.
    /// Every requested node must be positioned before an envelope exists.
    fn positioned_node_bounds(
        boxes: &[(&ArenaNode, Point)],
        round_dimensions: bool,
    ) -> (Point, Point) {
        const LABEL_PADDING: f64 = 5.0;
        if boxes.is_empty() {
            return (
                Point {
                    x: f64::NEG_INFINITY,
                    y: f64::NEG_INFINITY,
                },
                Point {
                    x: f64::INFINITY,
                    y: f64::INFINITY,
                },
            );
        }
        boxes
            .iter()
            .copied()
            .map(|(node, position)| {
                let size = node.rect.size;
                let (mut top_left, mut bottom_right) =
                    Self::positioned_node_body_bounds(node, position, round_dimensions);
                if let Some(label) = Self::selected_external_label(node) {
                    let leftmost = boxes.iter().all(|(_, other)| other.x >= position.x);
                    let topmost = boxes.iter().all(|(_, other)| other.y >= position.y);
                    let right = position.x + size.width;
                    let bottom = position.y + size.height;
                    let rightmost = boxes.iter().all(|(other, other_position)| {
                        other_position.x + other.rect.size.width <= right
                    });
                    let bottommost = boxes.iter().all(|(other, other_position)| {
                        other_position.y + other.rect.size.height <= bottom
                    });
                    let label_top_left = labels::point_on_box(
                        external_label_position(label),
                        Rect {
                            origin: position,
                            size,
                        },
                        label.size,
                    );
                    if label_top_left.x < top_left.x {
                        top_left.x = (label_top_left.x
                            - LABEL_PADDING * if leftmost { 1.0 } else { 2.0 })
                        .floor();
                    }
                    if label_top_left.y < top_left.y {
                        top_left.y = (label_top_left.y
                            - LABEL_PADDING * if topmost { 1.0 } else { 2.0 })
                        .floor();
                    }
                    if label_top_left.x > bottom_right.x {
                        bottom_right.x = (label_top_left.x
                            + label.size.width
                            + LABEL_PADDING * if rightmost { 1.0 } else { 2.0 })
                        .ceil();
                    }
                    if label_top_left.y > bottom_right.y {
                        bottom_right.y = (label_top_left.y
                            + label.size.height
                            + LABEL_PADDING * if bottommost { 1.0 } else { 2.0 })
                        .ceil();
                    }
                }
                if let Some((side, alignment)) = (node.shape != ShapeKind::Image)
                    .then_some(node.icon_position)
                    .flatten()
                    .and_then(Self::outside_side_alignment)
                {
                    let icon = ExternalLabel {
                        size: Size {
                            width: 64.0,
                            height: 64.0,
                        },
                        side,
                        alignment,
                        automatic: false,
                        reserve_space: true,
                    };
                    let icon_top_left = labels::point_on_box(
                        external_label_position(icon),
                        Rect {
                            origin: position,
                            size,
                        },
                        icon.size,
                    );
                    let outside_padding = 2.0 * LABEL_PADDING;
                    top_left.x = top_left.x.min((icon_top_left.x - outside_padding).floor());
                    top_left.y = top_left.y.min((icon_top_left.y - outside_padding).floor());
                    bottom_right.x = bottom_right
                        .x
                        .max((icon_top_left.x + icon.size.width + outside_padding).ceil());
                    bottom_right.y = bottom_right
                        .y
                        .max((icon_top_left.y + icon.size.height + outside_padding).ceil());
                }
                (top_left, bottom_right)
            })
            .fold(
                (
                    Point {
                        x: f64::INFINITY,
                        y: f64::INFINITY,
                    },
                    Point {
                        x: f64::NEG_INFINITY,
                        y: f64::NEG_INFINITY,
                    },
                ),
                |(low, high), (node_low, node_high)| {
                    (
                        Point {
                            x: routing::channel_solver::minimum(low.x, node_low.x),
                            y: routing::channel_solver::minimum(low.y, node_low.y),
                        },
                        Point {
                            x: routing::channel_solver::maximum(high.x, node_high.x),
                            y: routing::channel_solver::maximum(high.y, node_high.y),
                        },
                    )
                },
            )
    }

    /// Projected views carry the source ancestry captured with their geometry.
    /// Keep the two source corner pointers independent until a caller requires
    /// a complete rectangle: fixedOrigin still runs after a nil body bound.
    pub(super) fn fixed_external_node_corners(
        nodes: &[ArenaNode],
    ) -> (Option<Point>, Option<Point>) {
        let (mut low, high) = Self::external_node_bounds(nodes)
            .map_or((None, None), |(low, high)| (Some(low), Some(high)));
        let origin = selected_fixed_origin(
            nodes,
            |node| {
                (
                    node.scoring_container_parent,
                    node.scoring_container_ancestors.len(),
                )
            },
            |node| Self::node_origin_for_geometry(node, node.position),
        );
        if origin.is_some() {
            low = origin;
        }
        (low, high)
    }

    /// Bounds of the positioned node rectangles only. This is deliberately
    /// distinct from both label-aware and render-aware fixed bounds.
    pub(super) fn plain_node_bounds(&self, nodes: &[NodeId]) -> Option<(Point, Point)> {
        nodes
            .iter()
            .copied()
            .filter_map(|node| {
                let position = self.position(node)?;
                let size = self.nodes[node.0 as usize].rect.size;
                Some((
                    position,
                    Point {
                        x: position.x + size.width,
                        y: position.y + size.height,
                    },
                ))
            })
            .fold(None, |bounds, (top_left, bottom_right)| {
                Some(match bounds {
                    None => (top_left, bottom_right),
                    Some((mut low, mut high)) => {
                        low.x = low.x.min(top_left.x);
                        low.y = low.y.min(top_left.y);
                        high.x = high.x.max(bottom_right.x);
                        high.y = high.y.max(bottom_right.y);
                        (low, high)
                    }
                })
            })
    }

    /// Recovered rectangle bounds plus reserved outside labels/icons.
    /// `fixed_node_bounds` extends this layer with modifier and self-loop
    /// geometry.
    pub(super) fn external_label_node_bounds(&self, nodes: &[NodeId]) -> Option<(Point, Point)> {
        const LABEL_PADDING: f64 = 5.0;
        let boxes = nodes
            .iter()
            .copied()
            .filter_map(|node| {
                let position = self.position(node)?;
                let size = self.nodes[node.0 as usize].rect.size;
                Some((node, position, size))
            })
            .collect::<Vec<_>>();

        boxes
            .iter()
            .copied()
            .map(|(node, position, size)| {
                let mut top_left = position;
                let mut bottom_right = Point {
                    x: position.x + size.width,
                    y: position.y + size.height,
                };
                if let Some(label) = Self::selected_external_label(&self.nodes[node.0 as usize]) {
                    // Recovered Node.getBoundingBox first positions an outside
                    // label LABEL_PADDING away from its box, then adds one
                    // LABEL_PADDING at the boundary or two when another node
                    // reaches farther in that direction.
                    let leftmost = boxes.iter().all(|(_, other, _)| other.x >= position.x);
                    let topmost = boxes.iter().all(|(_, other, _)| other.y >= position.y);
                    let right = position.x + size.width;
                    let bottom = position.y + size.height;
                    let rightmost = boxes
                        .iter()
                        .all(|(_, other, other_size)| other.x + other_size.width <= right);
                    let bottommost = boxes
                        .iter()
                        .all(|(_, other, other_size)| other.y + other_size.height <= bottom);
                    let label_top_left = label.top_left(
                        Rect {
                            origin: position,
                            size,
                        },
                        LABEL_PADDING,
                    );

                    if label_top_left.x < top_left.x {
                        let outside_padding = LABEL_PADDING * if leftmost { 1.0 } else { 2.0 };
                        top_left.x = (label_top_left.x - outside_padding).floor();
                    }
                    if label_top_left.y < top_left.y {
                        let outside_padding = LABEL_PADDING * if topmost { 1.0 } else { 2.0 };
                        top_left.y = (label_top_left.y - outside_padding).floor();
                    }
                    if label_top_left.x > bottom_right.x {
                        let outside_padding = LABEL_PADDING * if rightmost { 1.0 } else { 2.0 };
                        bottom_right.x =
                            (label_top_left.x + label.size.width + outside_padding).ceil();
                    }
                    if label_top_left.y > bottom_right.y {
                        let outside_padding = LABEL_PADDING * if bottommost { 1.0 } else { 2.0 };
                        bottom_right.y =
                            (label_top_left.y + label.size.height + outside_padding).ceil();
                    }
                }
                if let Some((side, alignment)) = (self.nodes[node.0 as usize].shape
                    != ShapeKind::Image)
                    .then_some(self.nodes[node.0 as usize].icon_position)
                    .flatten()
                    .and_then(Self::outside_side_alignment)
                {
                    let icon = ExternalLabel {
                        size: Size {
                            width: 64.0,
                            height: 64.0,
                        },
                        side,
                        alignment,
                        automatic: false,
                        reserve_space: true,
                    };
                    let icon_top_left = icon.top_left(
                        Rect {
                            origin: position,
                            size,
                        },
                        LABEL_PADDING,
                    );
                    let outside_padding = 2.0 * LABEL_PADDING;
                    top_left.x = top_left.x.min((icon_top_left.x - outside_padding).floor());
                    top_left.y = top_left.y.min((icon_top_left.y - outside_padding).floor());
                    bottom_right.x = bottom_right
                        .x
                        .max((icon_top_left.x + icon.size.width + outside_padding).ceil());
                    bottom_right.y = bottom_right
                        .y
                        .max((icon_top_left.y + icon.size.height + outside_padding).ceil());
                }
                (top_left, bottom_right)
            })
            .fold(None, |bounds, (top_left, bottom_right)| {
                Some(match bounds {
                    None => (top_left, bottom_right),
                    Some((mut low, mut high)) => {
                        low.x = low.x.min(top_left.x);
                        low.y = low.y.min(top_left.y);
                        high.x = high.x.max(bottom_right.x);
                        high.y = high.y.max(bottom_right.y);
                        (low, high)
                    }
                })
            })
    }

    /// Recovered `Nodes.getFixedBoundingBox` surface. The node-level bounds
    /// include reserved labels/icons, multiple/3D render modifiers, and
    /// per-side self-loop extents.
    ///
    /// This is the ordinary `Nodes.BoundingBox` calculation.  It deliberately
    /// leaves the computed top-left intact; `FixedBoundingBox` applies the
    /// separate fixed-origin override after calculating the same content
    /// bounds.
    pub(super) fn ordinary_node_bounds(&self, nodes: &[NodeId]) -> Option<(Point, Point)> {
        let boxes = nodes
            .iter()
            .map(|id| Some((&self.nodes[id.0 as usize], self.position(*id)?)))
            .collect::<Option<Vec<_>>>()?;
        Some(Self::positioned_node_bounds(&boxes, true))
    }

    /// Source `Graph.Area` for a retained placement component. The sorting
    /// key keeps fractional node bounds while routed edges keep their own
    /// rounded corners; rounded placement envelopes are a separate contract.
    pub(super) fn component_graph_area(&self, component: &[NodeId]) -> f64 {
        if component.is_empty() {
            return 0.0;
        }
        let Some((mut top_left, mut bottom_right)) = self.unrounded_fixed_node_bounds(component)
        else {
            return 0.0;
        };
        let members = component.iter().copied().collect::<BTreeSet<_>>();
        for edge_id in &self.edge_order.read() {
            let edge = &self.edge_records[edge_id.0 as usize];
            if !members.contains(&edge.from) || !members.contains(&edge.to) {
                continue;
            }
            if let Some((edge_top_left, edge_bottom_right)) = routed_edge_bounds(edge) {
                if edge_top_left.x.is_infinite() {
                    continue;
                }
                top_left.x = source_min(top_left.x, edge_top_left.x);
                top_left.y = source_min(top_left.y, edge_top_left.y);
                bottom_right.x = source_max(bottom_right.x, edge_bottom_right.x);
                bottom_right.y = source_max(bottom_right.y, edge_bottom_right.y);
            }
        }
        (top_left.x - bottom_right.x).abs() * (top_left.y - bottom_right.y).abs()
    }

    /// Recovered `Nodes.unroundedFixedBounds` surface used by `Graph.Area`.
    /// Unlike layout bounds, node dimensions and modifier/loop extents remain
    /// fractional here; positioned outside labels and icons retain their
    /// source padding rules.
    pub(super) fn unrounded_fixed_node_bounds(&self, nodes: &[NodeId]) -> Option<(Point, Point)> {
        let (low, high) = self.fixed_node_corners(nodes, false);
        low.zip(high)
    }

    /// Shared Nodes.fixedBounds/unroundedFixedBounds, including source nil
    /// corners. Graph bounds consumers require both corners; direct node
    /// bounds observers can retain an origin alongside an absent far corner.
    pub(super) fn fixed_node_corners(
        &self,
        nodes: &[NodeId],
        round_dimensions: bool,
    ) -> (Option<Point>, Option<Point>) {
        // Only a node with FixedTopLeft has an origin. Without one the
        // selected origin is nil whichever container the pure ancestry
        // walk selects, so that walk is skipped.
        let select_origin = nodes
            .iter()
            .any(|node| self.nodes[node.0 as usize].fixed_top_left.is_some());
        self.fixed_node_corners_with_origin(nodes, round_dimensions, select_origin, |node| {
            Self::node_origin_for_geometry(&self.nodes[node.0 as usize], self.position(node))
        })
    }

    pub(super) fn fixed_node_corners_with_origin(
        &self,
        nodes: &[NodeId],
        round_dimensions: bool,
        select_origin: bool,
        origin_for_node: impl Fn(NodeId) -> Option<Point>,
    ) -> (Option<Point>, Option<Point>) {
        let boxes = nodes
            .iter()
            .map(|id| (&self.nodes[id.0 as usize], self.position(*id)))
            .collect::<Vec<_>>();
        let positioned = boxes
            .iter()
            .map(|(node, position)| Some((*node, (*position)?)))
            .collect::<Option<Vec<_>>>();
        let (mut low, high) = positioned
            .as_ref()
            .map(|boxes| Self::positioned_node_bounds(boxes, round_dimensions))
            .map_or((None, None), |(low, high)| (Some(low), Some(high)));
        if !select_origin {
            return (low, high);
        }
        let origin = selected_fixed_origin(
            nodes,
            |node| self.bounds_container_context(*node),
            |node| origin_for_node(*node),
        );
        if origin.is_some() {
            low = origin;
        }
        (low, high)
    }

    fn node_origin_for_geometry(node: &ArenaNode, position: Option<Point>) -> Option<Point> {
        let position = position?;
        let fixed = node.fixed_top_left?;
        Some(Point {
            x: position.x - fixed.x,
            y: position.y - fixed.y,
        })
    }

    fn bounds_container_context(&self, node: NodeId) -> (Option<NodeId>, usize) {
        let container = self.bounds_owning_container(node);
        let mut current = container;
        let mut level = 0;
        while let Some(parent) = current {
            level += 1;
            current = self.bounds_owning_container(parent);
        }
        (container, level)
    }

    fn bounds_owning_container(&self, node: NodeId) -> Option<NodeId> {
        if self.shared_topology_initialized() {
            match super::owner_placement::owner_owning_container(self, node) {
                Ok(parent) => return parent,
                // Legacy induced geometry can precede allocation of the
                // canonical vessel. Its existing effective-container view
                // supplies ancestry until that binding is materialized.
                Err(super::shared_topology::OwnerTopologyError::UnboundVessel(_)) => {}
                Err(error) => panic!("fixed bounds owning container: {error}"),
            }
        }
        // Decoded active vessels can be concrete Graph.Nodes entries while
        // every member is filtered out of that slice. Numerical carrier
        // activity is not the member's Node.Cluster/Sequence.isActive state.
        // Before canonical pointer bindings exist, follow the separate live
        // vessel's actual Container (node.go::container), in cluster-first order.
        let vessel_ids = self
            .clusters
            .iter()
            .filter(|group| group.members.contains(&node))
            .map(|group| group.vessel_tala_id)
            .chain(
                self.sequences
                    .iter()
                    .filter(|group| group.members.contains(&node))
                    .map(|group| group.vessel_tala_id),
            );
        for vessel_id in vessel_ids {
            if let Some(vessel) = self
                .node_order
                .iter()
                .copied()
                .find(|id| self.nodes[id.0 as usize].tala_id == vessel_id)
            {
                return self.nodes[vessel.0 as usize].container;
            }
        }
        self.active_node_container(node)
    }

    /// Geometry projections of known nodes must read ancestry from their
    /// owning arena. In particular, an active cluster's vessel can belong to
    /// a different container than the member's earlier captured ancestry.
    /// Explicit NodeIds carry identity; entity IDs and dense snapshot order do
    /// not establish an owning-node correspondence.
    pub(super) fn fixed_projected_node_corners(
        &self,
        nodes: &[(NodeId, &ArenaNode)],
    ) -> (Option<Point>, Option<Point>) {
        let boxes = nodes
            .iter()
            .map(|(_, node)| Some((*node, node.position?)))
            .collect::<Option<Vec<_>>>();
        let (mut low, high) = boxes
            .as_ref()
            .map(|boxes| Self::positioned_node_bounds(boxes, true))
            .map_or((None, None), |(low, high)| (Some(low), Some(high)));
        let origin = selected_fixed_origin(
            nodes,
            |(id, _)| self.bounds_container_context(*id),
            |(_, node)| Self::node_origin_for_geometry(node, node.position),
        );
        if origin.is_some() {
            low = origin;
        }
        (low, high)
    }

    /// Source Nodes.fixedBounds with a complete rectangle required by the caller.
    pub(super) fn fixed_node_bounds(&self, nodes: &[NodeId]) -> Option<(Point, Point)> {
        let (low, high) = self.fixed_node_corners(nodes, true);
        low.zip(high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_origin_reads_the_bound_groups_live_vessel_container() {
        use super::super::owner_groups::OwnerGroupKind;
        use super::super::shared_topology::OwnerGroupFieldView;
        let mut input = Graph::default();
        for (name, parent) in [
            ("a", None),
            ("b", None),
            ("member", Some(NodeId(0))),
            ("peer", Some(NodeId(1))),
            ("vessel", Some(NodeId(1))),
        ] {
            let mut node = crate::Node::new(
                name,
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            );
            node.parent = parent;
            input.add_node(node);
        }
        let mut owner = ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        let group = owner.new_owner_group(OwnerGroupKind::Cluster);
        let nodes = owner
            .allocate_owner_member_header(&[Some(NodeId(2))], 1)
            .unwrap();
        owner
            .set_owner_group_fields(
                group,
                OwnerGroupFieldView {
                    vessel: Some(NodeId(4)),
                    nodes,
                    graph: Some(OwnerGraphId(0)),
                    container: Some(NodeId(1)),
                },
            )
            .unwrap();
        owner
            .set_owner_node_group(NodeId(2), OwnerGroupKind::Cluster, Some(group))
            .unwrap();
        owner.set_position(NodeId(2), Point { x: 200.0, y: 300.0 });
        owner.nodes[2].fixed_top_left = Some(Point { x: 20.0, y: 30.0 });
        owner.set_position(NodeId(3), Point { x: 100.0, y: 100.0 });
        assert_eq!(
            owner.fixed_node_corners(&[NodeId(3), NodeId(2)], true).0,
            Some(Point { x: 180.0, y: 270.0 })
        );
        // Node.Graph controls activity; clearing it exposes the raw member
        // container and the first peer's different parent wins the depth tie.
        owner
            .set_owner_node_graph_reference(NodeId(4), None)
            .unwrap();
        assert_eq!(
            owner.fixed_node_corners(&[NodeId(3), NodeId(2)], true).0,
            Some(Point { x: 100.0, y: 100.0 })
        );
    }

    #[test]
    fn fixed_origin_chooses_shallowest_parent_before_first_fixed_node() {
        let mut input = Graph::default();
        for (name, parent) in [
            ("outer", None),
            ("nested", Some(NodeId(0))),
            ("deep", Some(NodeId(1))),
            ("shallow", Some(NodeId(0))),
        ] {
            let mut node = crate::Node::new(
                name,
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            );
            node.parent = parent;
            input.add_node(node);
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(NodeId(2), Point { x: 200.0, y: 300.0 });
        graph.nodes[2].fixed_top_left = Some(Point { x: 20.0, y: 30.0 });
        graph.set_position(NodeId(3), Point { x: 500.0, y: 600.0 });
        graph.nodes[3].fixed_top_left = Some(Point { x: 40.0, y: 50.0 });
        let expected = (
            Some(Point { x: 460.0, y: 550.0 }),
            Some(Point { x: 540.0, y: 630.0 }),
        );
        let selection = [NodeId(2), NodeId(3)];
        for round in [false, true] {
            assert_eq!(graph.fixed_node_corners(&selection, round), expected);
        }
        let projected = [graph.nodes[2].clone(), graph.nodes[3].clone()];
        assert_eq!(
            ArenaGraph::fixed_external_node_corners(&projected),
            expected
        );
        let mut changed = projected.clone();
        changed[0].scoring_container_parent = None;
        changed[0].scoring_container_ancestors.clear();
        changed[1].position.as_mut().unwrap().x += 100.0;
        let identities = [(NodeId(2), &changed[0]), (NodeId(3), &changed[1])];
        assert_eq!(
            graph.fixed_projected_node_corners(&identities),
            (
                Some(Point { x: 560.0, y: 550.0 }),
                Some(Point { x: 640.0, y: 630.0 })
            )
        );
    }

    #[test]
    fn fixed_origin_keeps_first_parent_on_depth_ties_and_uses_node_identity() {
        let mut input = Graph::default();
        for (name, parent) in [
            ("a", None),
            ("b", None),
            ("child-a", Some(NodeId(0))),
            ("child-b", Some(NodeId(1))),
        ] {
            let mut node = crate::Node::new(
                name,
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            );
            node.parent = parent;
            input.add_node(node);
        }
        let mut graph = ArenaGraph::from_input(&input);
        // Entity IDs are not parent pointer identity in the live arena.
        graph.nodes[1].tala_id = graph.nodes[0].tala_id;
        graph.set_position(NodeId(2), Point { x: 200.0, y: 300.0 });
        graph.set_position(NodeId(3), Point { x: 500.0, y: 600.0 });
        graph.nodes[3].fixed_top_left = Some(Point { x: 20.0, y: 30.0 });
        assert_eq!(
            graph.fixed_node_corners(&[NodeId(2), NodeId(3)], true).0,
            Some(Point { x: 200.0, y: 300.0 })
        );
        assert_eq!(
            graph.fixed_node_corners(&[NodeId(3), NodeId(2)], true).0,
            Some(Point { x: 480.0, y: 570.0 })
        );
    }

    #[test]
    fn fixed_origin_survives_a_missing_other_corner() {
        let mut input = Graph::default();
        for name in ["unpositioned", "fixed"] {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 40.0,
                    height: 30.0,
                },
            ));
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(NodeId(1), Point { x: 200.0, y: 300.0 });
        graph.nodes[1].fixed_top_left = Some(Point { x: 20.0, y: 30.0 });
        let expected = (Some(Point { x: 180.0, y: 270.0 }), None);
        for round in [false, true] {
            assert_eq!(
                graph.fixed_node_corners(&[NodeId(0), NodeId(1)], round),
                expected
            );
        }
        assert_eq!(
            ArenaGraph::fixed_external_node_corners(&graph.nodes),
            expected
        );
        // Graph bounds require both corners, as in source Graph.boundingBox.
        assert_eq!(graph.fixed_node_bounds(&[NodeId(0), NodeId(1)]), None);
    }

    #[test]
    fn node_bounds_preserve_empty_and_unpositioned_source_results() {
        let mut input = Graph::default();
        input.add_node(crate::Node::new(
            "a",
            Size {
                width: 100.0,
                height: 80.0,
            },
        ));
        input.add_node(crate::Node::new(
            "b",
            Size {
                width: 100.0,
                height: 80.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(NodeId(0), Point::default());
        let ids = [NodeId(0), NodeId(1)];
        assert_eq!(graph.ordinary_node_bounds(&ids), None);
        assert_eq!(graph.unrounded_fixed_node_bounds(&ids), None);
        assert_eq!(ArenaGraph::external_node_bounds(&graph.nodes), None);
        let empty = Some((
            Point {
                x: f64::NEG_INFINITY,
                y: f64::NEG_INFINITY,
            },
            Point {
                x: f64::INFINITY,
                y: f64::INFINITY,
            },
        ));
        assert_eq!(graph.ordinary_node_bounds(&[]), empty);
        assert_eq!(graph.unrounded_fixed_node_bounds(&[]), empty);
        assert_eq!(ArenaGraph::external_node_bounds(&[]), empty);
    }

    #[test]
    fn node_bounds_keep_modifier_and_loop_operations_separate() {
        let mut input = Graph::default();
        input.add_node(crate::Node::new(
            "a",
            Size {
                width: 100.0,
                height: 80.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(NodeId(0), Point { x: 0.1, y: 0.7 });
        graph.nodes[0].is_multiple = true;
        graph.nodes[0].loop_offsets = Some([(4, -2.25)].into_iter().collect());
        let (low, high) = graph.ordinary_node_bounds(&[NodeId(0)]).unwrap();
        // Pristine Node.boundingBoxValues: (0.7 - 10) - (-2.25).
        assert_eq!(low.y.to_bits(), 0xc01c_3333_3333_3334);
        assert_eq!(high, Point { x: 110.0, y: 81.0 });
        let (fractional_low, fractional_high) =
            graph.unrounded_fixed_node_bounds(&[NodeId(0)]).unwrap();
        assert_eq!(fractional_low.y.to_bits(), 0xc01c_3333_3333_3334);
        assert_eq!(fractional_high, Point { x: 110.1, y: 80.7 });
        assert_eq!(
            ArenaGraph::external_node_bounds(&graph.nodes),
            Some((low, high))
        );
    }

    #[test]
    fn three_dimensional_hexagon_uses_half_of_the_floating_offset() {
        let mut input = Graph::default();
        input.add_node(crate::Node::new(
            "hex",
            Size {
                width: 100.0,
                height: 80.0,
            },
        ));
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(NodeId(0), Point::default());
        graph.nodes[0].shape = ShapeKind::Hexagon;
        graph.nodes[0].is_3d = true;
        graph.nodes[0].is_multiple = true;
        assert_eq!(modifier_element_adjustments(&graph.nodes[0]), (15.0, 7.5));
        assert_eq!(
            graph.ordinary_node_bounds(&[NodeId(0)]),
            Some((Point { x: 0.0, y: -7.5 }, Point { x: 115.0, y: 80.0 }))
        );
    }

    #[test]
    fn projected_fixed_bounds_round_body_corners_before_shape_fit() {
        for (coordinate, expected) in [
            (150.4, 450.0),
            (150.5, 451.0),
            (150.7, 451.0),
            (-450.4, -150.0),
            (-450.5, -151.0),
        ] {
            let mut graph = Graph::default();
            graph.add_node(crate::Node::new(
                "child",
                Size {
                    width: 300.0,
                    height: 300.0,
                },
            ));
            let mut arena = ArenaGraph::from_input(&graph);
            let position = Point {
                x: coordinate,
                y: coordinate,
            };
            arena.nodes[0].position = Some(position);
            arena.nodes[0].fixed_top_left = Some(position);
            let (low, high) =
                ArenaGraph::fixed_external_node_bounds(&arena.nodes).expect("node bounds");
            assert_eq!(arena.fixed_node_bounds(&[NodeId(0)]), Some((low, high)));
            assert_eq!(low, Point { x: 0.0, y: 0.0 });
            assert_eq!(
                high,
                Point {
                    x: expected,
                    y: expected
                }
            );
        }
    }

    #[test]
    fn projected_fixed_bounds_use_the_selected_live_label_side() {
        let mut graph = Graph::default();
        let mut node = crate::Node::new(
            "child",
            Size {
                width: 128.0,
                height: 128.0,
            },
        );
        node.label_size = Some(Size {
            width: 51.0,
            height: 21.0,
        });
        node.label_position = LabelPosition::OutsideTopCenter;
        node.external_label = Some(ExternalLabel {
            size: Size {
                width: 51.0,
                height: 21.0,
            },
            side: ExternalSide::Bottom,
            alignment: crate::ExternalAlignment::Center,
            automatic: true,
            reserve_space: true,
        });
        graph.add_node(node);

        let mut arena = ArenaGraph::from_input(&graph);
        arena.nodes[0].position = Some(Point { x: 0.0, y: 100.0 });

        let (top_left, bottom_right) =
            ArenaGraph::fixed_external_node_bounds(&arena.nodes).expect("node bounds");
        assert_eq!(top_left.y, 69.0);
        assert_eq!(bottom_right.y, 228.0);
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
#[path = "edge_label_bounds_tests.rs"]
mod edge_label_bounds_source_contract;
