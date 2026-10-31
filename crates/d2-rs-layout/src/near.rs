// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Constant-near placement corresponding to D2's `d2near` package.

use d2_rs_core::{CanvasPosition, Graph, LayoutError, LayoutResult, NodeId, Point, Rect};
use std::collections::{BTreeMap, BTreeSet};

/// Clearance between a near object and its reference object's bounds.
pub const PAD: f64 = 20.0;

/// Applies node-to-node near constraints after the ordinary layout pass.
pub fn apply(graph: &Graph, mut result: LayoutResult) -> Result<LayoutResult, LayoutError> {
    #[cfg(feature = "diagnostic-traces")]
    if crate::diagnostic_env_enabled("DAGRE_DEBUG_NEAR") {
        for (id, node) in graph.nodes() {
            eprintln!(
                "NEAR_NODE {:?} external={} near={:?} canvas={:?} parent={:?} box={:?}",
                id,
                node.external_id,
                node.near,
                node.canvas_position,
                node.parent,
                result.boxes.get(&id)
            );
        }
    }
    let mut visiting = BTreeSet::new();
    let mut done = BTreeSet::new();
    let near_nodes = graph
        .nodes()
        .filter_map(|(id, node)| node.near.map(|target| (id, target)))
        .collect::<Vec<_>>();
    for (id, _) in near_nodes {
        place_one(id, graph, &mut result.boxes, &mut visiting, &mut done)?;
    }
    // A near move changes only endpoint coordinates. Rebuild routes in stable
    // input order so callers never retain a route attached to the old box.
    for (id, edge) in graph.edges() {
        let Some(source) = result.boxes.get(&edge.source).copied() else {
            continue;
        };
        let Some(target) = result.boxes.get(&edge.target).copied() else {
            continue;
        };
        result
            .routes
            .insert(id, vec![source.center(), target.center()]);
    }
    Ok(result)
}

fn place_one(
    id: NodeId,
    graph: &Graph,
    boxes: &mut BTreeMap<NodeId, Rect>,
    visiting: &mut BTreeSet<NodeId>,
    done: &mut BTreeSet<NodeId>,
) -> Result<(), LayoutError> {
    if done.contains(&id) {
        return Ok(());
    }
    if !visiting.insert(id) {
        return Err(LayoutError::NearCycle { node: id });
    }
    let node = graph.node(id).ok_or(LayoutError::IdOverflow)?;
    let target_id = node
        .near
        .ok_or(LayoutError::MissingNear { node: id, near: id })?;
    if graph.node(target_id).is_none() {
        return Err(LayoutError::MissingNear {
            node: id,
            near: target_id,
        });
    }
    if graph
        .node(target_id)
        .and_then(|target| target.near)
        .is_some()
    {
        place_one(target_id, graph, boxes, visiting, done)?;
    }
    let target = boxes.get(&target_id).copied().unwrap_or_default();
    let position = node.canvas_position.unwrap_or(CanvasPosition::TopRight);
    let origin = near_origin(position, target, node.size);
    boxes.insert(
        id,
        Rect {
            origin,
            size: node.size,
        },
    );
    visiting.remove(&id);
    done.insert(id);
    Ok(())
}

fn near_origin(position: CanvasPosition, target: Rect, size: d2_rs_core::Size) -> Point {
    match position {
        CanvasPosition::TopLeft => Point {
            x: target.origin.x - size.width - PAD,
            y: target.origin.y - size.height - PAD,
        },
        CanvasPosition::TopCenter => Point {
            x: target.center().x - size.width / 2.0,
            y: target.origin.y - size.height - PAD,
        },
        CanvasPosition::TopRight => Point {
            x: target.right() + PAD,
            y: target.origin.y - size.height - PAD,
        },
        CanvasPosition::CenterLeft => Point {
            x: target.origin.x - size.width - PAD,
            y: target.center().y - size.height / 2.0,
        },
        CanvasPosition::CenterRight => Point {
            x: target.right() + PAD,
            y: target.center().y - size.height / 2.0,
        },
        CanvasPosition::BottomLeft => Point {
            x: target.origin.x - size.width - PAD,
            y: target.bottom() + PAD,
        },
        CanvasPosition::BottomCenter => Point {
            x: target.center().x - size.width / 2.0,
            y: target.bottom() + PAD,
        },
        CanvasPosition::BottomRight => Point {
            x: target.right() + PAD,
            y: target.bottom() + PAD,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::apply;
    use d2_rs_core::{
        CanvasPosition, Graph, LayoutReport, LayoutResult, Node, Point, Rect, Score, Size,
    };
    use std::collections::BTreeMap;

    #[test]
    fn near_places_a_node_relative_to_its_target() {
        let mut graph = Graph::default();
        let target = graph.add_node(Node::new(
            "target",
            Size {
                width: 40.0,
                height: 20.0,
            },
        ));
        let mut near = Node::new(
            "near",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        near.near = Some(target);
        near.canvas_position = Some(CanvasPosition::BottomRight);
        graph.add_node(near);
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    target,
                    Rect {
                        origin: Point { x: 0.0, y: 0.0 },
                        size: Size {
                            width: 40.0,
                            height: 20.0,
                        },
                    },
                ),
                (
                    d2_rs_core::NodeId(1),
                    Rect {
                        origin: Point::default(),
                        size: Size {
                            width: 10.0,
                            height: 10.0,
                        },
                    },
                ),
            ]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        let result = apply(&graph, result).unwrap();
        assert_eq!(
            result.boxes[&d2_rs_core::NodeId(1)].origin,
            Point { x: 60.0, y: 40.0 }
        );
    }

    #[test]
    fn near_cycles_are_reported_before_mutating_results() {
        let mut graph = Graph::default();
        let first = graph.add_node(Node::new(
            "first",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let second = graph.add_node(Node::new(
            "second",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        graph.node_mut(first).unwrap().near = Some(second);
        graph.node_mut(second).unwrap().near = Some(first);
        let result = LayoutResult {
            removed_edges: Default::default(),
            source_edge_storage: Default::default(),
            synthetic_root: None,
            route_curves: Default::default(),
            sequence_lifelines: Vec::new(),
            edge_label_percentage_commits: Default::default(),
            boxes: BTreeMap::from([
                (
                    first,
                    Rect {
                        origin: Point::default(),
                        size: Size {
                            width: 10.0,
                            height: 10.0,
                        },
                    },
                ),
                (
                    second,
                    Rect {
                        origin: Point::default(),
                        size: Size {
                            width: 10.0,
                            height: 10.0,
                        },
                    },
                ),
            ]),
            routes: BTreeMap::new(),
            edge_labels: BTreeMap::new(),
            node_labels: BTreeMap::new(),
            report: LayoutReport {
                selected_seed: 1,
                selected_pass: 0,
                score: Score::default(),
                improvement_passes: 0,
                candidates: Vec::new(),
                warnings: Vec::new(),
            },
        };
        assert!(matches!(
            apply(&graph, result),
            Err(d2_rs_core::LayoutError::NearCycle { .. })
        ));
    }
}
