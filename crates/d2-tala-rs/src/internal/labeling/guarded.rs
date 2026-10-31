// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! labeling.Place/PlaceNewEdges and their independent validation/work boundary.
#[path = "guarded_nodes.rs"]
mod node_stage;
use super::*;
use crate::internal::{
    OwnerGraphId,
    limits::{Context, Error},
};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
const NEW_LOCATION: &str = "PlaceNewEdgeLabels";
const ALL_LOCATION: &str = "PlaceLabels";
pub(super) const MAX_WORK: i64 = 50_000_000;
fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn check(context: &dyn Context, location: &'static str) -> Result<(), Error> {
    match context.error() {
        Some(cause) => Err(Error::Context {
            location: location.into(),
            cause,
        }),
        None => Ok(()),
    }
}
struct Work<'a> {
    context: &'a dyn Context,
    used: Cell<i64>,
    limit: i64,
    location: &'static str,
    recover_invariants: bool,
}
impl<'a> Work<'a> {
    fn new(context: &'a dyn Context, limit: i64, location: &'static str) -> Result<Self, Error> {
        if limit < 0 {
            return Err(invalid(format!(
                "TALA {location} work limit must not be negative"
            )));
        }
        let work = Self {
            context,
            used: Cell::new(0),
            limit,
            location,
            recover_invariants: false,
        };
        work.observe();
        work.check()?;
        Ok(work)
    }
    // Source runLocalSeed converts label-stage nil dereferences to an ordered
    // seed failure. WASM cannot unwind, so keep that recovery path fallible.
    // Standalone Place/PlaceNewEdges still expose the source panic contract.
    fn required<T>(&self, value: Option<T>) -> Result<T, Error> {
        match value {
            Some(value) => Ok(value),
            None if self.recover_invariants => Err(Error::SeedInvariant(
                "runtime error: invalid memory address or nil pointer dereference",
            )),
            None => panic!("runtime error: invalid memory address or nil pointer dereference"),
        }
    }
    fn observe(&self) {
        #[cfg(feature = "diagnostic-traces")]
        self.context.label_placement_work_used(self.used.get());
    }
    fn check(&self) -> Result<(), Error> {
        check(self.context, self.location)
    }
    fn step(&self) -> Result<(), Error> {
        self.add(1)
    }
    fn add(&self, units: i64) -> Result<(), Error> {
        let location = self.location;
        if units < 0 {
            return Err(invalid(format!(
                "TALA {location} received a negative work estimate"
            )));
        }
        let before = self.used.get();
        if before < 0 || before > self.limit || units > self.limit - before {
            self.check()?;
            return Err(Error::Limit {
                location: self.location.into(),
                limit: self.limit,
            });
        }
        self.used.set(before + units);
        self.observe();
        if units == 0 || before / 64 != self.used.get() / 64 {
            self.check()?;
        }
        Ok(())
    }
}

fn validate(
    graph: &ArenaGraph,
    selected: &[Option<usize>],
    context: Option<&dyn Context>,
    location: &'static str,
) -> Result<(), Error> {
    graph.validate_owner_graph_topology(Some(OwnerGraphId(0)), context, location)?;
    let context = context.ok_or_else(|| invalid(format!("TALA {location} requires a context")))?;
    check(context, location)?;
    let view = graph.owner_graph_view(OwnerGraphId(0)).unwrap();
    if view.nodes.len() > 10_000 {
        return Err(invalid(format!(
            "TALA {location} node count exceeds limit 10000"
        )));
    }
    if view.edges.header().len() > 50_000 {
        return Err(invalid(format!(
            "TALA {location} edge count exceeds limit 50000"
        )));
    }
    if selected.len() > 50_000 {
        return Err(invalid(format!(
            "TALA {location} requested edge count exceeds limit 50000"
        )));
    }
    let mut checks = 0usize;
    let mut poll = || {
        checks += 1;
        if checks % 64 == 0 {
            check(context, location)
        } else {
            Ok(())
        }
    };
    for (index, id) in view.nodes.iter_slots().enumerate() {
        poll()?;
        let id = id.ok_or_else(|| {
            invalid(format!(
                "TALA {location} graph node at index {index} is nil"
            ))
        })?;
        let node = &graph.nodes[id.0 as usize];
        if graph.position(id).is_none() {
            return Err(invalid(format!(
                "TALA {location} graph node {} has no position",
                node.tala_id
            )));
        }
        if graph
            .owner_node_group_vessel(id, crate::internal::owner_groups::OwnerGroupKind::Cluster)
            .unwrap()
            == Some(None)
        {
            return Err(invalid(format!(
                "TALA {location} graph node {} has a cluster without a vessel",
                node.tala_id
            )));
        }
        if graph
            .owner_node_group_vessel(id, crate::internal::owner_groups::OwnerGroupKind::Sequence)
            .unwrap()
            == Some(None)
        {
            return Err(invalid(format!(
                "TALA {location} graph node {} has a sequence without a vessel",
                node.tala_id
            )));
        }
    }
    let mut validated = BTreeSet::new();
    let mut route_points = 0usize;
    for (kind, entries) in [
        (
            "graph edge",
            view.edges
                .header()
                .iter_slots()
                .map(|e| e.map(|e| e.0 as usize))
                .collect::<Vec<_>>(),
        ),
        ("requested edge", selected.to_vec()),
    ] {
        let mut seen = BTreeSet::new();
        for (index, id) in entries.into_iter().enumerate() {
            poll()?;
            let id = id.ok_or_else(|| {
                invalid(format!("TALA {location} {kind} at index {index} is nil"))
            })?;
            let edge = &graph.edge_records[id];
            if !seen.insert(id) {
                return Err(invalid(format!(
                    "TALA {location} {kind} at index {index} repeats edge {}",
                    edge.tala_id
                )));
            }
            if !validated.insert(id) {
                continue;
            }
            if edge.from.0 as usize >= graph.nodes.len() || edge.to.0 as usize >= graph.nodes.len()
            {
                return Err(invalid(format!(
                    "TALA {location} edge {} has missing endpoints",
                    edge.tala_id
                )));
            }
            if edge.route.len() > 1_000_000 - route_points {
                return Err(invalid(format!(
                    "TALA {location} route point count exceeds limit 1000000"
                )));
            }
            route_points += edge.route.len();
            for (index, point) in edge.route.header().iter_slots().enumerate() {
                poll()?;
                if point.is_none() {
                    return Err(invalid(format!(
                        "TALA {location} edge {} route point at index {index} is nil",
                        edge.tala_id
                    )));
                }
            }
            if (edge.label.is_some()
                || edge.source_arrowhead_label.is_some()
                || edge.target_arrowhead_label.is_some())
                && edge.route.len() < 2
            {
                return Err(invalid(format!(
                    "TALA {location} labeled edge {} requires at least two route points",
                    edge.tala_id
                )));
            }
        }
    }
    check(context, location)
}

pub(super) fn place(
    graph: &mut ArenaGraph,
    selected: &[Option<usize>],
    context: Option<&dyn Context>,
    limit: i64,
) -> Result<(), Error> {
    transaction(graph, selected, context, limit, false, false)
}
pub(super) fn place_all(
    graph: &mut ArenaGraph,
    context: Option<&dyn Context>,
    limit: i64,
) -> Result<(), Error> {
    transaction(graph, &[], context, limit, true, false)
}
pub(super) fn place_seed(
    graph: &mut ArenaGraph,
    context: Option<&dyn Context>,
    limit: i64,
) -> Result<(), Error> {
    transaction(graph, &[], context, limit, true, true)
}
fn transaction(
    graph: &mut ArenaGraph,
    selected: &[Option<usize>],
    context: Option<&dyn Context>,
    limit: i64,
    all: bool,
    recover_invariants: bool,
) -> Result<(), Error> {
    let location = if all { ALL_LOCATION } else { NEW_LOCATION };
    validate(graph, selected, context, location)?;
    let selected = selected.iter().map(|id| id.unwrap()).collect::<Vec<_>>();
    // Snapshot only label positions and edge percentages, like the source.
    // A context's independent geometry mutation must not be rolled back here.
    let nodes = graph
        .graph_node_order()
        .into_iter()
        .map(|id| {
            (
                id,
                graph.nodes[id.0 as usize].label_position,
                graph.nodes[id.0 as usize].icon_position,
            )
        })
        .collect::<Vec<_>>();
    let edge_ids = graph
        .member_edge_indices()
        .chain(selected.iter().copied())
        .collect::<BTreeSet<_>>();
    let labels = edge_ids
        .iter()
        .map(|&id| {
            (
                id,
                graph.edge_records[id]
                    .label
                    .as_ref()
                    .map(|label| (label.position, label.percentage)),
            )
        })
        .collect::<Vec<_>>();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut work = Work::new(context.unwrap(), limit, location)?;
        work.recover_invariants = recover_invariants;
        if all {
            place_all_inner(graph, &work)
        } else {
            place_inner(graph, &selected, &work)
        }
    }));
    if !matches!(&result, Ok(Ok(()))) {
        for (id, position, icon) in nodes {
            graph.nodes[id.0 as usize].label_position = position;
            graph.nodes[id.0 as usize].icon_position = icon;
        }
        for (id, state) in labels {
            if let (Some(label), Some((position, percentage))) =
                (&mut graph.edge_records[id].label, state)
            {
                label.position = position;
                label.percentage = percentage;
            }
        }
    }
    match result {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
fn label_box(
    graph: &ArenaGraph,
    id: usize,
    position: LabelPosition,
    work: &Work<'_>,
) -> Result<Option<Rect>, Error> {
    for _ in 0..2 {
        work.add(graph.edge_records[id].route.len() as i64)?;
    }
    let edge = &graph.edge_records[id];
    let label = edge.label.as_ref().unwrap();
    let rect = label_rect(edge, position, label.percentage);
    work.check()?;
    Ok(rect)
}
fn arrow_box(graph: &ArenaGraph, id: usize, target: bool, work: &Work<'_>) -> Result<Rect, Error> {
    for _ in 0..3 {
        work.add(graph.edge_records[id].route.len() as i64)?;
    }
    let rect =
        arrowhead_label_rect(&graph.edge_records[id], target).expect("validated arrowhead route");
    work.check()?;
    Ok(rect)
}
fn length(graph: &ArenaGraph, id: usize, work: &Work<'_>) -> Result<f64, Error> {
    let mut length = 0.0;
    let mut index = 1;
    while index < graph.edge_records[id].route.len() {
        work.step()?;
        let route = &graph.edge_records[id].route;
        length += label_segment_length(route.point(index - 1), route.point(index));
        index += 1;
    }
    Ok(length)
}
fn stable_sort<T: Copy>(
    items: &mut Vec<T>,
    less: impl Fn(T, T) -> bool,
    work: &Work<'_>,
) -> Result<(), Error> {
    let mut buffer = items.clone();
    let mut width = 1;
    while width < items.len() {
        let mut start = 0;
        while start < items.len() {
            let middle = (start + width).min(items.len());
            let end = (start + 2 * width).min(items.len());
            let (mut left, mut right, mut output) = (start, middle, start);
            while left < middle && right < end {
                work.step()?;
                if less(items[right], items[left]) {
                    buffer[output] = items[right];
                    right += 1;
                } else {
                    buffer[output] = items[left];
                    left += 1;
                }
                output += 1;
            }
            while left < middle {
                work.step()?;
                buffer[output] = items[left];
                left += 1;
                output += 1;
            }
            while right < end {
                work.step()?;
                buffer[output] = items[right];
                right += 1;
                output += 1;
            }
            start += 2 * width;
        }
        std::mem::swap(items, &mut buffer);
        if width > items.len() / 2 {
            break;
        }
        width *= 2;
    }
    Ok(())
}
fn shared_boxes(graph: &ArenaGraph, work: &Work<'_>) -> Result<Vec<Rect>, Error> {
    work.step()?;
    let mut groups: [BTreeMap<(u64, usize), Vec<AxisSegment>>; 2] =
        std::array::from_fn(|_| BTreeMap::new());
    let mut nan_key = 0usize;
    for id in graph.member_edge_indices() {
        work.step()?;
        let mut index = 0;
        while index + 1 < graph.edge_records[id].route.len() {
            work.step()?;
            let route = &graph.edge_records[id].route;
            let (a, b) = (route.point(index), route.point(index + 1));
            index += 1;
            if a == b {
                continue;
            }
            let (axis, fixed, mut start, mut end) = if a.x == b.x {
                (0, a.x, a.y, b.y)
            } else if a.y == b.y {
                (1, a.y, a.x, b.x)
            } else {
                continue;
            };
            if end < start {
                std::mem::swap(&mut start, &mut end);
            }
            groups[axis]
                .entry((
                    crate::internal::routing::go_float_map_key(fixed),
                    if fixed.is_nan() {
                        nan_key += 1;
                        nan_key
                    } else {
                        0
                    },
                ))
                .or_default()
                .push(AxisSegment { fixed, start, end });
        }
    }
    let mut shared = Vec::new();
    for (axis, groups) in groups.into_iter().enumerate() {
        for (_, mut segments) in groups {
            work.step()?;
            if segments.len() == 1 {
                continue;
            }
            stable_sort(
                &mut segments,
                |a, b| {
                    if a.start == b.start {
                        a.end < b.end
                    } else {
                        a.start < b.start
                    }
                },
                work,
            )?;
            let mut previous = segments[0];
            let mut current = None::<AxisSegment>;
            let emit = |segment: AxisSegment, shared: &mut Vec<(usize, AxisSegment)>| {
                shared.push((axis, segment))
            };
            for next in segments.into_iter().skip(1) {
                work.step()?;
                if previous.end > next.start {
                    if current.is_some_and(|part| next.start > part.end) {
                        emit(current.take().unwrap(), &mut shared);
                    }
                    let end = previous.end.min(next.end);
                    if let Some(part) = &mut current {
                        part.end = part.end.max(end);
                    } else {
                        current = Some(AxisSegment { end, ..next });
                    }
                } else if let Some(part) = current.take() {
                    emit(part, &mut shared);
                }
                if next.end > previous.end {
                    previous = next;
                }
            }
            if let Some(part) = current {
                emit(part, &mut shared);
            }
        }
    }
    let mut boxes = Vec::with_capacity(shared.len());
    for (axis, segment) in shared {
        work.step()?;
        boxes.push(if axis == 0 {
            Rect {
                origin: Point {
                    x: segment.fixed - 12.5,
                    y: segment.start,
                },
                size: Size {
                    width: 25.0,
                    height: segment.end - segment.start,
                },
            }
        } else {
            Rect {
                origin: Point {
                    x: segment.start,
                    y: segment.fixed - 12.5,
                },
                size: Size {
                    width: segment.end - segment.start,
                    height: 25.0,
                },
            }
        });
    }
    Ok(boxes)
}
fn place_inner(graph: &mut ArenaGraph, selected: &[usize], work: &Work<'_>) -> Result<(), Error> {
    let mut new_edges = BTreeSet::new();
    for &id in selected {
        work.step()?;
        new_edges.insert(id);
    }
    let mut placed = Vec::new();
    for id in graph.member_edge_indices() {
        work.step()?;
        let edge = &graph.edge_records[id];
        if edge.source_arrowhead_label.is_some() {
            placed.push(Some(arrow_box(graph, id, false, work)?));
        }
        if edge.target_arrowhead_label.is_some() {
            placed.push(Some(arrow_box(graph, id, true, work)?));
        }
        if edge.label.is_none()
            || edge.from == edge.to
            || new_edges.contains(&id) && !edge.label_position_fixed
        {
            continue;
        }
        placed.push(label_box(
            graph,
            id,
            edge.label.as_ref().unwrap().position,
            work,
        )?);
    }
    for id in graph.member_edge_indices() {
        work.step()?;
        let edge = &graph.edge_records[id];
        if edge.from == edge.to
            && let Some(label) = &edge.label
        {
            placed.push(label_box(graph, id, label.position, work)?);
        }
    }
    for id in graph.graph_node_order() {
        work.step()?;
        placed.extend(
            node::positioned_node_label_obstacles(graph, id)
                .into_iter()
                .map(Some),
        );
    }
    place_edges(graph, selected, placed, work)
}
fn place_all_inner(graph: &mut ArenaGraph, work: &Work<'_>) -> Result<(), Error> {
    let mut placed = Vec::new();
    for id in graph.member_edge_indices() {
        work.step()?;
        if graph.edge_records[id].source_arrowhead_label.is_some() {
            placed.push(Some(arrow_box(graph, id, false, work)?));
        }
        if graph.edge_records[id].target_arrowhead_label.is_some() {
            placed.push(Some(arrow_box(graph, id, true, work)?));
        }
    }
    for id in graph.member_edge_indices() {
        work.step()?;
        let edge = &graph.edge_records[id];
        if let Some(label) = &edge.label
            && (edge.from == edge.to || edge.label_position_fixed)
        {
            placed.push(label_box(graph, id, label.position, work)?);
        }
    }
    node_stage::place(graph, &mut placed, work)?;
    place_edges(
        graph,
        &graph.member_edge_indices().collect::<Vec<_>>(),
        placed,
        work,
    )
}
fn place_edges(
    graph: &mut ArenaGraph,
    selected: &[usize],
    mut placed: Vec<Option<Rect>>,
    work: &Work<'_>,
) -> Result<(), Error> {
    let shared = shared_boxes(graph, work)?;
    let mut items = Vec::new();
    for &id in selected {
        work.step()?;
        let edge = &graph.edge_records[id];
        if edge.label.is_some() && edge.from != edge.to && !edge.label_position_fixed {
            items.push(id);
        }
    }
    let mut sorted = Vec::with_capacity(items.len());
    for id in items {
        work.step()?;
        sorted.push((id, length(graph, id, work)?));
    }
    stable_sort(
        &mut sorted,
        |a, b| {
            let left = graph.edge_records[a.0].route.len();
            let right = graph.edge_records[b.0].route.len();
            if right == 2 && left > 2 {
                true
            } else if left == 2 && right > 2 {
                false
            } else {
                a.1 < b.1
            }
        },
        work,
    )?;
    work.check()?;
    for (id, _) in sorted {
        work.step()?;
        let (rect, position, percentage) = best_position(graph, id, &placed, &shared, work)?;
        placed.push(rect);
        let label = graph.edge_records[id].label.as_mut().unwrap();
        label.position = position;
        label.percentage = percentage;
    }
    work.check()
}
fn descendant(
    graph: &ArenaGraph,
    mut current: Option<NodeId>,
    ancestor: NodeId,
    work: &Work<'_>,
) -> Result<bool, Error> {
    let location = work.location;
    for depth in 0.. {
        if current == Some(ancestor) {
            return Ok(true);
        }
        let Some(id) = current else { return Ok(false) };
        if depth >= 256 {
            return Err(invalid(format!(
                "TALA {location} ancestry depth exceeds limit 256"
            )));
        }
        work.step()?;
        let node = graph.topology_node(id).unwrap();
        current = if node.container.is_some() {
            node.container
        } else if let Some(vessel) = graph
            .owner_node_group_vessel(id, crate::internal::owner_groups::OwnerGroupKind::Cluster)
            .unwrap()
        {
            vessel
        } else {
            graph
                .owner_node_group_vessel(
                    id,
                    crate::internal::owner_groups::OwnerGroupKind::Sequence,
                )
                .unwrap()
                .flatten()
        };
    }
    unreachable!()
}
fn node_area(
    graph: &ArenaGraph,
    rect: Rect,
    nodes: &[NodeId],
    partial: bool,
    work: &Work<'_>,
) -> Result<(f64, usize), Error> {
    let (mut area, mut count) = (0.0, 0);
    for &id in nodes {
        work.step()?;
        let node = &graph.nodes[id.0 as usize];
        let other = Rect {
            origin: graph.position(id).expect("validated position"),
            size: node.rect.size,
        };
        if !overlaps(rect, other, 5.0) || (partial && covers(other, rect)) {
            continue;
        }
        if overlaps(rect, other, 0.0) {
            area += overlap_area(rect, other);
            count += 1;
        }
        if !node.is_container {
            area += rect.size.width * rect.size.height;
            count += 1;
        }
    }
    Ok((area, count))
}
fn edge_count(
    graph: &ArenaGraph,
    rect: Rect,
    edges: &[usize],
    padding: f64,
    work: &Work<'_>,
) -> Result<usize, Error> {
    let mut count = 0;
    for &id in edges {
        work.step()?;
        let mut i = 1;
        while i < graph.edge_records[id].route.len() {
            work.step()?;
            let route = &graph.edge_records[id].route;
            if rect_overlaps_segment(rect, route.point(i - 1), route.point(i), padding) {
                count += 1;
            }
            i += 1;
        }
    }
    Ok(count)
}
fn boxes_count(
    rect: Rect,
    boxes: impl IntoIterator<Item = Option<Rect>>,
    padding: f64,
    work: &Work<'_>,
) -> Result<usize, Error> {
    let mut count = 0;
    for other in boxes {
        work.step()?;
        if overlaps(rect, work.required(other)?, padding) {
            count += 1;
        }
    }
    Ok(count)
}
fn score(
    graph: &ArenaGraph,
    id: usize,
    position: LabelPosition,
    other_edges: &[usize],
    ancestors: &[NodeId],
    others: &[NodeId],
    placed: &[Option<Rect>],
    shared: &[Rect],
    work: &Work<'_>,
) -> Result<(f64, Rect), Error> {
    work.step()?;
    let rect = work.required(label_box(graph, id, position, work)?)?;
    let edge = &graph.edge_records[id];
    let mut edges = if is_on_edge(position) {
        other_edges.to_vec()
    } else {
        graph.member_edge_indices().collect()
    };
    let from = cluster(graph, edge.from);
    let to = cluster(graph, edge.to);
    if from.is_some() || to.is_some() {
        for &other_id in other_edges {
            work.step()?;
            let other = &graph.edge_records[other_id];
            if from.is_some() && cluster(graph, other.from) == from
                || from.is_none() && cluster(graph, other.to) == to
            {
                edges.push(other_id);
            }
        }
    }
    let (a, ac) = node_area(graph, rect, ancestors, true, work)?;
    let (b, bc) = node_area(graph, rect, others, false, work)?;
    let ep = edge_count(graph, rect, &edges, 5.0, work)?;
    let ee = edge_count(graph, rect, &edges, 0.0, work)?;
    let ec = ep + (ee as f64 * 0.5).ceil() as usize;
    let lp = boxes_count(rect, placed.iter().copied(), 5.0, work)?;
    let le = boxes_count(rect, placed.iter().copied(), 0.0, work)?;
    let almost = lp + (le as f64 * 0.5).ceil() as usize;
    let exact = boxes_count(rect, placed.iter().copied(), 0.0, work)?;
    let sc = boxes_count(rect, shared.iter().copied().map(Some), 5.0, work)?;
    Ok((
        edge_label_overlap_score(
            rect.size.width * rect.size.height,
            a + b,
            exact,
            ec,
            almost,
            ac + bc,
            sc,
        ),
        rect,
    ))
}
fn range(graph: &ArenaGraph, id: usize, length: f64) -> (f64, f64) {
    let edge = &graph.edge_records[id];
    let label = edge.label.as_ref().unwrap();
    let hh = label.size.height / 2.0;
    let hw = label.size.width / 2.0;
    if cluster(graph, edge.from).is_some() {
        let (a, b) = (edge.route.point(0), edge.route.point(1));
        if a.x == b.x && (a.y - b.y).abs() > hh {
            return (0.0, (((a.y - b.y).abs() - hh) / length).min(1.0));
        } else if (a.x - b.x).abs() > hw {
            return (0.0, (((a.x - b.x).abs() - hw) / length).min(1.0));
        }
    } else if cluster(graph, edge.to).is_some() {
        let (a, b) = (
            edge.route.point(edge.route.len() - 1),
            edge.route.point(edge.route.len() - 2),
        );
        if a.x == b.x && (a.y - b.y).abs() > hh {
            return (((length - (a.y - b.y).abs() + hh) / length).max(0.0), 1.0);
        } else if (a.x - b.x).abs() > hw {
            return (((length - (a.x - b.x).abs() + hw) / length).max(0.0), 1.0);
        }
    }
    (0.0, 1.0)
}
fn best_position(
    graph: &mut ArenaGraph,
    id: usize,
    placed: &[Option<Rect>],
    shared: &[Rect],
    work: &Work<'_>,
) -> Result<(Option<Rect>, LabelPosition, f64), Error> {
    let mut other_edges = Vec::new();
    for other in graph.member_edge_indices() {
        work.step()?;
        if other != id {
            other_edges.push(other);
        }
    }
    let positions = positions(graph, id, work)?;
    let mut ancestors = Vec::new();
    let mut others = Vec::new();
    for node in graph.graph_node_order() {
        work.step()?;
        let edge = &graph.edge_records[id];
        let from = descendant(graph, Some(edge.from), node, work)?;
        let to = descendant(graph, Some(edge.to), node, work)?;
        if node != edge.from && node != edge.to && (from || to) {
            ancestors.push(node);
        } else {
            others.push(node);
        }
    }
    let mut best = None;
    let mut best_score = f64::INFINITY;
    let mut route_length = None;
    for position in positions {
        work.step()?;
        if best_score == 0.0 {
            break;
        }
        if is_unlocked(position) && graph.edge_records[id].label.as_ref().unwrap().percentage == 0.0
        {
            let length = match route_length {
                Some(value) => value,
                None => {
                    let value = length(graph, id, work)?;
                    route_length = Some(value);
                    value
                }
            };
            let (mut value, end) = range(graph, id, length);
            while value < end {
                work.step()?;
                graph.edge_records[id].label.as_mut().unwrap().percentage = value;
                let (score, rect) = score(
                    graph,
                    id,
                    position,
                    &other_edges,
                    &ancestors,
                    &others,
                    placed,
                    shared,
                    work,
                )?;
                if score < best_score {
                    best_score = score;
                    best = Some((Some(rect), position, value));
                }
                if best_score == 0.0 {
                    break;
                }
                value += 0.025;
            }
            graph.edge_records[id].label.as_mut().unwrap().percentage = 0.0;
        } else {
            let (score, rect) = score(
                graph,
                id,
                position,
                &other_edges,
                &ancestors,
                &others,
                placed,
                shared,
                work,
            )?;
            if score < best_score {
                best_score = score;
                best = Some((
                    Some(rect),
                    position,
                    graph.edge_records[id].label.as_ref().unwrap().percentage,
                ));
            }
        }
    }
    work.check()?;
    Ok(best.unwrap_or((None, LabelPosition::Unset, 0.0)))
}
fn cluster(
    graph: &ArenaGraph,
    node: NodeId,
) -> Option<crate::internal::owner_groups::OwnerGroupId> {
    graph
        .owner_node_group_id(node, crate::internal::owner_groups::OwnerGroupKind::Cluster)
        .expect("validated cluster reference")
}
fn cluster_nodes(
    graph: &ArenaGraph,
    group: crate::internal::owner_groups::OwnerGroupId,
) -> Vec<NodeId> {
    graph
        .owner_group_fields(group)
        .unwrap()
        .nodes
        .iter_slots()
        .map(|n| n.expect("cluster node"))
        .collect()
}
fn shared_path(graph: &ArenaGraph, id: usize, work: &Work<'_>) -> Result<bool, Error> {
    work.step()?;
    let edge = &graph.edge_records[id];
    let (group, adjacent, from) = if let Some(group) = cluster(graph, edge.from) {
        (group, edge.to, true)
    } else if let Some(group) = cluster(graph, edge.to) {
        (group, edge.from, false)
    } else {
        return Ok(false);
    };
    let points = |edge: &ArenaEdge| {
        if from {
            (
                edge.route.point(edge.route.len() - 1),
                edge.route.point(edge.route.len() - 2),
            )
        } else {
            (edge.route.point(0), edge.route.point(1))
        }
    };
    let (a, b) = points(edge);
    for node in cluster_nodes(graph, group) {
        work.step()?;
        for other in graph.nodes[node.0 as usize].edges.header().iter_slots() {
            work.step()?;
            let other = other.expect("incident edge").0 as usize;
            if other == id {
                continue;
            }
            let edge = &graph.edge_records[other];
            let target = if edge.from == node {
                edge.to
            } else {
                edge.from
            };
            if target != adjacent {
                continue;
            }
            let (c, d) = points(edge);
            if a.x == b.x
                && c.x == d.x
                && a.x == c.x
                && a.y.max(b.y) >= c.y.min(d.y)
                && c.y.max(d.y) >= a.y.min(b.y)
            {
                return Ok(true);
            }
            if a.y == b.y
                && c.y == d.y
                && a.y == c.y
                && a.x.max(b.x) >= c.x.min(d.x)
                && c.x.max(d.x) >= a.x.min(b.x)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn boundary(
    graph: &ArenaGraph,
    id: NodeId,
    nodes: &[NodeId],
    side: usize,
    work: &Work<'_>,
) -> Result<bool, Error> {
    for &other in nodes {
        work.step()?;
        if other == id {
            continue;
        }
        let Some(p) = graph.position(other) else {
            continue;
        };
        let p0 = graph.position(id).unwrap();
        let n = &graph.nodes[id.0 as usize];
        let m = &graph.nodes[other.0 as usize];
        if match side {
            0 => p.x < p0.x,
            1 => p.y < p0.y,
            2 => p.x + m.rect.size.width > p0.x + n.rect.size.width,
            _ => p.y + m.rect.size.height > p0.y + n.rect.size.height,
        } {
            return Ok(false);
        }
    }
    Ok(true)
}
fn cluster_box(
    graph: &ArenaGraph,
    id: NodeId,
    nodes: &[NodeId],
    work: &Work<'_>,
) -> Result<Rect, Error> {
    if graph.position(id).is_none() {
        return Err(invalid(format!(
            "TALA {} cluster contains an unplaced node",
            work.location
        )));
    }
    work.step()?;
    let node = &graph.nodes[id.0 as usize];
    let mut tl = graph.position(id).unwrap();
    let mut br = Point {
        x: (tl.x + node.rect.size.width).round(),
        y: (tl.y + node.rect.size.height).round(),
    };
    let (dx, dy) = crate::internal::bounds::modifier_element_adjustments(node);
    if dx != 0.0 || dy != 0.0 {
        tl.y -= dy;
        br.x += dx;
    }
    let offsets = crate::internal::loop_offsets::loop_cardinal_extents(node.loop_offsets.as_ref());
    tl.x -= offsets[3];
    tl.y -= offsets[0];
    br.x += offsets[1];
    br.y += offsets[2];
    if node.label_size.is_some() && super::node::is_outside(node.label_position) {
        let label = super::node::positioned_node_label_rect(graph, id).unwrap();
        if label.origin.x < tl.x {
            let pad = if boundary(graph, id, nodes, 0, work)? {
                5.0
            } else {
                10.0
            };
            tl.x = (label.origin.x - pad).floor();
        }
        if label.origin.y < tl.y {
            let pad = if boundary(graph, id, nodes, 1, work)? {
                5.0
            } else {
                10.0
            };
            tl.y = (label.origin.y - pad).floor();
        }
        if label.origin.x > br.x {
            let pad = if boundary(graph, id, nodes, 2, work)? {
                5.0
            } else {
                10.0
            };
            br.x = (label.right() + pad).ceil();
        }
        if label.origin.y > br.y {
            let pad = if boundary(graph, id, nodes, 3, work)? {
                5.0
            } else {
                10.0
            };
            br.y = (label.bottom() + pad).ceil();
        }
    }
    if node.has_icon
        && node.shape != ShapeKind::Image
        && node.icon_position.is_some_and(super::node::is_outside)
    {
        let icon = super::node::node_label_rect(
            graph,
            id,
            node.icon_position.unwrap(),
            Size {
                width: 64.0,
                height: 64.0,
            },
        )
        .unwrap();
        tl.x = tl.x.min((icon.origin.x - 10.0).floor());
        tl.y = tl.y.min((icon.origin.y - 10.0).floor());
        br.x = br.x.max((icon.right() + 10.0).ceil());
        br.y = br.y.max((icon.bottom() + 10.0).ceil());
    }
    Ok(Rect {
        origin: tl,
        size: Size {
            width: br.x - tl.x,
            height: br.y - tl.y,
        },
    })
}
fn positions(graph: &ArenaGraph, id: usize, work: &Work<'_>) -> Result<Vec<LabelPosition>, Error> {
    let edge = &graph.edge_records[id];
    let initial = edge.label.as_ref().unwrap().position;
    let mut positions = Vec::new();
    if is_unlocked(initial) {
        if initial == LabelPosition::UnlockedMiddle {
            positions.extend([
                initial,
                LabelPosition::UnlockedTop,
                LabelPosition::UnlockedBottom,
            ]);
        } else {
            positions.extend([initial, initial.mirrored(), LabelPosition::UnlockedMiddle]);
        }
        positions.extend_from_slice(EDGE_LABEL_PREFERENCE_ORDER);
        return Ok(positions);
    }
    let source = cluster(graph, edge.from);
    let target = cluster(graph, edge.to);
    if (source.is_some() || target.is_some()) && shared_path(graph, id, work)? {
        let (group, member, external, from) = if let Some(group) = source {
            (group, edge.from, edge.to, true)
        } else {
            (target.unwrap(), edge.to, edge.from, false)
        };
        let nodes = cluster_nodes(graph, group);
        let (mut tl, mut br) = if nodes.is_empty() {
            (
                Point {
                    x: f64::NEG_INFINITY,
                    y: f64::NEG_INFINITY,
                },
                Point {
                    x: f64::INFINITY,
                    y: f64::INFINITY,
                },
            )
        } else {
            (
                Point {
                    x: f64::INFINITY,
                    y: f64::INFINITY,
                },
                Point {
                    x: f64::NEG_INFINITY,
                    y: f64::NEG_INFINITY,
                },
            )
        };
        for &node in &nodes {
            let rect = cluster_box(graph, node, &nodes, work)?;
            tl.x = tl.x.min(rect.origin.x);
            tl.y = tl.y.min(rect.origin.y);
            br.x = br.x.max(rect.right());
            br.y = br.y.max(rect.bottom());
        }
        let value = &graph.nodes[external.0 as usize];
        let orientation = graph.sized_box_orientation(
            (graph.position(external).unwrap(), value.rect.size),
            (
                tl,
                Size {
                    width: br.x - tl.x,
                    height: br.y - tl.y,
                },
            ),
        );
        let mut index = -1;
        for (i, &node) in nodes.iter().enumerate() {
            work.step()?;
            if node == member {
                index = i as i64;
                break;
            }
        }
        let midpoint = (nodes.len() as f64 - 1.0) / 2.0;
        let before = (index as f64) < midpoint;
        let after = (index as f64) > midpoint;
        use crate::internal::Orientation;
        let forward = matches!(orientation, Orientation::Right | Orientation::Top);
        let backward = matches!(orientation, Orientation::Left | Orientation::Bottom);
        if forward || backward {
            if (from == forward && before) || (from != forward && after) {
                positions.push(LabelPosition::UnlockedTop);
            } else if (from == forward && after) || (from != forward && before) {
                positions.push(LabelPosition::UnlockedBottom);
            }
        }
        positions.push(LabelPosition::UnlockedMiddle);
        positions.extend_from_slice(EDGE_LABEL_PREFERENCE_ORDER);
    } else {
        positions.extend_from_slice(EDGE_LABEL_PREFERENCE_ORDER);
        positions.extend([
            LabelPosition::UnlockedTop,
            LabelPosition::UnlockedBottom,
            LabelPosition::UnlockedMiddle,
        ]);
    }
    Ok(positions)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Counting {
        polls: Cell<usize>,
        cancel: Cell<bool>,
    }
    impl Context for Counting {
        fn done(&self) -> Option<bool> {
            Some(false)
        }
        fn error(&self) -> Option<crate::internal::limits::ContextError> {
            self.polls.set(self.polls.get() + 1);
            self.cancel
                .get()
                .then_some(crate::internal::limits::ContextError::Cancelled)
        }
    }
    #[test]
    fn seed_nil_label_fault_retains_work_and_context_order_without_unwinding() {
        let ctx = Counting {
            polls: Cell::new(0),
            cancel: Cell::new(false),
        };
        let rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let mut work = Work::new(&ctx, 64, ALL_LOCATION).unwrap();
        work.recover_invariants = true;
        let error = boxes_count(rect, [None], 0.0, &work).unwrap_err();
        assert!(matches!(error, Error::SeedInvariant(_)));
        assert_eq!(work.used.get(), 1);
        assert_eq!(ctx.polls.get(), 1);
        let mut limited = Work::new(&ctx, 0, ALL_LOCATION).unwrap();
        limited.recover_invariants = true;
        assert!(matches!(
            boxes_count(rect, [None], 0.0, &limited),
            Err(Error::Limit { .. })
        ));
        assert_eq!(limited.used.get(), 0);
        let mut cancelled = Work::new(&ctx, 64, ALL_LOCATION).unwrap();
        cancelled.recover_invariants = true;
        cancelled.add(63).unwrap();
        ctx.cancel.set(true);
        assert!(matches!(
            boxes_count(rect, [None], 0.0, &cancelled),
            Err(Error::Context { .. })
        ));
        assert_eq!(cancelled.used.get(), 64);
    }
    #[test]
    fn standalone_nil_label_fault_still_panics_after_its_work_step() {
        let ctx = Counting {
            polls: Cell::new(0),
            cancel: Cell::new(false),
        };
        let work = Work::new(&ctx, 64, ALL_LOCATION).unwrap();
        let rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            boxes_count(rect, [None], 0.0, &work)
        }));
        assert!(result.is_err());
        assert_eq!(work.used.get(), 1);
    }
    #[test]
    fn label_work_checks_open_context_at_stride_and_rejects_without_charging() {
        let ctx = Counting {
            polls: Cell::new(0),
            cancel: Cell::new(false),
        };
        let work = Work::new(&ctx, 64, NEW_LOCATION).unwrap();
        work.add(63).unwrap();
        assert_eq!(ctx.polls.get(), 1);
        work.step().unwrap();
        assert_eq!(ctx.polls.get(), 2);
        ctx.cancel.set(true);
        assert!(matches!(work.step(), Err(Error::Context { .. })));
        assert_eq!(work.used.get(), 64);
        assert_eq!(ctx.polls.get(), 3);
    }
    #[test]
    fn label_sort_keeps_equal_lengths_and_charges_every_merge_output() {
        let ctx = Counting {
            polls: Cell::new(0),
            cancel: Cell::new(false),
        };
        let work = Work::new(&ctx, 100, NEW_LOCATION).unwrap();
        let mut items = vec![(2, 0), (1, 1), (2, 2), (1, 3), (2, 4)];
        stable_sort(&mut items, |a, b| a.0 < b.0, &work).unwrap();
        assert_eq!(items, vec![(1, 1), (1, 3), (2, 0), (2, 2), (2, 4)]);
        assert_eq!(work.used.get(), 15);
    }
}
