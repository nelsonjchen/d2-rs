// SPDX-License-Identifier: MPL-2.0
//! Original d2elklayout.deleteBends and its exact collision predicates.
use super::bend_index::CollisionIndex;
use d2_rs_core::shape_geometry::{box_intersections, line_intersection_point, trace_shape_border};
use d2_rs_core::{EdgeId, Graph, LayoutResult, NodeId, Point, Rect, ShapeKind, Size};
use std::collections::BTreeMap;
type Segment = (Point, Point);

fn horizontal((a, b): Segment) -> bool {
    a.y.ceil() == b.y.ceil()
}
fn projected_overlap((a, b): Segment, (c, d): Segment, horizontal_axis: bool) -> bool {
    let (a, b, c, d) = if horizontal_axis {
        (a.y, b.y, c.y, d.y)
    } else {
        (a.x, b.x, c.x, d.x)
    };
    !(a.min(b) - c.max(d) >= 0.0 || c.min(d) - a.max(b) >= 0.0)
}
fn counts(
    graph: &Graph,
    result: &LayoutResult,
    index: &CollisionIndex,
    edge: EdgeId,
    segments: &[Segment],
) -> [usize; 5] {
    let mut out = [0; 5];
    let e = &graph.edges[edge.0 as usize];
    for &segment in segments {
        for id in index.objects(segment.0, segment.1) {
            if id == e.source || id == e.target {
                continue;
            }
            if let Some(rect) = result.boxes.get(&id) {
                // Source Box.Intersects tests the four expanded sides; an
                // entirely enclosed segment is not itself an intersection.
                let expanded = Rect {
                    origin: Point {
                        x: rect.origin.x - 39.0,
                        y: rect.origin.y - 39.0,
                    },
                    size: Size {
                        width: rect.size.width + 78.0,
                        height: rect.size.height + 78.0,
                    },
                };
                if !box_intersections(expanded, segment.0, segment.1).is_empty() {
                    out[0] += 1;
                }
            }
        }
        let h = horizontal(segment);
        for (other_id, i) in index.segments(segment.0, segment.1) {
            if other_id == edge {
                continue;
            }
            let route = &result.routes[&other_id];
            let other = (route[i], route[i + 1]);
            if h == horizontal(other) {
                if projected_overlap(segment, other, !h) {
                    let distance = if h {
                        (segment.0.y - other.0.y).abs()
                    } else {
                        (segment.0.x - other.0.x).abs()
                    };
                    if distance < 20.0 {
                        out[2] += 1;
                        if distance < 10.0 {
                            out[3] += 1;
                            // The original vertical touching predicate uses Y.
                            if (segment.0.y - other.0.y).abs() < 1.0 {
                                out[4] += 1;
                            }
                        }
                    }
                }
            } else if line_intersection_point(segment.0, segment.1, other.0, other.1).is_some() {
                out[1] += 1;
            }
        }
    }
    out
}
pub(super) fn modifiers(graph: &Graph, id: NodeId) -> (f64, f64) {
    let node = graph.node(id).unwrap();
    if node.is_3d {
        (
            15.0,
            if node.shape == ShapeKind::Hexagon {
                7.0
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
fn no_worse(old: [usize; 5], new: [usize; 5]) -> bool {
    old.into_iter().zip(new).all(|(a, b)| b <= a)
}

pub(super) fn delete_bends(graph: &Graph, result: &mut LayoutResult) {
    if !graph
        .edges()
        .any(|(id, e)| e.source != e.target && result.routes.get(&id).is_some_and(|r| r.len() >= 4))
    {
        return;
    }
    let mut index = CollisionIndex::new(graph, result);
    for source in [true, false] {
        for (id, edge) in graph.edges() {
            if edge.source == edge.target {
                continue;
            }
            let Some(route) = result.routes.get(&id).filter(|r| r.len() >= 4) else {
                continue;
            };
            let n = route.len();
            let (start, corner, end, node, column) = if source {
                (
                    route[0],
                    route[1],
                    route[2],
                    edge.source,
                    graph.edge_table_columns(id).source,
                )
            } else {
                (
                    route[n - 1],
                    route[n - 2],
                    route[n - 3],
                    edge.target,
                    graph.edge_table_columns(id).target,
                )
            };
            let Some(rect) = result.boxes.get(&node).copied() else {
                continue;
            };
            let h = horizontal((start, corner));
            let (dx, dy) = modifiers(graph, node);
            if let Some(column) = column {
                let row_height =
                    rect.size.height / (graph.table_column_count(node).unwrap_or(0) + 1) as f64;
                let center = rect.origin.y + row_height * (column + 1) as f64 + row_height / 2.0;
                if (end.y - center).abs() > row_height / 3.0 {
                    continue;
                }
            } else if h {
                if end.y <= rect.origin.y + 10.0 - dy
                    || end.y >= rect.origin.y + rect.size.height - 10.0
                {
                    continue;
                }
            } else if end.x <= rect.origin.x + 10.0
                || end.x >= rect.origin.x + rect.size.width - 10.0 + dx
            {
                continue;
            }
            let new_start = if h {
                Point {
                    x: start.x,
                    y: end.y,
                }
            } else {
                Point {
                    x: end.x,
                    y: start.y,
                }
            };
            let new_start =
                trace_shape_border(graph.node(node).unwrap().shape, rect, new_start, end);
            if !no_worse(
                counts(graph, result, &index, id, &[(start, corner)]),
                counts(graph, result, &index, id, &[(new_start, end)]),
            ) {
                continue;
            }
            let mut next = if source {
                vec![new_start]
            } else {
                route[..n - 3].to_vec()
            };
            if source {
                next.extend_from_slice(&route[3..]);
            } else {
                next.push(new_start);
            }
            result.routes.insert(id, next);
            index.update_edge(id, result);
        }
    }
    // Point sharing is deliberately captured once, before ladder commits.
    // Go map[geo.Point] treats signed zeros as equal. Preserve every other
    // finite f64 bit pattern; this is identity, not coordinate rounding.
    let key = |p: Point| {
        (
            if p.x == 0.0 { 0 } else { p.x.to_bits() },
            if p.y == 0.0 { 0 } else { p.y.to_bits() },
        )
    };
    let mut points = BTreeMap::<(u64, u64), usize>::new();
    for (id, _) in graph.edges() {
        if let Some(route) = result.routes.get(&id) {
            for &p in route {
                if p.x.is_nan() || p.y.is_nan() {
                    continue;
                }
                *points.entry(key(p)).or_default() += 1;
            }
        }
    }
    for (id, edge) in graph.edges() {
        if edge.source == edge.target {
            continue;
        }
        let Some(route) = result.routes.get(&id).filter(|r| r.len() >= 6) else {
            continue;
        };
        for i in 1..route.len() - 3 {
            let (before, start, corner, end, after) = (
                route[i - 1],
                route[i],
                route[i + 1],
                route[i + 2],
                route[i + 3],
            );
            if !corner.x.is_nan()
                && !corner.y.is_nan()
                && points.get(&key(corner)).copied().unwrap_or(0) > 1
            {
                continue;
            }
            let new_corner = if start.x.ceil() == corner.x.ceil() {
                if (end.x > start.x) != (start.x > before.x)
                    || (end.y > start.y) != (after.y > end.y)
                {
                    continue;
                }
                Point {
                    x: end.x,
                    y: start.y,
                }
            } else {
                if (end.y > start.y) != (start.y > before.y)
                    || (end.x > start.x) != (after.x > end.x)
                {
                    continue;
                }
                Point {
                    x: start.x,
                    y: end.y,
                }
            };
            if !no_worse(
                counts(graph, result, &index, id, &[(start, corner), (corner, end)]),
                counts(
                    graph,
                    result,
                    &index,
                    id,
                    &[(start, new_corner), (new_corner, end)],
                ),
            ) {
                continue;
            }
            let mut next = route[..i].to_vec();
            next.push(new_corner);
            next.extend_from_slice(&route[i + 3..]);
            result.routes.insert(id, next);
            index.update_edge(id, result);
            break;
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    use d2_rs_core::{Edge, EdgeTableColumns, LayoutReport, Node};
    use serde_json::Value;

    #[test]
    fn original_go_delete_bends_routes_and_collision_counts() {
        // Captured by invoking unmodified private deleteBends and its collision
        // index in the pinned Go source. See the pinned Go source.
        let cases: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_delete_bends.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 276);
        let mut query_count = 0;
        for case in cases {
            let name = case["Name"].as_str().unwrap();
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
                let mut node = Node::new(
                    format!("node{i}"),
                    Size {
                        width: n["W"].as_f64().unwrap(),
                        height: n["H"].as_f64().unwrap(),
                    },
                );
                node.shape = serde_json::from_value(n["Shape"].clone()).unwrap();
                node.is_3d = n["ThreeD"].as_bool().unwrap();
                node.is_multiple = n["Multiple"].as_bool().unwrap();
                let size = node.size;
                let id = graph.add_node(node);
                graph.set_table_column_count(id, n["Columns"].as_u64().map(|n| n as usize));
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
            }
            for e in case["Edges"].as_array().unwrap() {
                let id = graph.add_edge(Edge {
                    source: NodeId(e["Source"].as_u64().unwrap() as u32),
                    target: NodeId(e["Target"].as_u64().unwrap() as u32),
                });
                graph.set_edge_table_columns(
                    id,
                    EdgeTableColumns {
                        source: e["SourceColumn"].as_u64().map(|n| n as usize),
                        target: e["TargetColumn"].as_u64().map(|n| n as usize),
                    },
                );
                result
                    .routes
                    .insert(id, serde_json::from_value(e["Route"].clone()).unwrap());
            }
            let index = CollisionIndex::new(&graph, &result);
            if let Some(queries) = case["Queries"].as_array() {
                for (i, q) in queries.iter().enumerate() {
                    let seg: [Point; 2] = serde_json::from_value(q["Segment"].clone()).unwrap();
                    let actual = counts(
                        &graph,
                        &result,
                        &index,
                        EdgeId(q["Edge"].as_u64().unwrap() as u32),
                        &[(seg[0], seg[1])],
                    );
                    let expected: [usize; 5] =
                        serde_json::from_value(case["Counts"][i].clone()).unwrap();
                    assert_eq!(actual, expected, "{name}: collision query {i}");
                    query_count += 1;
                }
            }
            delete_bends(&graph, &mut result);
            for (id, _) in graph.edges() {
                let expected: Vec<Point> =
                    serde_json::from_value(case["Expected"][id.0 as usize].clone()).unwrap();
                assert_eq!(result.routes[&id], expected, "{name}: route {}", id.0);
            }
        }
        assert_eq!(query_count, 1280);
    }
}
