// SPDX-License-Identifier: MPL-2.0
//! d2graph.Edge.TraceToShape and the ELK adapter's temporary modifier boxes.
use d2_rs_core::shape_geometry::{box_intersections, trace_shape_border};
use d2_rs_core::{Graph, LabelPosition, LayoutResult, NodeId, Point, Rect, ShapeKind, Size};

#[derive(Clone, Copy)]
struct Endpoint {
    rect: Rect,
    shape: ShapeKind,
    label: Option<(LabelPosition, Rect)>,
    icon: Option<(LabelPosition, Rect)>,
}

fn outside_box(rect: Rect, position: LabelPosition, size: Size) -> Option<Rect> {
    use LabelPosition::*;
    let mut p = rect.origin;
    // Keep GetPointOnBox's operation order, including origin + (width - ...).
    match position {
        OutsideTopLeft => {
            p.x -= 5.0;
            p.y -= 5.0 + size.height;
        }
        OutsideTopCenter => {
            p.x = rect.center().x - size.width / 2.0;
            p.y -= 5.0 + size.height;
        }
        OutsideTopRight => {
            p.x += rect.size.width - size.width - 5.0;
            p.y -= 5.0 + size.height;
        }
        OutsideLeftTop => {
            p.x -= 5.0 + size.width;
            p.y += 5.0;
        }
        OutsideLeftMiddle => {
            p.x -= 5.0 + size.width;
            p.y = rect.center().y - size.height / 2.0;
        }
        OutsideLeftBottom => {
            p.x -= 5.0 + size.width;
            p.y += rect.size.height - size.height - 5.0;
        }
        OutsideRightTop => {
            p.x += rect.size.width + 5.0;
            p.y += 5.0;
        }
        OutsideRightMiddle => {
            p.x += rect.size.width + 5.0;
            p.y = rect.center().y - size.height / 2.0;
        }
        OutsideRightBottom => {
            p.x += rect.size.width + 5.0;
            p.y += rect.size.height - size.height - 5.0;
        }
        OutsideBottomLeft => {
            p.x += 5.0;
            p.y += rect.size.height + 5.0;
        }
        OutsideBottomCenter => {
            p.x = rect.center().x - size.width / 2.0;
            p.y += rect.size.height + 5.0;
        }
        OutsideBottomRight => {
            p.x += rect.size.width - size.width - 5.0;
            p.y += rect.size.height + 5.0;
        }
        _ => return None,
    }
    Some(Rect { origin: p, size })
}

fn endpoint(graph: &Graph, result: &LayoutResult, id: NodeId, source: bool) -> Endpoint {
    let n = graph.node(id).unwrap();
    let rect = result.boxes[&id];
    let state = result.node_labels.get(&id);
    let position = state.map_or(n.label_position, |s| s.position);
    let label_size = state.and_then(|s| s.size).or(n.label_size);
    let label = if !matches!(
        n.shape,
        ShapeKind::Text | ShapeKind::Code | ShapeKind::Class | ShapeKind::SqlTable
    ) && graph.node_labels.get(&id).is_some_and(|s| !s.is_empty())
    {
        label_size
            .and_then(|size| outside_box(rect, position, size))
            .map(|mut b| {
                b.origin.x -= 5.0;
                b.size.width += 10.0;
                (position, b)
            })
    } else {
        None
    };
    let icon_position = state.and_then(|s| s.icon_position).or(n.icon_position);
    let icon = if n.has_icon && n.shape != ShapeKind::Image {
        icon_position.and_then(|p| {
            let min_dimension = rect.size.width.min(rect.size.height).trunc();
            let size = if source {
                64.0
            } else {
                min_dimension
                    .min(32.0f64.max((0.5 * min_dimension).ceil()))
                    .min(64.0)
            };
            outside_box(
                rect,
                p,
                Size {
                    width: size,
                    height: size,
                },
            )
            .map(|b| (p, b))
        })
    } else {
        None
    };
    Endpoint {
        rect,
        shape: n.shape,
        label,
        icon,
    }
}

fn contains(b: Rect, p: Point) -> bool {
    !(p.x < b.origin.x
        || b.origin.x + b.size.width < p.x
        || p.y < b.origin.y
        || b.origin.y + b.size.height < p.y)
}
fn length(a: Point, b: Point) -> f64 {
    if a.x == b.x {
        (a.y - b.y).abs()
    } else if a.y == b.y {
        (a.x - b.x).abs()
    } else {
        let x = a.x - b.x;
        let y = a.y - b.y;
        (x * x + y * y).sqrt()
    }
}
fn outer(position: LabelPosition, b: Rect, a: Point, z: Point) -> Option<Point> {
    use LabelPosition::*;
    let mut points = box_intersections(b, a, z);
    points.sort_by(|a, b| {
        let (a, b) = match position {
            OutsideTopLeft | OutsideTopCenter | OutsideTopRight => (a.y, b.y),
            OutsideBottomLeft | OutsideBottomCenter | OutsideBottomRight => (b.y, a.y),
            OutsideLeftTop | OutsideLeftMiddle | OutsideLeftBottom => (a.x, b.x),
            OutsideRightTop | OutsideRightMiddle | OutsideRightBottom => (b.x, a.x),
            _ => (0.0, 0.0),
        };
        a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
    });
    points.first().copied()
}

fn trace_route(mut points: Vec<Point>, source: Endpoint, target: Endpoint) -> Vec<Point> {
    if points.len() < 2 {
        return points;
    }
    let mut start = 0;
    let mut end = points.len() - 1;
    let (a, mut b) = (points[1], points[0]);
    let mut intercepted = false;
    for decoration in [source.label, source.icon].into_iter().flatten() {
        // Source's original containment predicate is start+1 > end. It
        // cannot traverse a valid route, unlike the target's < equivalent.
        if let Some(p) = outer(decoration.0, decoration.1, a, b) {
            points[start] = p;
            b = p;
            intercepted = true;
            if start + 1 < end && length(a, b) < 10.0 {
                points[start + 1] = points[start];
                start += 1;
            }
            break;
        }
    }
    if !intercepted {
        if let Some(p) = box_intersections(source.rect, a, b).first().copied() {
            points[start] = p;
            b = p;
            if start + 1 < end && length(a, b) < 10.0 {
                points[start + 1] = points[start];
                start += 1;
            }
        }
        points[start] =
            trace_shape_border(source.shape, source.rect, points[start], points[start + 1]);
    }
    let (mut a, mut b) = (points[end - 1], points[end]);
    intercepted = false;
    for decoration in [target.label, target.icon].into_iter().flatten() {
        while contains(decoration.1, a) && end - 1 > start {
            b = a;
            a = points[end - 2];
            end -= 1;
        }
        if let Some(p) = outer(decoration.0, decoration.1, a, b) {
            points[end] = p;
            b = p;
            intercepted = true;
            if end - 1 > start && length(a, b) < 10.0 {
                points[end - 1] = points[end];
                end -= 1;
            }
            break;
        }
    }
    if !intercepted {
        if let Some(p) = box_intersections(target.rect, a, b).first().copied() {
            points[end] = p;
            b = p;
            if end - 1 > start && length(a, b) < 10.0 {
                points[end - 1] = points[end];
                end -= 1;
            }
        }
        points[end] = trace_shape_border(target.shape, target.rect, points[end], points[end - 1]);
    }
    points[start..=end].to_vec()
}

pub(super) fn trace_routes(graph: &Graph, result: &mut LayoutResult) {
    for (id, e) in graph.edges() {
        let Some(points) = result.routes.get(&id).filter(|p| p.len() >= 2).cloned() else {
            continue;
        };
        if !result.boxes.contains_key(&e.source) || !result.boxes.contains_key(&e.target) {
            continue;
        }
        let mut restore = [None, None];
        for (i, (node, p)) in [(e.source, points[0]), (e.target, *points.last().unwrap())]
            .into_iter()
            .enumerate()
        {
            let (dx, dy) = super::bends::modifiers(graph, node);
            let b = result.boxes.get_mut(&node).unwrap();
            if (dx != 0.0 || dy != 0.0)
                && p.x > b.origin.x + dx
                && p.y < b.origin.y + b.size.height - dy
            {
                restore[i] = Some((node, b.origin));
                b.origin.x += dx;
                b.origin.y -= dy;
            }
        }
        let source = endpoint(graph, result, e.source, true);
        let target = endpoint(graph, result, e.target, false);
        result
            .routes
            .insert(id, trace_route(points, source, target));
        // Preserve source order even if a loop aliases both endpoint boxes.
        for (id, p) in restore.into_iter().flatten() {
            result.boxes.get_mut(&id).unwrap().origin = p;
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    use d2_rs_core::{Edge, LayoutReport, Node, NodeLabelState};
    use serde_json::Value;
    use std::collections::BTreeMap;
    #[test]
    fn original_go_trace_to_shape_complete_routes() {
        let cases: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_trace_to_shape.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 618);
        for case in cases {
            let mut graph = Graph::default();
            let mut result = LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                boxes: BTreeMap::new(),
                routes: BTreeMap::new(),
                edge_labels: BTreeMap::new(),
                edge_label_percentage_commits: Default::default(),
                node_labels: BTreeMap::new(),
                report: LayoutReport {
                    selected_seed: 1,
                    selected_pass: 0,
                    score: Default::default(),
                    improvement_passes: 0,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            };
            for (i, n) in case["Nodes"].as_array().unwrap().iter().enumerate() {
                let size = Size {
                    width: n["W"].as_f64().unwrap(),
                    height: n["H"].as_f64().unwrap(),
                };
                let mut node = Node::new(format!("node{i}"), size);
                node.shape = serde_json::from_value(n["Shape"].clone()).unwrap();
                node.has_icon = n["Icon"].as_bool().unwrap();
                let id = graph.add_node(node);
                graph.set_node_label(id, n["Label"].as_str().unwrap());
                result.boxes.insert(
                    id,
                    Rect {
                        origin: Point {
                            x: n["X"].as_f64().unwrap(),
                            y: n["Y"].as_f64().unwrap(),
                        },
                        size,
                    },
                );
                // Engine-committed positions override the graph's Unset state.
                result.node_labels.insert(
                    id,
                    NodeLabelState {
                        font_size_committed: false,
                        size: Some(Size {
                            width: n["LabelW"].as_f64().unwrap(),
                            height: n["LabelH"].as_f64().unwrap(),
                        }),
                        font_size: Some(16),
                        position: serde_json::from_value(n["Position"].clone()).unwrap(),
                        icon_position: Some(
                            serde_json::from_value(n["IconPosition"].clone()).unwrap(),
                        ),
                    },
                );
            }
            let edge = graph.add_edge(Edge {
                source: NodeId(0),
                target: NodeId(1),
            });
            result
                .routes
                .insert(edge, serde_json::from_value(case["Route"].clone()).unwrap());
            trace_routes(&graph, &mut result);
            let expected: Vec<Point> = serde_json::from_value(case["Expected"].clone()).unwrap();
            assert_eq!(result.routes[&edge], expected, "{}", case["Name"]);
        }
    }
}
