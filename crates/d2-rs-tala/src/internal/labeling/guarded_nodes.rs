// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Node/icon portion of labeling.place, preserving work and publication order.
use super::*;

fn node_rect(graph: &ArenaGraph, id: NodeId) -> Rect {
    Rect {
        origin: graph
            .position(id)
            .expect("runtime error: invalid memory address or nil pointer dereference"),
        size: graph.nodes[id.0 as usize].rect.size,
    }
}
fn ancestors(graph: &ArenaGraph, id: NodeId, work: &Work<'_>) -> Result<Vec<NodeId>, Error> {
    let mut result = Vec::new();
    let mut current = Some(id);
    for depth in 0.. {
        let Some(id) = current else { break };
        if depth >= 256 {
            return Err(invalid(format!(
                "TALA {} container depth exceeds limit 256",
                work.location
            )));
        }
        work.step()?;
        result.push(id);
        current = crate::internal::owner_placement::owner_owning_container(graph, id)
            .expect("validated container ownership");
    }
    Ok(result)
}
fn node_count(
    graph: &ArenaGraph,
    rect: Rect,
    nodes: &[NodeId],
    padding: f64,
    partial: bool,
    work: &Work<'_>,
) -> Result<usize, Error> {
    let mut result = 0;
    for &id in nodes {
        work.step()?;
        let other = node_rect(graph, id);
        if overlaps(rect, other, padding) && (!partial || !covers(other, rect)) {
            result += 1;
        }
    }
    Ok(result)
}
fn icon_box(graph: &ArenaGraph, id: NodeId, position: LabelPosition) -> Rect {
    let side = node::icon_size(graph, id, position);
    node::node_label_rect(
        graph,
        id,
        position,
        Size {
            width: side,
            height: side,
        },
    )
    .expect("runtime error: invalid memory address or nil pointer dereference")
}
fn label_candidate(
    graph: &ArenaGraph,
    id: NodeId,
    position: LabelPosition,
    size: Size,
) -> Option<Rect> {
    let rect = node::node_label_rect(graph, id, position, size)
        .expect("runtime error: invalid memory address or nil pointer dereference");
    if node::is_outside(position) {
        return Some(rect);
    }
    let padded = node::padded(rect, 5.0);
    if !covers(
        graph.shape_inner_box(id).expect("validated position"),
        padded,
    ) {
        return None;
    }
    // Go mutates the temporary box twice. Preserve the second operation's
    // rounding rather than substituting the original unpadded value.
    Some(node::padded(padded, -5.0))
}
fn label_score(
    graph: &ArenaGraph,
    rect: Rect,
    siblings: &[NodeId],
    icon: Option<Rect>,
    ancestors: &[NodeId],
    placed: &[Option<Rect>],
    work: &Work<'_>,
) -> Result<usize, Error> {
    let mut siblings_count = node_count(graph, rect, siblings, 5.0, false, work)?;
    siblings_count += boxes_count(rect, icon.into_iter().map(Some), 5.0, work)?;
    let ancestors_count = node_count(graph, rect, ancestors, 5.0, true, work)?;
    let edges = edge_count(
        graph,
        rect,
        &graph.member_edge_indices().collect::<Vec<_>>(),
        4.0,
        work,
    )?;
    let labels = boxes_count(rect, placed.iter().copied(), 5.0, work)?;
    Ok(siblings_count + ancestors_count + edges + labels * 2)
}

pub(super) fn place(
    graph: &mut ArenaGraph,
    placed: &mut Vec<Option<Rect>>,
    work: &Work<'_>,
) -> Result<(), Error> {
    for id in graph.graph_node_order() {
        work.step()?;
        let mut siblings = Vec::new();
        let container = graph.topology_node(id).unwrap().container;
        let containers = graph.owner_container_map(OwnerGraphId(0)).unwrap();
        if let Some(header) = containers.get(container) {
            for other in (0..header.len()).map(|index| header.get(index).unwrap()) {
                work.step()?;
                let other = other
                    .expect("runtime error: invalid memory address or nil pointer dereference");
                if other != id {
                    siblings.push(other);
                }
            }
        }
        if graph.nodes[id.0 as usize].is_container
            && let Some(header) = containers.get(Some(id))
        {
            for child in (0..header.len()).map(|index| header.get(index).unwrap()) {
                work.step()?;
                siblings
                    .push(child.expect(
                        "runtime error: invalid memory address or nil pointer dereference",
                    ));
            }
        }
        let ancestors = ancestors(graph, id, work)?;
        let mut best_icon_position = LabelPosition::Unset;
        let mut best_box = None;
        let mut best_score = f64::INFINITY;
        if graph.nodes[id.0 as usize].has_icon
            && graph.nodes[id.0 as usize].shape != ShapeKind::Image
        {
            if !graph.nodes[id.0 as usize].icon_position_fixed {
                let value = &graph.nodes[id.0 as usize];
                let preferences = node::preference_tranches(value.shape, value.is_container);
                for position in preferences.into_iter().flatten() {
                    work.step()?;
                    let rect = icon_box(graph, id, position);
                    let value = &graph.nodes[id.0 as usize];
                    let fixed = if value.label_position_fixed {
                        value.label_size.map(|size| {
                            node::node_label_rect(graph, id, value.label_position, size)
                                .expect("validated position")
                        })
                    } else {
                        None
                    };
                    let nodes = node_count(graph, rect, &siblings, 0.0, false, work)?
                        + boxes_count(rect, fixed.into_iter().map(Some), 0.0, work)?;
                    let edges = edge_count(
                        graph,
                        rect,
                        &graph.member_edge_indices().collect::<Vec<_>>(),
                        0.0,
                        work,
                    )?;
                    let labels = boxes_count(rect, placed.iter().copied(), 0.0, work)?;
                    let score = (nodes + edges + 2 * labels) as f64;
                    if score < best_score {
                        best_score = score;
                        best_icon_position = position;
                        best_box = Some(rect);
                        if score == 0.0 {
                            break;
                        }
                    }
                }
            } else {
                best_icon_position = graph.nodes[id.0 as usize]
                    .icon_position
                    .unwrap_or(LabelPosition::Unset);
                best_box = Some(icon_box(graph, id, best_icon_position));
            }
            work.step()?;
            placed.push(best_box);
            graph.nodes[id.0 as usize].icon_position = Some(best_icon_position);
        }
        let value = &graph.nodes[id.0 as usize];
        let Some(size) = value.label_size else {
            continue;
        };
        if value.label_position_fixed {
            continue;
        }
        let icon = (value.has_icon && value.shape != ShapeKind::Image).then(|| {
            icon_box(
                graph,
                id,
                value.icon_position.unwrap_or(LabelPosition::Unset),
            )
        });
        let shape = value.shape;
        let tranches = node::preference_tranches(shape, value.is_container);
        best_score = f64::INFINITY;
        let mut best_position = LabelPosition::Unset;
        for tranche in tranches {
            work.step()?;
            let mut tied = Vec::new();
            for position in tranche {
                work.step()?;
                if position == best_icon_position {
                    continue;
                }
                let Some(rect) = label_candidate(graph, id, position, size) else {
                    continue;
                };
                let score =
                    label_score(graph, rect, &siblings, icon, &ancestors, placed, work)? as f64;
                #[cfg(feature = "diagnostic-traces")]
                node::trace_label_node_candidate(
                    "candidate",
                    graph,
                    id,
                    position,
                    rect,
                    score as usize,
                );
                if score < best_score {
                    best_score = score;
                    best_position = position;
                    best_box = Some(rect);
                    tied.clear();
                    if node::is_outside(position) {
                        tied.push((position, rect));
                    }
                } else if score == best_score
                    && node::label_preference_score(shape, position)
                        == node::label_preference_score(shape, best_position)
                    && node::is_outside(position)
                    && node::is_outside(best_position)
                {
                    tied.push((position, rect));
                }
            }
            if tied.len() > 1 {
                let mut tie_score = f64::INFINITY;
                let mut tie_position = LabelPosition::Unset;
                let mut tie_box = None;
                for (position, original) in tied {
                    work.step()?;
                    let Some(rect) = label_candidate(
                        graph,
                        id,
                        position,
                        Size {
                            width: size.width * 2.0,
                            height: size.height * 2.0,
                        },
                    ) else {
                        continue;
                    };
                    let score =
                        label_score(graph, rect, &siblings, icon, &ancestors, placed, work)? as f64;
                    #[cfg(feature = "diagnostic-traces")]
                    node::trace_label_node_candidate(
                        "tiebreak",
                        graph,
                        id,
                        position,
                        rect,
                        score as usize,
                    );
                    if score < tie_score {
                        tie_score = score;
                        tie_position = position;
                        tie_box = Some(original);
                    }
                }
                best_position = tie_position;
                best_box = tie_box;
            }
        }
        work.step()?;
        placed.push(best_box);
        #[cfg(feature = "diagnostic-traces")]
        if let Some(rect) = best_box {
            node::trace_label_node_candidate(
                "selected",
                graph,
                id,
                best_position,
                rect,
                best_score as usize,
            );
        }
        graph.nodes[id.0 as usize].label_position = best_position;
    }
    Ok(())
}
