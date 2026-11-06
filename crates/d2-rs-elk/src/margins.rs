// SPDX-License-Identifier: MPL-2.0
//! Source-ordered shrinking bands and descendantShifter route operations.
use d2_rs_core::{EdgeId, Graph, Insets, LayoutResult, NodeId, Point};
use std::collections::BTreeMap;

struct DescendantShifter {
    order: Vec<NodeId>,
    begin: Vec<usize>,
    end: Vec<usize>,
    incident: Vec<Vec<EdgeId>>,
    seen: Vec<u64>,
    generation: u64,
    candidates: Vec<EdgeId>,
}
impl DescendantShifter {
    fn new(graph: &Graph) -> Self {
        let n = graph.nodes().len();
        let mut children = BTreeMap::<Option<NodeId>, Vec<NodeId>>::new();
        for (id, node) in graph.nodes() {
            children.entry(node.parent).or_default().push(id);
        }
        for (parent, ids) in &graph.hierarchy_children_order {
            children.insert(*parent, ids.clone());
        }
        let mut s = Self {
            order: Vec::with_capacity(n),
            begin: vec![usize::MAX; n],
            end: vec![0; n],
            incident: vec![Vec::new(); n],
            seen: vec![0; graph.edges().len()],
            generation: 0,
            candidates: Vec::new(),
        };
        fn visit(
            s: &mut DescendantShifter,
            id: NodeId,
            children: &BTreeMap<Option<NodeId>, Vec<NodeId>>,
        ) {
            s.begin[id.0 as usize] = s.order.len();
            s.order.push(id);
            if let Some(ids) = children.get(&Some(id)) {
                for &child in ids {
                    visit(s, child, children);
                }
            }
            s.end[id.0 as usize] = s.order.len();
        }
        if let Some(roots) = children.get(&None) {
            for &id in roots {
                visit(&mut s, id, &children);
            }
        }
        for (id, e) in graph.edges() {
            s.incident[e.source.0 as usize].push(id);
            if e.target != e.source {
                s.incident[e.target.0 as usize].push(id);
            }
        }
        s
    }
    fn inside(&self, root: NodeId, id: NodeId) -> bool {
        let a = self.begin[root.0 as usize];
        let b = self.begin[id.0 as usize];
        a != usize::MAX && b != usize::MAX && a <= b && b < self.end[root.0 as usize]
    }
    fn shift(&mut self, graph: &Graph, result: &mut LayoutResult, root: NodeId, dx: f64, dy: f64) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.seen.fill(0);
            self.generation = 1;
        }
        self.candidates.clear();
        let start = self.begin[root.0 as usize];
        if start == usize::MAX {
            return;
        }
        for i in start..self.end[root.0 as usize] {
            let id = self.order[i];
            if i != start {
                if let Some(b) = result.boxes.get_mut(&id) {
                    b.origin.x += dx;
                    b.origin.y += dy;
                }
            }
            for &edge in &self.incident[id.0 as usize] {
                if self.seen[edge.0 as usize] != self.generation {
                    self.seen[edge.0 as usize] = self.generation;
                    self.candidates.push(edge);
                }
            }
        }
        for &id in &self.candidates {
            let e = &graph.edges[id.0 as usize];
            let source = self.inside(root, e.source);
            let target = self.inside(root, e.target);
            let Some(route) = result.routes.get_mut(&id) else {
                continue;
            };
            if source && target {
                for p in route {
                    p.x += dx;
                    p.y += dy;
                }
            } else if source && e.source != root {
                shift_endpoint(route, dx, dy, true);
            } else if target && e.target != root {
                shift_endpoint(route, dx, dy, false);
            }
        }
    }
}
fn shift_endpoint(route: &mut Vec<Point>, dx: f64, dy: f64, source: bool) {
    if route.len() < 2 {
        return;
    }
    if dx != 0.0 && dy != 0.0 {
        let i = if source { 0 } else { route.len() - 1 };
        route[i].x += dx;
        route[i].y += dy;
        return;
    }
    let horizontal = dx != 0.0;
    let delta = if horizontal { dx } else { dy };
    let axis = |p: Point| if horizontal { p.x } else { p.y };
    let other = |p: Point| if horizontal { p.y } else { p.x };
    let i = if source { 0 } else { route.len() - 1 };
    let increasing = if source {
        axis(route[0]) < axis(route[1])
    } else {
        axis(route[i - 1]) < axis(route[i])
    };
    if horizontal {
        route[i].x += delta;
    } else {
        route[i].y += delta;
    }
    if increasing == if source { delta < 0.0 } else { delta > 0.0 } {
        return;
    }
    let p = route[i];
    let mut remove = vec![false; route.len()];
    for step in 1..route.len() - 1 {
        let j = if source { step } else { route.len() - 1 - step };
        if other(route[j]) != other(p) {
            break;
        }
        if if delta > 0.0 {
            axis(route[j]) < axis(p)
        } else {
            axis(route[j]) > axis(p)
        } {
            remove[j] = true;
        }
    }
    let mut j = 0;
    route.retain(|_| {
        let keep = !remove[j];
        j += 1;
        keep
    });
}

pub(super) fn apply(
    graph: &Graph,
    result: &mut LayoutResult,
    adjustments: &BTreeMap<NodeId, Insets>,
) {
    let mut shifter = DescendantShifter::new(graph);
    for (id, _) in graph.nodes() {
        let Some(m) = adjustments.get(&id) else {
            continue;
        };
        for (horizontal, leading, amount) in [
            (true, true, m.left),
            (true, false, m.right),
            (false, true, m.top),
            (false, false, m.bottom),
        ] {
            if !(amount > 0.0) {
                continue;
            }
            let Some(b) = result.boxes.get(&id).copied() else {
                continue;
            };
            let boundary = if horizontal {
                b.origin.x + if leading { 0.0 } else { b.size.width }
            } else {
                b.origin.y + if leading { 0.0 } else { b.size.height }
            };
            let delta = if leading { amount } else { -amount };
            for &edge in &shifter.incident[id.0 as usize] {
                let e = &graph.edges[edge.0 as usize];
                let Some(route) = result.routes.get_mut(&edge) else {
                    continue;
                };
                if route.is_empty() {
                    continue;
                }
                for (matches, index) in [(e.source == id, 0), (e.target == id, route.len() - 1)] {
                    if matches {
                        let p = &mut route[index];
                        let value = if horizontal { &mut p.x } else { &mut p.y };
                        if *value == boundary {
                            *value += delta;
                        }
                    }
                }
            }
            if leading {
                let b = result.boxes.get_mut(&id).unwrap();
                if horizontal {
                    b.origin.x += amount;
                } else {
                    b.origin.y += amount;
                }
            }
            let half = delta / 2.0;
            shifter.shift(
                graph,
                result,
                id,
                if horizontal { half } else { 0.0 },
                if horizontal { 0.0 } else { half },
            );
            let b = result.boxes.get_mut(&id).unwrap();
            if horizontal {
                b.size.width -= amount;
            } else {
                b.size.height -= amount;
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    use d2_rs_core::{Edge, LayoutReport, Node, Rect, Size};
    use serde_json::Value;
    #[test]
    fn original_go_margin_block_complete_boxes_and_routes() {
        let cases: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_margin_shrink.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 249);
        for case in cases {
            let nodes = case["Nodes"].as_array().unwrap();
            let mut graph = Graph::default();
            let mut ids = vec![NodeId(0); nodes.len()];
            for i in case["Order"].as_array().unwrap() {
                let i = i.as_u64().unwrap() as usize;
                let n = &nodes[i];
                ids[i] = graph.add_node(Node::new(
                    format!("node{i}"),
                    Size {
                        width: n["W"].as_f64().unwrap(),
                        height: n["H"].as_f64().unwrap(),
                    },
                ));
            }
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
            let mut children = BTreeMap::<Option<NodeId>, Vec<NodeId>>::new();
            let mut adjustments = BTreeMap::new();
            for (i, n) in nodes.iter().enumerate() {
                let parent = n["Parent"].as_u64().map(|j| ids[j as usize]);
                graph.node_mut(ids[i]).unwrap().parent = parent;
                children.entry(parent).or_default().push(ids[i]);
                result.boxes.insert(
                    ids[i],
                    Rect {
                        origin: Point {
                            x: n["X"].as_f64().unwrap(),
                            y: n["Y"].as_f64().unwrap(),
                        },
                        size: graph.node(ids[i]).unwrap().size,
                    },
                );
                let m = &n["Margin"];
                adjustments.insert(
                    ids[i],
                    Insets {
                        top: m["Top"].as_f64().unwrap(),
                        right: m["Right"].as_f64().unwrap(),
                        bottom: m["Bottom"].as_f64().unwrap(),
                        left: m["Left"].as_f64().unwrap(),
                    },
                );
            }
            graph.set_hierarchy_children_order(children);
            for e in case["Edges"].as_array().unwrap() {
                let id = graph.add_edge(Edge {
                    source: ids[e["Source"].as_u64().unwrap() as usize],
                    target: ids[e["Target"].as_u64().unwrap() as usize],
                });
                result
                    .routes
                    .insert(id, serde_json::from_value(e["Route"].clone()).unwrap());
            }
            apply(&graph, &mut result, &adjustments);
            for (i, &id) in ids.iter().enumerate() {
                let b = result.boxes[&id];
                let expected: [f64; 4] =
                    serde_json::from_value(case["ExpectedBoxes"][i].clone()).unwrap();
                assert_eq!(
                    [b.origin.x, b.origin.y, b.size.width, b.size.height],
                    expected,
                    "{}: box{i}",
                    case["Name"]
                );
            }
            for (id, _) in graph.edges() {
                let expected: Vec<Point> =
                    serde_json::from_value(case["ExpectedRoutes"][id.0 as usize].clone()).unwrap();
                assert_eq!(
                    result.routes[&id], expected,
                    "{}: route{}",
                    case["Name"], id.0
                );
            }
        }
    }
}
