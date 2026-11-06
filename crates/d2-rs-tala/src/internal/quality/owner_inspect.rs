// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! quality/inspection*.go over actual owner records and live point allocations.
use super::*;
use crate::internal::owner_groups::OwnerGroupKind;
use crate::internal::{ArenaGraph, OwnerGraphId, labels, limits};
use crate::{EdgeId, LabelPosition, NodeId, ShapeKind};
use limits::{Context, Error, WorkGuard};
use std::collections::BTreeSet;

pub(in crate::internal) fn owner_ancestry_parent(
    graph: &ArenaGraph,
    node: NodeId,
) -> Option<NodeId> {
    if let Some(container) = graph.nodes[node.0 as usize].container {
        return Some(container);
    }
    for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
        if let Some(vessel) = graph
            .owner_node_group_vessel(node, kind)
            .expect("validated node group")
        {
            return vessel;
        }
    }
    None
}
fn invalid(message: String) -> Error {
    Error::Invalid(message)
}
fn context_check(context: &dyn Context, location: &str) -> Result<(), Error> {
    context.error().map_or(Ok(()), |cause| {
        Err(Error::Context {
            location: location.into(),
            cause,
        })
    })
}
fn validate_positioned(
    graph: &ArenaGraph,
    receiver: OwnerGraphId,
    context: &dyn Context,
) -> Result<(), Error> {
    let location = "Inspect";
    graph.validate_owner_graph_topology(Some(receiver), Some(context), location)?;
    context_check(context, location)?;
    let view = graph.owner_graph_view(receiver).unwrap();
    if view.nodes.len() > 10_000 {
        return Err(invalid(
            "TALA Inspect node count exceeds limit 10000".into(),
        ));
    }
    if view.edges.len() > 50_000 {
        return Err(invalid(
            "TALA Inspect edge count exceeds limit 50000".into(),
        ));
    }
    let mut checks = 0usize;
    let mut check = || {
        checks += 1;
        if checks.is_multiple_of(64) {
            context_check(context, location)
        } else {
            Ok(())
        }
    };
    for (index, node) in view.nodes.iter_slots().enumerate() {
        check()?;
        let node = node
            .ok_or_else(|| invalid(format!("TALA Inspect graph node at index {index} is nil")))?;
        let id = graph.nodes[node.0 as usize].tala_id;
        if graph.position(node).is_none() {
            return Err(invalid(format!(
                "TALA Inspect graph node {id} has no position"
            )));
        }
        for (kind, name) in [
            (OwnerGroupKind::Cluster, "cluster"),
            (OwnerGroupKind::Sequence, "sequence"),
        ] {
            if graph.owner_node_group_vessel(node, kind).unwrap() == Some(None) {
                return Err(invalid(format!(
                    "TALA Inspect graph node {id} has a {name} without a vessel"
                )));
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut route_points = 0;
    for (index, edge) in view.edges.header().iter_slots().enumerate() {
        check()?;
        let edge = edge
            .ok_or_else(|| invalid(format!("TALA Inspect graph edge at index {index} is nil")))?;
        let e = &graph.edge_records[edge.0 as usize];
        let id = e.tala_id;
        if !seen.insert(edge) {
            return Err(invalid(format!(
                "TALA Inspect graph edge at index {index} repeats edge {id}"
            )));
        }
        if e.route.len() > 1_000_000 - route_points {
            return Err(invalid(
                "TALA Inspect route point count exceeds limit 1000000".into(),
            ));
        }
        route_points += e.route.len();
        for point in 0..e.route.len() {
            check()?;
            if e.route.slot(point).is_none() {
                return Err(invalid(format!(
                    "TALA Inspect edge {id} route point at index {point} is nil"
                )));
            }
        }
        if (e.label.is_some()
            || e.source_arrowhead_label.is_some()
            || e.target_arrowhead_label.is_some())
            && e.route.len() < 2
        {
            return Err(invalid(format!(
                "TALA Inspect labeled edge {id} requires at least two route points"
            )));
        }
    }
    context_check(context, location)
}
fn rectangle(graph: &ArenaGraph, node: NodeId) -> Rect {
    Rect {
        origin: graph
            .position(node)
            .expect("runtime error: invalid memory address or nil pointer dereference"),
        size: graph.nodes[node.0 as usize].rect.size,
    }
}
fn rectangular(graph: &ArenaGraph, node: NodeId) -> bool {
    matches!(
        graph.nodes[node.0 as usize].shape,
        ShapeKind::Rectangle
            | ShapeKind::Square
            | ShapeKind::RealSquare
            | ShapeKind::Image
            | ShapeKind::Text
            | ShapeKind::Code
            | ShapeKind::Class
            | ShapeKind::SqlTable
    )
}
fn ancestor(
    graph: &ArenaGraph,
    mut node: Option<NodeId>,
    ancestor: Option<NodeId>,
    work: &mut WorkGuard<'_>,
) -> Result<bool, Error> {
    for depth in 0.. {
        work.step()?;
        if node == ancestor {
            return Ok(true);
        }
        let Some(current) = node else {
            return Ok(false);
        };
        if depth >= 256 {
            return Err(invalid(
                "TALA Evaluate ancestry depth exceeds limit 256".into(),
            ));
        }
        node = owner_ancestry_parent(graph, current);
        if node.is_none() {
            return Ok(ancestor.is_none());
        }
    }
    unreachable!()
}
fn endpoint_ancestor(
    graph: &ArenaGraph,
    edge: EdgeId,
    node: NodeId,
    work: &mut WorkGuard<'_>,
) -> Result<bool, Error> {
    let e = &graph.edge_records[edge.0 as usize];
    if ancestor(graph, Some(e.from), Some(node), work)? {
        return Ok(true);
    }
    ancestor(graph, Some(e.to), Some(node), work)
}
struct PlacedLabel {
    rect: Rect,
    node: Option<NodeId>,
    edge: Option<EdgeId>,
}
fn measure_labels(
    graph: &ArenaGraph,
    receiver: OwnerGraphId,
    score: &mut Metrics,
    work: &mut WorkGuard<'_>,
) -> Result<(), Error> {
    work.step()?;
    let view = graph.owner_graph_view(receiver).unwrap();
    let mut positioned = Vec::new();
    let mut append = |rect: Option<Rect>, node, edge| {
        if let Some(rect) = rect
            && rect.size.width > 0.0
            && rect.size.height > 0.0
        {
            positioned.push(PlacedLabel { rect, node, edge });
        }
    };
    for node in view.nodes.require_nodes() {
        work.step()?;
        let n = &graph.nodes[node.0 as usize];
        if n.is_invisible {
            continue;
        }
        if n.label_size.is_some() && n.label_position != LabelPosition::Unset {
            if n.label_position.is_outside() {
                work.add((4 * view.nodes.len()) as i64)?;
            }
            append(
                labels::positioned_node_label_rect(graph, node),
                Some(node),
                None,
            );
        }
        if n.has_icon && n.shape != ShapeKind::Image {
            if n.icon_position.is_some_and(|p| p.is_outside()) {
                work.add((4 * view.nodes.len()) as i64)?;
            }
            append(
                labels::positioned_icon_label_rect(graph, node),
                Some(node),
                None,
            );
        }
    }
    let mut visible = Vec::new();
    for edge in view.edges.header().iter_slots().map(Option::unwrap) {
        work.step()?;
        let e = &graph.edge_records[edge.0 as usize];
        if e.is_invisible() {
            continue;
        }
        visible.push(edge);
        if e.route.len() < 2 {
            continue;
        }
        if let Some(label) = &e.label
            && label.position != LabelPosition::Unset
        {
            work.add(e.route.len() as i64)?;
            append(
                labels::label_rect(e, label.position, label.percentage),
                None,
                Some(edge),
            );
        }
        for target in [false, true] {
            if (if target {
                &e.target_arrowhead_label
            } else {
                &e.source_arrowhead_label
            })
            .is_some()
            {
                work.add(e.route.len() as i64)?;
                append(
                    labels::arrowhead_label_rect_for_route(e, &e.route.read(), target),
                    None,
                    Some(edge),
                );
            }
        }
    }
    for (index, placed) in positioned.iter().enumerate() {
        work.step()?;
        for other in &positioned[..index] {
            work.step()?;
            if overlaps(placed.rect, other.rect) {
                score.text_occlusions += 1;
            }
        }
        for node in view.nodes.require_nodes() {
            work.step()?;
            let n = &graph.nodes[node.0 as usize];
            if n.is_invisible
                || !rectangular(graph, node)
                || !overlaps(placed.rect, rectangle(graph, node))
            {
                continue;
            }
            let related = if let Some(label_node) = placed.node {
                ancestor(graph, Some(label_node), Some(node), work)?
            } else {
                let edge = placed.edge.unwrap();
                let related = endpoint_ancestor(graph, edge, node, work)?;
                let e = &graph.edge_records[edge.0 as usize];
                related && node != e.from && node != e.to
            };
            if related && covers(rectangle(graph, node), placed.rect) {
                continue;
            }
            score.text_occlusions += 1;
        }
        for &edge in &visible {
            work.step()?;
            let e = &graph.edge_records[edge.0 as usize];
            if Some(edge) == placed.edge || e.is_curve {
                continue;
            }
            for j in 1..e.route.len() {
                work.step()?;
                if enters(placed.rect, e.route.point(j - 1), e.route.point(j)) {
                    score.text_occlusions += 1;
                    break;
                }
            }
        }
    }
    Ok(())
}
pub(in crate::internal) fn inspect_owner(
    graph: &ArenaGraph,
    receiver: OwnerGraphId,
    context: &dyn Context,
) -> Result<Metrics, Error> {
    validate_positioned(graph, receiver, context)?;
    let mut work = WorkGuard::new(Some(context), "Inspect", 50_000_000)?;
    let view = graph.owner_graph_view(receiver).unwrap();
    let mut score = Metrics::default();
    let (mut scale, mut count) = (0.0, 0usize);
    for node in view.nodes.require_nodes() {
        work.step()?;
        let n = &graph.nodes[node.0 as usize];
        if !n.is_container && !n.is_invisible {
            scale += (n.rect.size.width + n.rect.size.height) / 2.0;
            count += 1;
        }
    }
    scale = maximum(1.0, scale / count.max(1) as f64);
    let mut visible = Vec::new();
    for edge in view.edges.header().iter_slots().map(Option::unwrap) {
        work.step()?;
        let e = &graph.edge_records[edge.0 as usize];
        if e.is_invisible() {
            continue;
        }
        visible.push(edge);
        let mut length = 0.0;
        for i in 1..e.route.len() {
            work.step()?;
            let a = e.route.point(i - 1);
            let b = e.route.point(i);
            length += go_hypot(b.x - a.x, b.y - a.y);
        }
        score.route_length += length / scale;
        if e.route.len() < 2 || e.from == e.to {
            continue;
        }
        let a = e.route.point(0);
        let b = e.route.point(e.route.len() - 1);
        let direct = (b.x - a.x).abs() + (b.y - a.y).abs();
        score.detour += maximum(0.0, length - direct) / maximum(scale, direct);
    }
    let mut crossings = 0i64;
    for (i, &edge) in visible.iter().enumerate() {
        work.step()?;
        let e = &graph.edge_records[edge.0 as usize];
        for &other in &visible[i + 1..] {
            work.step()?;
            let other = &graph.edge_records[other.0 as usize];
            for i in 1..e.route.len() {
                work.step()?;
                for j in 1..other.route.len() {
                    work.step()?;
                    if crossing(
                        e.route.point(i - 1),
                        e.route.point(i),
                        other.route.point(j - 1),
                        other.route.point(j),
                    ) {
                        crossings += 1;
                    }
                }
            }
        }
    }
    score.crossings = crossings as f64;
    for (index, node) in view.nodes.require_nodes().enumerate() {
        work.step()?;
        let n = &graph.nodes[node.0 as usize];
        if n.is_invisible || !rectangular(graph, node) {
            continue;
        }
        for other in view.nodes.require_nodes().skip(index + 1) {
            work.step()?;
            if graph.nodes[other.0 as usize].is_invisible
                || !rectangular(graph, other)
                || !overlaps(rectangle(graph, node), rectangle(graph, other))
            {
                continue;
            }
            if !ancestor(graph, Some(node), Some(other), &mut work)?
                && !ancestor(graph, Some(other), Some(node), &mut work)?
            {
                score.node_overlaps += 1;
            }
        }
        for edge in view.edges.header().iter_slots().map(Option::unwrap) {
            work.step()?;
            let e = &graph.edge_records[edge.0 as usize];
            if e.is_invisible() || e.is_curve {
                continue;
            }
            if endpoint_ancestor(graph, edge, node, &mut work)? && e.from != node && e.to != node {
                continue;
            }
            for i in 1..e.route.len() {
                work.step()?;
                if i == 1 && e.from == node || i == e.route.len() - 1 && e.to == node {
                    continue;
                }
                if enters(
                    rectangle(graph, node),
                    e.route.point(i - 1),
                    e.route.point(i),
                ) {
                    score.route_obstructions += 1;
                    break;
                }
            }
        }
    }
    measure_labels(graph, receiver, &mut score, &mut work)?;
    if !score.route_length.is_finite() || !score.detour.is_finite() {
        return Err(invalid("TALA Inspect produced non-finite geometry".into()));
    }
    work.finish()?;
    Ok(score)
}
