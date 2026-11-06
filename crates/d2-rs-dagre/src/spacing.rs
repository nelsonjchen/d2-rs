// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! D2 v0.9.0 d2dagrelayout's post-Dagro spacing passes.
//!
//! The immutable graph retains extracted sequence actors; only objects with
//! boxes in the parent layout participate here. The caller supplies the routes
//! produced by Dagro before spacing; both passes update those routes in place
//! so fitting sees the same moved endpoints and interior bends as upstream.

use super::{has_rankable_children, is_descendant, is_sequence_descendant};
use d2_rs_core::{EdgeId, Graph, Insets, LabelPosition, NodeId, Point, Rect, ShapeKind};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn active_nodes(graph: &Graph, boxes: &BTreeMap<NodeId, Rect>) -> Vec<NodeId> {
    // Both spacing passes range over Go's g.Objects, the same source-ordered
    // slice registered with Dagro. Arena order groups descendants together
    // and can apply a later container's margin before an earlier sibling's.
    super::node_registration_order(graph)
        .into_iter()
        .filter(|id| boxes.contains_key(id) && !is_sequence_descendant(graph, *id))
        .collect()
}

fn start(rect: Rect, horizontal: bool) -> f64 {
    if horizontal {
        rect.origin.x
    } else {
        rect.origin.y
    }
}

fn end(rect: Rect, horizontal: bool) -> f64 {
    if horizontal {
        rect.right()
    } else {
        rect.bottom()
    }
}

fn move_box(rect: &mut Rect, distance: f64, horizontal: bool) {
    if horizontal {
        rect.origin.x += distance;
    } else {
        rect.origin.y += distance;
    }
}

fn grow_box(rect: &mut Rect, distance: f64, horizontal: bool) {
    if horizontal {
        rect.size.width += distance;
    } else {
        rect.size.height += distance;
    }
}

fn before(spacing: Insets, horizontal: bool) -> f64 {
    if horizontal {
        spacing.left
    } else {
        spacing.top
    }
}

fn after(spacing: Insets, horizontal: bool) -> f64 {
    if horizontal {
        spacing.right
    } else {
        spacing.bottom
    }
}

fn point_axis(point: Point, horizontal: bool) -> f64 {
    if horizontal { point.x } else { point.y }
}

fn move_point(point: &mut Point, distance: f64, horizontal: bool) {
    if horizontal {
        point.x += distance
    } else {
        point.y += distance
    }
}

/// The route portion of upstream `shiftDown`, before any object translations.
/// Container growth has already happened when these endpoint guards run.
fn shift_down_routes(
    graph: &Graph,
    boxes: &BTreeMap<NodeId, Rect>,
    routes: &mut BTreeMap<EdgeId, Vec<Point>>,
    position: f64,
    distance: f64,
    horizontal: bool,
) {
    for (id, edge) in graph.edges() {
        let (Some(source), Some(target), Some(route)) = (
            boxes.get(&edge.source),
            boxes.get(&edge.target),
            routes.get_mut(&id).filter(|route| !route.is_empty()),
        ) else {
            continue;
        };
        let last = route.len() - 1;
        for (index, object) in [(0, *source), (last, *target)] {
            let coordinate = point_axis(route[index], horizontal);
            if position <= coordinate {
                let on_static_object =
                    coordinate == end(object, horizontal) && start(object, horizontal) < position;
                if !on_static_object {
                    move_point(&mut route[index], distance, horizontal);
                }
            }
        }
        for point in route.iter_mut().take(last).skip(1) {
            if position <= point_axis(*point, horizontal) {
                move_point(point, distance, horizontal);
            }
        }
    }
}

/// The route portion of upstream `shiftUp`, before any object translations.
fn shift_up_routes(
    graph: &Graph,
    boxes: &BTreeMap<NodeId, Rect>,
    routes: &mut BTreeMap<EdgeId, Vec<Point>>,
    position: f64,
    distance: f64,
    horizontal: bool,
) {
    for (id, edge) in graph.edges() {
        let (Some(source), Some(target), Some(route)) = (
            boxes.get(&edge.source),
            boxes.get(&edge.target),
            routes.get_mut(&id).filter(|route| !route.is_empty()),
        ) else {
            continue;
        };
        let last = route.len() - 1;
        for (index, object, is_target) in [(0, *source, false), (last, *target, true)] {
            let coordinate = point_axis(route[index], horizontal);
            if coordinate <= position {
                // v0.9.0's vertical destination guard compares against its
                // top, not its bottom. Preserve this source asymmetry.
                let static_limit = if !horizontal && is_target {
                    start(object, horizontal)
                } else {
                    end(object, horizontal)
                };
                let on_static_object =
                    coordinate == start(object, horizontal) && position < static_limit;
                if !on_static_object {
                    move_point(&mut route[index], -distance, horizontal);
                }
            }
        }
        for point in route.iter_mut().take(last).skip(1) {
            if point_axis(*point, horizontal) <= position {
                move_point(point, -distance, horizontal);
            }
        }
    }
}

/// Translate Object.Spacing(), including positionLabelsIcons defaults.
/// Unlike TALA's content insets, only the decoration's selected side grows.
pub(super) fn object_spacing(graph: &Graph, id: NodeId) -> (Insets, Insets) {
    let Some(node) = graph.node(id) else {
        return (Insets::uniform(0.0), Insets::uniform(0.0));
    };
    let container = has_rankable_children(graph, id);
    let has_label = node.label_size.is_some()
        && !matches!(
            node.shape,
            ShapeKind::Text | ShapeKind::Code | ShapeKind::Class | ShapeKind::SqlTable
        );
    let mut label_position = node.label_position;
    let mut icon_position = node.icon_position;
    let mut container_icon_default = false;
    if node.has_icon && icon_position.is_none() {
        icon_position = Some(
            if container
                || matches!(
                    node.shape,
                    ShapeKind::Class | ShapeKind::SqlTable | ShapeKind::Code | ShapeKind::Text
                )
            {
                LabelPosition::OutsideTopLeft
            } else {
                LabelPosition::InsideMiddleCenter
            },
        );
        if container && label_position == LabelPosition::Unset {
            label_position = LabelPosition::OutsideTopRight;
            container_icon_default = true;
        }
    }
    if has_label && label_position == LabelPosition::Unset && !container_icon_default {
        label_position = if container {
            LabelPosition::OutsideTopCenter
        } else if matches!(node.shape, ShapeKind::Image | ShapeKind::Person) {
            LabelPosition::OutsideBottomCenter
        } else if node.has_icon {
            LabelPosition::InsideTopCenter
        } else {
            LabelPosition::InsideMiddleCenter
        };
        if let Some(size) = node.label_size
            && (size.width > node.size.width || size.height > node.size.height)
        {
            label_position = if container {
                LabelPosition::OutsideTopCenter
            } else {
                LabelPosition::OutsideBottomCenter
            };
        }
    }
    let mut margin = Insets::uniform(0.0);
    let mut padding = Insets::uniform(0.0);
    if has_label && let Some(size) = node.label_size {
        apply_decoration(
            &mut margin,
            &mut padding,
            label_position,
            if size.width > 0.0 {
                size.width + 10.0
            } else {
                0.0
            },
            if size.height > 0.0 {
                size.height + 10.0
            } else {
                0.0
            },
        );
    }
    if node.has_icon && node.shape != ShapeKind::Image {
        apply_decoration(
            &mut margin,
            &mut padding,
            icon_position.unwrap_or(LabelPosition::Unset),
            74.0,
            74.0,
        );
    }
    let (dx, dy) = super::modifier_element_adjustments(node);
    margin.right += dx;
    margin.top += dy;
    (margin, padding)
}

fn apply_decoration(
    margin: &mut Insets,
    padding: &mut Insets,
    position: LabelPosition,
    width: f64,
    height: f64,
) {
    use LabelPosition::*;
    let (side, value) = match position {
        OutsideTopLeft | OutsideTopCenter | OutsideTopRight => (&mut margin.top, height),
        OutsideBottomLeft | OutsideBottomCenter | OutsideBottomRight => {
            (&mut margin.bottom, height)
        }
        OutsideLeftTop | OutsideLeftMiddle | OutsideLeftBottom => (&mut margin.left, width),
        OutsideRightTop | OutsideRightMiddle | OutsideRightBottom => (&mut margin.right, width),
        InsideTopLeft | InsideTopCenter | InsideTopRight => (&mut padding.top, height),
        InsideBottomLeft | InsideBottomCenter | InsideBottomRight => (&mut padding.bottom, height),
        InsideMiddleLeft => (&mut padding.left, width),
        InsideMiddleRight => (&mut padding.right, width),
        _ => return,
    };
    *side = side.max(value);
}

struct Ranks {
    layers: Vec<Vec<NodeId>>,
    objects: BTreeMap<NodeId, usize>,
    starts: BTreeMap<NodeId, usize>,
    ends: BTreeMap<NodeId, usize>,
}

fn get_ranks(
    graph: &Graph,
    boxes: &BTreeMap<NodeId, Rect>,
    active: &[NodeId],
    horizontal: bool,
) -> Ranks {
    let mut aligned = active
        .iter()
        .copied()
        .filter(|id| !has_rankable_children(graph, *id))
        .map(|id| {
            let rect = boxes[&id];
            let span = if horizontal {
                rect.size.width
            } else {
                rect.size.height
            };
            (id, (start(rect, horizontal) + span / 2.0).ceil())
        })
        .collect::<Vec<_>>();
    aligned.sort_by(|left, right| left.1.total_cmp(&right.1));
    let mut ranks = Ranks {
        layers: Vec::new(),
        objects: BTreeMap::new(),
        starts: BTreeMap::new(),
        ends: BTreeMap::new(),
    };
    let mut previous_center = None;
    for (id, center) in aligned {
        if previous_center != Some(center) {
            ranks.layers.push(Vec::new());
            previous_center = Some(center);
        }
        let rank = ranks.layers.len() - 1;
        ranks.layers[rank].push(id);
        ranks.objects.insert(id, rank);
        let mut parent = graph.node(id).and_then(|node| node.parent);
        while let Some(id) = parent {
            if boxes.contains_key(&id) {
                ranks
                    .starts
                    .entry(id)
                    .and_modify(|value| *value = (*value).min(rank))
                    .or_insert(rank);
                ranks
                    .ends
                    .entry(id)
                    .and_modify(|value| *value = (*value).max(rank))
                    .or_insert(rank);
            }
            parent = graph.node(id).and_then(|node| node.parent);
        }
    }
    ranks
}

/// Apply the source adapter's flow-axis spacing, from the last rank upward.
pub(super) fn adjust_rank_spacing(
    graph: &Graph,
    boxes: &mut BTreeMap<NodeId, Rect>,
    routes: &mut BTreeMap<EdgeId, Vec<Point>>,
    rank_sep: f64,
    horizontal: bool,
) {
    let active = active_nodes(graph, boxes);
    let ranks = get_ranks(graph, boxes, &active, horizontal);
    // Go walks parent.ChildrenArray for each starting/ending ancestor.
    // Keep the active objects' encounter order without rescanning unrelated
    // objects every time that ancestor appears in the frontier.
    let mut children = BTreeMap::<NodeId, Vec<NodeId>>::new();
    for &id in &active {
        if let Some(parent) = graph.node(id).and_then(|node| node.parent) {
            children.entry(parent).or_default().push(id);
        }
    }
    for rank in (0..ranks.layers.len()).rev() {
        let mut starting = Vec::new();
        let mut ending = Vec::new();
        for &id in &ranks.layers[rank] {
            let Some(parent) = graph
                .node(id)
                .and_then(|node| node.parent)
                .filter(|id| boxes.contains_key(id))
            else {
                continue;
            };
            if ranks.starts.get(&parent) == Some(&rank) {
                starting.push(parent);
            }
            if ranks.ends.get(&parent) == Some(&rank) {
                ending.push(parent);
            }
        }
        let mut start_positions = BTreeMap::<NodeId, f64>::new();
        while !starting.is_empty() {
            let mut ancestors = Vec::new();
            for parent in starting {
                let (_, padding) = object_spacing(graph, parent);
                let mut position = start(boxes[&parent], horizontal)
                    - (before(padding, horizontal) - rank_sep / 2.0).max(0.0);
                for &child in children.get(&parent).into_iter().flatten() {
                    let child_rank = ranks
                        .objects
                        .get(&child)
                        .or_else(|| ranks.starts.get(&child));
                    if child_rank.copied().unwrap_or(0) != rank {
                        continue;
                    }
                    let (margin, _) = object_spacing(graph, child);
                    position = position.min(
                        start(boxes[&child], horizontal)
                            - before(margin, horizontal)
                            - before(padding, horizontal),
                    );
                }
                start_positions
                    .entry(parent)
                    .and_modify(|value| *value = value.min(position))
                    .or_insert(position);
                if let Some(parent) = graph
                    .node(parent)
                    .and_then(|node| node.parent)
                    .filter(|id| boxes.contains_key(id))
                {
                    ancestors.push(parent);
                }
            }
            starting = ancestors;
        }
        let mut end_positions = BTreeMap::<NodeId, f64>::new();
        while !ending.is_empty() {
            let mut ancestors = Vec::new();
            for parent in ending {
                let (_, padding) = object_spacing(graph, parent);
                let mut position =
                    end(boxes[&parent], horizontal) + after(padding, horizontal) - rank_sep / 2.0;
                for &child in children.get(&parent).into_iter().flatten() {
                    let child_rank = ranks.objects.get(&child).or_else(|| ranks.ends.get(&child));
                    if child_rank.copied().unwrap_or(0) != rank {
                        continue;
                    }
                    let (margin, _) = object_spacing(graph, child);
                    position = position.max(
                        end(boxes[&child], horizontal)
                            + after(margin, horizontal)
                            + after(padding, horizontal),
                    );
                }
                end_positions
                    .entry(parent)
                    .and_modify(|value| *value = value.max(position))
                    .or_insert(position);
                if let Some(parent) = graph
                    .node(parent)
                    .and_then(|node| node.parent)
                    .filter(|id| boxes.contains_key(id))
                {
                    ancestors.push(parent);
                }
            }
            ending = ancestors;
        }
        let mut end_order = end_positions.into_iter().collect::<Vec<_>>();
        end_order.sort_by(|left, right| right.1.total_cmp(&left.1));
        for (ancestor, desired) in end_order {
            let position = end(boxes[&ancestor], horizontal);
            let delta = desired - position;
            if delta <= 0.0 {
                continue;
            }
            for &id in &active {
                if has_rankable_children(graph, id)
                    && ranks.starts.get(&id).is_some_and(|first| *first <= rank)
                    && ranks.ends.get(&id).is_some_and(|last| rank <= *last)
                    && position <= end(boxes[&id], horizontal)
                {
                    grow_box(boxes.get_mut(&id).unwrap(), delta, horizontal);
                }
            }
            shift_down_routes(graph, boxes, routes, position, delta, horizontal);
            for &id in &active {
                if position <= start(boxes[&id], horizontal) {
                    move_box(boxes.get_mut(&id).unwrap(), delta, horizontal);
                }
            }
        }
        let mut start_order = start_positions.into_iter().collect::<Vec<_>>();
        start_order.sort_by(|left, right| left.1.total_cmp(&right.1));
        for (ancestor, desired) in start_order {
            let position = start(boxes[&ancestor], horizontal);
            let delta = position - desired;
            if delta <= 0.0 {
                continue;
            }
            for &id in &active {
                if has_rankable_children(graph, id)
                    && ranks.starts.get(&id).is_some_and(|first| *first <= rank)
                    && ranks.ends.get(&id).is_some_and(|last| rank <= *last)
                    && start(boxes[&id], horizontal) <= position
                {
                    grow_box(boxes.get_mut(&id).unwrap(), delta, horizontal);
                }
            }
            shift_up_routes(graph, boxes, routes, position, delta, horizontal);
            for &id in &active {
                if start(boxes[&id], horizontal) <= position {
                    move_box(boxes.get_mut(&id).unwrap(), -delta, horizontal);
                }
            }
        }
    }
}

struct CrossIndex {
    incident: BTreeMap<NodeId, Vec<(EdgeId, NodeId)>>,
}

struct Reachable<'a> {
    graph: &'a Graph,
    boxes: &'a mut BTreeMap<NodeId, Rect>,
    routes: &'a mut BTreeMap<EdgeId, Vec<Point>>,
    active: &'a [NodeId],
    index: &'a CrossIndex,
    root: NodeId,
    start: f64,
    distance: f64,
    horizontal: bool,
    margin: bool,
    queue: VecDeque<NodeId>,
    needs_move: BTreeSet<NodeId>,
    seen: BTreeSet<NodeId>,
    seen_order: Vec<NodeId>,
    shifted: BTreeSet<NodeId>,
    shifted_edges: BTreeSet<EdgeId>,
}

impl Reachable<'_> {
    fn enqueue(&mut self, id: NodeId) {
        if self.active.contains(&id) && !self.seen.contains(&id) {
            self.queue.push_back(id);
        }
    }

    fn check_below(&mut self, id: NodeId) {
        let current = self.boxes[&id];
        let original_end = end(current, self.horizontal)
            - if self.shifted.contains(&id) {
                self.distance
            } else {
                0.0
            };
        for &other in self.active {
            if other == id || is_descendant(self.graph, id, other) {
                continue;
            }
            let other_rect = self.boxes[&other];
            if original_end < start(other_rect, self.horizontal)
                && start(other_rect, self.horizontal) < original_end + self.distance + 100.0
                && start(current, !self.horizontal) < end(other_rect, !self.horizontal)
                && start(other_rect, !self.horizontal) < end(current, !self.horizontal)
            {
                self.enqueue(other);
            }
        }
    }

    fn process_queue(&mut self) {
        while let Some(id) = self.queue.pop_front() {
            if self.seen.contains(&id) {
                continue;
            }
            let current_start = start(self.boxes[&id], self.horizontal);
            if id != self.root && !self.needs_move.contains(&id) && current_start < self.start {
                continue;
            }
            let shifts = self.needs_move.contains(&id)
                || if self.margin {
                    self.start <= current_start
                } else {
                    self.start < current_start
                };
            if shifts {
                move_box(
                    self.boxes.get_mut(&id).unwrap(),
                    self.distance,
                    self.horizontal,
                );
                self.shifted.insert(id);
            }
            self.seen.insert(id);
            self.seen_order.push(id);
            if !is_descendant(self.graph, id, self.root)
                && let Some(parent) = self.graph.node(id).and_then(|node| node.parent)
            {
                self.enqueue(parent);
            }
            for &child in self.active {
                if self.graph.node(child).and_then(|node| node.parent) == Some(id) {
                    self.enqueue(child);
                }
            }
            let incident = self.index.incident.get(&id).cloned().unwrap_or_default();
            for (edge_id, other) in incident {
                if !self.shifted_edges.insert(edge_id) {
                    continue;
                }
                if id == other {
                    if let Some(route) = self.routes.get_mut(&edge_id) {
                        for point in route {
                            move_point(point, self.distance, self.horizontal);
                        }
                    }
                    continue;
                }
                let edge = &self.graph.edges[edge_id.0 as usize];
                let is_source = edge.source == id;
                if let Some(route) = self
                    .routes
                    .get_mut(&edge_id)
                    .filter(|route| !route.is_empty())
                {
                    let far_index = if is_source { route.len() - 1 } else { 0 };
                    let far = point_axis(route[far_index], self.horizontal);
                    if self.start <= far
                        && end(self.boxes[&other], self.horizontal) < far + self.distance
                    {
                        self.needs_move.insert(other);
                    }
                    let near_index = if is_source { 0 } else { route.len() - 1 };
                    let near = point_axis(route[near_index], self.horizontal);
                    let special = shifts
                        && near < start(self.boxes[&id], self.horizontal)
                        && near < self.start;
                    for (index, point) in route.iter_mut().enumerate() {
                        if (special && index == near_index)
                            || self.start <= point_axis(*point, self.horizontal)
                        {
                            move_point(point, self.distance, self.horizontal);
                        }
                    }
                }
                self.enqueue(other);
            }
            self.check_below(id);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn shift_reachable(
    graph: &Graph,
    boxes: &mut BTreeMap<NodeId, Rect>,
    routes: &mut BTreeMap<EdgeId, Vec<Point>>,
    active: &[NodeId],
    index: &CrossIndex,
    root: NodeId,
    start_position: f64,
    distance: f64,
    horizontal: bool,
    margin: bool,
) -> BTreeSet<NodeId> {
    let mut walk = Reachable {
        graph,
        boxes,
        routes,
        active,
        index,
        root,
        start: start_position,
        distance,
        horizontal,
        margin,
        queue: VecDeque::from([root]),
        needs_move: BTreeSet::new(),
        seen: BTreeSet::new(),
        seen_order: Vec::new(),
        shifted: BTreeSet::new(),
        shifted_edges: BTreeSet::new(),
    };
    walk.process_queue();
    #[cfg(feature = "diagnostic-traces")]
    super::diagnostics::cross_seen_order(
        graph,
        root,
        start_position,
        distance,
        horizontal,
        margin,
        &walk.seen_order,
    );
    let mut grown = BTreeSet::new();
    // Go's iterator retains the original eight-slot group when a small
    // seen map grows (internal/runtime/maps/table.go, Iter.Init/Next).
    // Collision processing may append more seen objects below, but entries
    // inserted into the replacement table are outside that iterator's group.
    let iteration_limit = if walk.seen_order.len() <= 8 {
        8
    } else {
        usize::MAX
    };
    let mut cursor = 0;
    while cursor < walk.seen_order.len().min(iteration_limit) {
        let id = walk.seen_order[cursor];
        cursor += 1;
        let mut parent = graph.node(id).and_then(|node| node.parent);
        while let Some(id) = parent {
            if !active.contains(&id) || walk.shifted.contains(&id) || grown.contains(&id) {
                break;
            }
            if start(walk.boxes[&id], horizontal) < start_position {
                grow_box(walk.boxes.get_mut(&id).unwrap(), distance, horizontal);
                grown.insert(id);
                walk.check_below(id);
                walk.process_queue();
            }
            parent = graph.node(id).and_then(|node| node.parent);
        }
    }
    let moved = walk.shifted.union(&grown).copied().collect::<Vec<_>>();
    moved
        .iter()
        .copied()
        .filter(|id| {
            let current = walk.boxes[id];
            !moved.iter().any(|other| {
                if other == id {
                    return false;
                }
                let other_rect = walk.boxes[other];
                if end(other_rect, !horizontal) < start(current, !horizontal)
                    || end(current, !horizontal) < start(other_rect, !horizontal)
                {
                    return false;
                }
                start(other_rect, horizontal) < start(current, horizontal)
                    && start(current, horizontal) < end(other_rect, horizontal) + 100.0
            })
        })
        .collect()
}

/// Apply cross-axis margins/padding in source object order.
/// `horizontal` selects the axis being changed, as in upstream's helper.
pub(super) fn adjust_cross_rank_spacing(
    graph: &Graph,
    boxes: &mut BTreeMap<NodeId, Rect>,
    routes: &mut BTreeMap<EdgeId, Vec<Point>>,
    _rank_sep: f64,
    horizontal: bool,
) {
    let active = active_nodes(graph, boxes);
    let mut index = CrossIndex {
        incident: BTreeMap::new(),
    };
    for (id, edge) in graph.edges() {
        if !active.contains(&edge.source) || !active.contains(&edge.target) {
            continue;
        }
        index
            .incident
            .entry(edge.source)
            .or_default()
            .push((id, edge.target));
        if edge.source != edge.target {
            index
                .incident
                .entry(edge.target)
                .or_default()
                .push((id, edge.source));
        }
    }
    let mut previous_before = BTreeMap::<NodeId, f64>::new();
    let mut previous_after = BTreeMap::<NodeId, f64>::new();
    for &id in &active {
        let node = graph.node(id).unwrap();
        if node.grid_rows.is_some() || node.grid_columns.is_some() {
            continue;
        }
        let (margin, padding) = object_spacing(graph, id);
        let margin_after =
            after(margin, horizontal) - previous_after.get(&id).copied().unwrap_or(0.0);
        if margin_after > 0.0 {
            let position = end(boxes[&id], horizontal);
            for moved in shift_reachable(
                graph,
                boxes,
                routes,
                &active,
                &index,
                id,
                position,
                margin_after,
                horizontal,
                true,
            ) {
                previous_after
                    .entry(moved)
                    .and_modify(|value| *value = value.max(margin_after))
                    .or_insert(margin_after);
            }
        }
        let padding_after = after(padding, horizontal);
        if padding_after > 0.0 {
            let position = end(boxes[&id], horizontal);
            shift_reachable(
                graph,
                boxes,
                routes,
                &active,
                &index,
                id,
                position,
                padding_after,
                horizontal,
                false,
            );
            grow_box(boxes.get_mut(&id).unwrap(), padding_after, horizontal);
        }
        let margin_before =
            before(margin, horizontal) - previous_before.get(&id).copied().unwrap_or(0.0);
        if margin_before > 0.0 {
            let position = start(boxes[&id], horizontal);
            for moved in shift_reachable(
                graph,
                boxes,
                routes,
                &active,
                &index,
                id,
                position,
                margin_before,
                horizontal,
                true,
            ) {
                previous_before
                    .entry(moved)
                    .and_modify(|value| *value = value.max(margin_before))
                    .or_insert(margin_before);
            }
        }
        let padding_before = before(padding, horizontal);
        if padding_before > 0.0 {
            let position = start(boxes[&id], horizontal);
            shift_reachable(
                graph,
                boxes,
                routes,
                &active,
                &index,
                id,
                position,
                padding_before,
                horizontal,
                false,
            );
            grow_box(boxes.get_mut(&id).unwrap(), padding_before, horizontal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_rs_core::{Edge, Node, Size};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }

    fn axis_point(coordinate: f64, horizontal: bool) -> Point {
        if horizontal {
            Point {
                x: coordinate,
                y: 17.0,
            }
        } else {
            Point {
                x: 17.0,
                y: coordinate,
            }
        }
    }

    fn axis_rect(coordinate: f64, horizontal: bool) -> Rect {
        if horizontal {
            rect(coordinate, 0.0, 100.0, 20.0)
        } else {
            rect(0.0, coordinate, 20.0, 100.0)
        }
    }

    #[test]
    fn shift_down_preserves_static_endpoints_and_moves_bends_at_the_boundary() {
        for horizontal in [false, true] {
            let mut graph = Graph::default();
            let source = graph.add_node(Node::new("source", Size::default()));
            let target = graph.add_node(Node::new("target", Size::default()));
            let edge = graph.add_edge(Edge { source, target });
            let reverse = graph.add_edge(Edge {
                source: target,
                target: source,
            });
            let boxes = BTreeMap::from([
                (source, axis_rect(0.0, horizontal)),
                (target, axis_rect(200.0, horizontal)),
            ]);
            let points = [100.0, 99.0, 100.0, 150.0, 200.0]
                .map(|value| axis_point(value, horizontal))
                .to_vec();
            let mut routes = BTreeMap::from([
                (edge, points.clone()),
                (reverse, points.into_iter().rev().collect()),
            ]);
            shift_down_routes(&graph, &boxes, &mut routes, 100.0, 25.0, horizontal);
            let expected = [100.0, 99.0, 125.0, 175.0, 225.0]
                .map(|value| axis_point(value, horizontal))
                .to_vec();
            assert_eq!(routes[&edge], expected);
            assert_eq!(
                routes[&reverse],
                expected.into_iter().rev().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn shift_up_preserves_v090_vertical_destination_guard() {
        for horizontal in [false, true] {
            let mut graph = Graph::default();
            let source = graph.add_node(Node::new("source", Size::default()));
            let target = graph.add_node(Node::new("target", Size::default()));
            let edge = graph.add_edge(Edge { source, target });
            let boxes = BTreeMap::from([
                (source, axis_rect(0.0, horizontal)),
                (target, axis_rect(0.0, horizontal)),
            ]);
            let mut routes = BTreeMap::from([(
                edge,
                [0.0, -1.0, 0.0, 1.0, 0.0]
                    .map(|value| axis_point(value, horizontal))
                    .to_vec(),
            )]);
            shift_up_routes(&graph, &boxes, &mut routes, 0.0, 25.0, horizontal);
            // Source shiftUp keeps the horizontal destination on its static
            // side, but its vertical destination guard does not keep it.
            let target_coordinate = if horizontal { 0.0 } else { -25.0 };
            assert_eq!(
                routes[&edge],
                [0.0, -26.0, -25.0, 1.0, target_coordinate]
                    .map(|value| axis_point(value, horizontal))
                    .to_vec()
            );
        }
    }

    #[test]
    fn icon_and_label_spacing_share_one_side_and_modifiers_add_afterward() {
        let mut graph = Graph::default();
        let mut node = Node::new(
            "label",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        node.label_size = Some(Size {
            width: 90.0,
            height: 20.0,
        });
        node.label_position = LabelPosition::InsideMiddleLeft;
        node.has_icon = true;
        node.icon_position = Some(LabelPosition::InsideMiddleLeft);
        node.is_3d = true;
        node.shape = ShapeKind::Hexagon;
        let id = graph.add_node(node);
        let (margin, padding) = object_spacing(&graph, id);
        assert_eq!(
            padding,
            Insets {
                left: 100.0,
                ..Insets::uniform(0.0)
            }
        );
        assert_eq!(
            margin,
            Insets {
                right: 15.0,
                top: 7.0,
                ..Insets::uniform(0.0)
            }
        );
        graph.node_mut(id).unwrap().shape = ShapeKind::Image;
        graph.node_mut(id).unwrap().label_size = None;
        graph.node_mut(id).unwrap().is_3d = false;
        assert_eq!(
            object_spacing(&graph, id),
            (Insets::uniform(0.0), Insets::uniform(0.0))
        );
    }

    #[test]
    fn rank_spacing_grows_parent_and_moves_earlier_ranks_for_outside_icon() {
        let mut graph = Graph::default();
        let parent = graph.add_node(Node::new(
            "parent",
            Size {
                width: 100.0,
                height: 120.0,
            },
        ));
        let child = graph.add_node(Node::new(
            "parent.child",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let earlier = graph.add_node(Node::new(
            "earlier",
            Size {
                width: 20.0,
                height: 20.0,
            },
        ));
        let node = graph.node_mut(child).unwrap();
        node.parent = Some(parent);
        node.has_icon = true;
        node.icon_position = Some(LabelPosition::OutsideTopCenter);
        let mut boxes = BTreeMap::from([
            (parent, rect(0.0, 100.0, 100.0, 120.0)),
            (child, rect(40.0, 150.0, 20.0, 20.0)),
            (earlier, rect(200.0, 0.0, 20.0, 20.0)),
        ]);
        let edge = graph.add_edge(Edge {
            source: earlier,
            target: child,
        });
        let mut routes = BTreeMap::from([(
            edge,
            vec![
                Point { x: 210.0, y: 20.0 },
                Point { x: 150.0, y: 73.0 },
                Point { x: 100.0, y: 100.0 },
                Point { x: 50.0, y: 150.0 },
            ],
        )]);
        adjust_rank_spacing(&graph, &mut boxes, &mut routes, 100.0, false);
        assert_eq!(boxes[&parent], rect(0.0, 76.0, 100.0, 144.0));
        assert_eq!(boxes[&child].origin.y, 150.0);
        assert_eq!(boxes[&earlier].origin.y, -24.0);
        assert_eq!(
            routes[&edge],
            vec![
                Point { x: 210.0, y: -4.0 },
                Point { x: 150.0, y: 49.0 },
                Point { x: 100.0, y: 76.0 },
                Point { x: 50.0, y: 150.0 },
            ]
        );
        assert!(graph.edge_route(edge).is_empty());
    }

    #[test]
    fn outside_margin_moves_neighbors_and_grows_parent_once() {
        let mut graph = Graph::default();
        let parent = graph.add_node(Node::new(
            "parent",
            Size {
                width: 260.0,
                height: 120.0,
            },
        ));
        let first = graph.add_node(Node::new(
            "parent.first",
            Size {
                width: 100.0,
                height: 20.0,
            },
        ));
        let second = graph.add_node(Node::new(
            "parent.second",
            Size {
                width: 100.0,
                height: 20.0,
            },
        ));
        for id in [first, second] {
            graph.node_mut(id).unwrap().parent = Some(parent);
        }
        let node = graph.node_mut(first).unwrap();
        node.has_icon = true;
        node.icon_position = Some(LabelPosition::OutsideLeftMiddle);
        let mut boxes = BTreeMap::from([
            (parent, rect(0.0, 0.0, 260.0, 120.0)),
            (first, rect(20.0, 50.0, 100.0, 20.0)),
            (second, rect(140.0, 50.0, 100.0, 20.0)),
        ]);
        adjust_cross_rank_spacing(&graph, &mut boxes, &mut BTreeMap::new(), 100.0, true);
        assert_eq!(boxes[&first].origin.x, 94.0);
        assert_eq!(boxes[&second].origin.x, 214.0);
        assert_eq!(boxes[&parent].size.width, 334.0);
    }

    #[test]
    fn cross_spacing_publishes_routes_and_forces_the_far_endpoint_once() {
        let mut graph = Graph::default();
        let source = graph.add_node(Node::new("source", Size::default()));
        let target = graph.add_node(Node::new("target", Size::default()));
        let node = graph.node_mut(source).unwrap();
        node.has_icon = true;
        node.icon_position = Some(LabelPosition::OutsideLeftMiddle);
        let edge = graph.add_edge(Edge { source, target });
        let self_edge = graph.add_edge(Edge {
            source,
            target: source,
        });
        let mut boxes = BTreeMap::from([
            (source, rect(100.0, 0.0, 20.0, 20.0)),
            (target, rect(0.0, 100.0, 150.0, 20.0)),
        ]);
        let mut routes = BTreeMap::from([
            (
                edge,
                vec![
                    Point { x: 90.0, y: 10.0 },
                    Point { x: 120.0, y: 60.0 },
                    Point { x: 130.0, y: 100.0 },
                ],
            ),
            (
                self_edge,
                vec![
                    Point { x: 90.0, y: 5.0 },
                    Point { x: 100.0, y: -10.0 },
                    Point { x: 120.0, y: 5.0 },
                ],
            ),
        ]);
        adjust_cross_rank_spacing(&graph, &mut boxes, &mut routes, 100.0, true);
        assert_eq!(boxes[&source].origin.x, 174.0);
        // This node begins behind the shifting plane. Its incoming route
        // endpoint would leave the box, so upstream forces the node to move.
        assert_eq!(boxes[&target].origin.x, 74.0);
        assert_eq!(
            routes[&edge],
            vec![
                Point { x: 164.0, y: 10.0 },
                Point { x: 194.0, y: 60.0 },
                Point { x: 204.0, y: 100.0 },
            ]
        );
        assert_eq!(
            routes[&self_edge],
            vec![
                Point { x: 164.0, y: 5.0 },
                Point { x: 174.0, y: -10.0 },
                Point { x: 194.0, y: 5.0 },
            ]
        );
        assert!(graph.edge_route(edge).is_empty());
    }
}
