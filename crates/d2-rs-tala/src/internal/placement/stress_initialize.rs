// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! D2 v0.9.0's bounded graph-distance starting arrangement for even seeds.
use super::{ArenaGraph, Point};
use d2_rs_core::stable_sort::StableSort;
use std::collections::{BTreeMap, BTreeSet};

impl ArenaGraph {
    pub(super) fn initialize_by_graph_distance(&mut self) -> bool {
        let nodes = self.node_order.clone();
        let n = nodes.len();
        if !(4..=64).contains(&n)
            || self.edge_order.len() > 256
            || nodes
                .iter()
                .any(|id| self.nodes[id.0 as usize].fixed_top_left.is_some())
        {
            return false;
        }
        let index = nodes
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i))
            .collect::<BTreeMap<_, _>>();
        let mut d = vec![vec![f64::INFINITY; n]; n];
        for (i, row) in d.iter_mut().enumerate() {
            row[i] = 0.0;
        }
        for edge in self.member_edges() {
            if let (Some(&i), Some(&j)) = (index.get(&edge.from), index.get(&edge.to))
                && i != j
            {
                d[i][j] = 1.0;
                d[j][i] = 1.0;
            }
        }
        for k in 0..n {
            for i in 0..n {
                for j in 0..n {
                    d[i][j] = d[i][j].min(d[i][k] + d[k][j]);
                }
            }
        }
        if d.iter().flatten().any(|v| v.is_infinite()) {
            return false;
        }
        let mut x = vec![0.0; n];
        let mut y = vec![0.0; n];
        for i in 0..n {
            let angle = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
            x[i] = (n as f64).sqrt() * angle.cos();
            y[i] = (n as f64).sqrt() * angle.sin();
        }
        for sweep in 0..48 {
            let eta = 0.7 * (1.0_f64 / 35.0).powf(sweep as f64 / 47.0);
            for p in 0..n {
                let i = (p + sweep) % n;
                for q in p + 1..n {
                    let j = (q + sweep) % n;
                    let mut dx = x[i] - x[j];
                    let mut dy = y[i] - y[j];
                    let mut length = super::routing::channel_points::go_hypot(dx, dy);
                    if length < 1e-9 {
                        dx = 1e-6;
                        dy = 1e-6;
                        length = 2.0_f64.sqrt() * 1e-6;
                    }
                    let mu = (eta / (d[i][j] * d[i][j])).min(1.0);
                    let amount = 0.5 * mu * (length - d[i][j]) / length;
                    x[i] -= dx * amount;
                    y[i] -= dy * amount;
                    x[j] += dx * amount;
                    y[j] += dy * amount;
                }
            }
        }
        let mut order = (0..n).collect::<Vec<_>>();
        order.stable_sort_by_key(|&i| {
            std::cmp::Reverse(self.nodes[nodes[i].0 as usize].edges.len())
        });
        let mut occupied = BTreeSet::new();
        let mut points = vec![Point::default(); n];
        for i in order {
            let tx = x[i] * 2.0 + n as f64;
            let ty = y[i] * 2.0 + n as f64;
            let cx = tx.round() as i64;
            let cy = ty.round() as i64;
            let mut best = (cx, cy);
            let mut cost = f64::INFINITY;
            for radius in 0..=n as i64 {
                for xx in cx - radius..=cx + radius {
                    for yy in cy - radius..=cy + radius {
                        if radius > 0
                            && xx != cx - radius
                            && xx != cx + radius
                            && yy != cy - radius
                            && yy != cy + radius
                        {
                            continue;
                        }
                        if occupied.contains(&(xx, yy)) {
                            continue;
                        }
                        let c = (xx as f64 - tx).powi(2) + (yy as f64 - ty).powi(2);
                        if c < cost {
                            best = (xx, yy);
                            cost = c;
                        }
                    }
                }
                if cost < f64::INFINITY {
                    break;
                }
            }
            occupied.insert(best);
            points[i] = Point {
                x: best.0 as f64,
                y: best.1 as f64,
            };
        }
        for (id, point) in nodes.into_iter().zip(points) {
            self.set_position(id, point);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, Size};

    fn path(n: usize, connected: bool) -> ArenaGraph {
        let mut input = Graph::default();
        let nodes = (0..n)
            .map(|i| {
                input.add_node(Node::new(
                    i.to_string(),
                    Size {
                        width: 100.0,
                        height: 50.0,
                    },
                ))
            })
            .collect::<Vec<_>>();
        if connected {
            for pair in nodes.windows(2) {
                input.add_edge(Edge {
                    source: pair[0],
                    target: pair[1],
                });
            }
        }
        ArenaGraph::from_input(&input)
    }

    #[test]
    fn graph_distance_preserves_sizes_and_global_path_distance() {
        let mut graph = path(12, true);
        assert!(graph.initialize_by_graph_distance());
        let points = graph
            .nodes
            .iter()
            .map(|n| n.position.unwrap())
            .collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        for (node, p) in graph.nodes.iter().zip(&points) {
            assert!(p.x.is_finite() && p.y.is_finite());
            assert_eq!(p.x.fract(), 0.0);
            assert_eq!(p.y.fract(), 0.0);
            assert!(seen.insert((p.x as i64, p.y as i64)));
            assert_eq!(
                node.rect.size,
                Size {
                    width: 100.0,
                    height: 50.0
                }
            );
        }
        let distance = |a: Point, b: Point| {
            crate::internal::routing::channel_points::go_hypot(a.x - b.x, a.y - b.y)
        };
        assert!(distance(points[0], points[11]) >= 2.0 * distance(points[0], points[1]));
        assert!(graph.initialize_by_graph_distance());
        assert_eq!(
            points,
            graph
                .nodes
                .iter()
                .map(|n| n.position.unwrap())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn graph_distance_fallback_preserves_existing_positions() {
        for (n, connected, fixed) in [
            (3, true, false),
            (65, true, false),
            (8, false, false),
            (8, true, true),
        ] {
            let mut graph = path(n, connected);
            for (i, node) in graph.nodes.iter_mut().enumerate() {
                node.position = Some(Point {
                    x: i as f64,
                    y: 17.0,
                });
            }
            if fixed {
                graph.nodes[2].fixed_top_left = Some(Point { x: 20.0, y: 30.0 });
            }
            let before = graph.nodes.iter().map(|n| n.position).collect::<Vec<_>>();
            assert!(!graph.initialize_by_graph_distance());
            assert_eq!(
                before,
                graph.nodes.iter().map(|n| n.position).collect::<Vec<_>>()
            );
        }
    }
}
