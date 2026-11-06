// Copyright (c) 2016 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// elk-go/internal/stress/majorization.go and javacompat/priority_queue.go.
use super::*;
struct Queue(Vec<usize>);
impl Queue {
    fn up(&mut self, mut i: usize, item: usize, d: &[f64]) -> usize {
        self.0[i] = item;
        while i > 0 {
            let p = (i - 1) / 2;
            if d[self.0[i]].total_cmp(&d[self.0[p]]) != std::cmp::Ordering::Less {
                break;
            }
            self.0.swap(i, p);
            i = p;
        }
        i
    }
    fn down(&mut self, mut i: usize, item: usize, d: &[f64]) -> usize {
        self.0[i] = item;
        loop {
            let left = i * 2 + 1;
            if left >= self.0.len() {
                return i;
            }
            let right = left + 1;
            let selected = if right < self.0.len()
                && d[self.0[left]].total_cmp(&d[self.0[right]]) == std::cmp::Ordering::Greater
            {
                right
            } else {
                left
            };
            if d[self.0[i]].total_cmp(&d[self.0[selected]]) != std::cmp::Ordering::Greater {
                return i;
            }
            self.0.swap(i, selected);
            i = selected;
        }
    }
    fn add(&mut self, n: usize, d: &[f64]) {
        let i = self.0.len();
        self.0.push(n);
        self.up(i, n, d);
    }
    fn remove(&mut self, i: usize, d: &[f64]) {
        let moved = self.0.pop().unwrap();
        if i == self.0.len() {
            return;
        }
        if self.down(i, moved, d) == i {
            self.up(i, moved, d);
        }
    }
    fn poll(&mut self, d: &[f64]) -> usize {
        let result = self.0[0];
        let moved = self.0.pop().unwrap();
        if !self.0.is_empty() {
            self.down(0, moved, d);
        }
        result
    }
}
impl Arena {
    pub(super) fn majorize(&mut self, c: &Component, graph: &ElkGraph) {
        let dimension = enum_value(
            &graph.layout_options,
            "stress.dimension",
            &["XY", "X", "Y"],
            "XY",
        );
        let limit = integer(&graph.layout_options, "stress.iterationLimit").unwrap_or(i32::MAX);
        let epsilon = number(&graph.layout_options, "stress.epsilon", 0.001);
        let desired = number(&graph.layout_options, "stress.desiredEdgeLength", 100.0);
        let mut incidence = vec![vec![]; self.nodes];
        for &e in &c.edges {
            incidence[self.edges[e].source].push(e);
            incidence[self.edges[e].target].push(e);
        }
        let mut distances = vec![vec![]; self.nodes];
        for &source in &c.nodes {
            let mut marked = vec![false; self.nodes];
            let mut d = vec![i32::MAX as f64; self.nodes];
            d[source] = 0.0;
            let mut q = Queue(vec![]);
            for &n in &c.nodes {
                q.add(n, &d);
            }
            while !q.0.is_empty() {
                let n = q.poll(&d);
                marked[n] = true;
                for &e in &incidence[n] {
                    let edge = &self.edges[e];
                    let other = if edge.source == n {
                        edge.target
                    } else {
                        edge.source
                    };
                    if marked[other] {
                        continue;
                    }
                    let length = graph.edges[edge.origin]
                        .layout_options
                        .as_ref()
                        .map(|o| number(o, "stress.desiredEdgeLength", desired))
                        .unwrap_or(desired);
                    let candidate = d[n] + length;
                    if candidate < d[other] {
                        d[other] = candidate;
                        let i = q.0.iter().position(|&n| n == other).unwrap();
                        q.remove(i, &d);
                        q.add(other, &d);
                    }
                }
            }
            distances[source] = d;
        }
        let weights: Vec<Vec<f64>> = distances
            .iter()
            .map(|row| row.iter().map(|d| 1.0 / (d * d)).collect())
            .collect();
        let compute_stress = |a: &Arena| {
            let mut stress = 0.0;
            for (i, &first) in c.nodes.iter().enumerate() {
                for &second in &c.nodes[i + 1..] {
                    let displacement = a.particles[first]
                        .position
                        .sub(a.particles[second].position)
                        .length()
                        - distances[first][second];
                    let weighted = weights[first][second] * displacement;
                    let term = weighted * displacement;
                    stress += term;
                }
            }
            stress
        };
        let mut previous = compute_stress(self);
        let mut current = f64::INFINITY;
        let mut count = 0;
        loop {
            if count > 0 {
                previous = current;
            }
            for &node in &c.nodes {
                let Kind::Node(origin) = self.particles[node].kind else {
                    unreachable!()
                };
                if graph.children[origin]
                    .layout_options
                    .as_ref()
                    .is_some_and(|o| flag(o, "stress.fixed", false))
                {
                    continue;
                }
                let (mut sum, mut x, mut y) = (0.0, 0.0, 0.0);
                for &other in &c.nodes {
                    if node == other {
                        continue;
                    }
                    let w = weights[node][other];
                    sum += w;
                    let distance = self.particles[node]
                        .position
                        .sub(self.particles[other].position)
                        .length();
                    if distance > 0.0 && dimension != "Y" {
                        let difference =
                            self.particles[node].position.x - self.particles[other].position.x;
                        let scaled = distances[node][other] * difference;
                        let target = self.particles[other].position.x + scaled / distance;
                        let contribution = w * target;
                        x += contribution;
                    }
                    if distance > 0.0 && dimension != "X" {
                        let difference =
                            self.particles[node].position.y - self.particles[other].position.y;
                        let scaled = distances[node][other] * difference;
                        let target = self.particles[other].position.y + scaled / distance;
                        let contribution = w * target;
                        y += contribution;
                    }
                }
                let previous = self.particles[node].position;
                self.particles[node].position = match dimension.as_str() {
                    "X" => V {
                        x: x / sum,
                        y: previous.y,
                    },
                    "Y" => V {
                        x: previous.x,
                        y: y / sum,
                    },
                    _ => V {
                        x: x / sum,
                        y: y / sum,
                    },
                };
            }
            current = compute_stress(self);
            let done =
                previous == 0.0 || (previous - current) / previous < epsilon || count >= limit;
            count += 1;
            if done {
                break;
            }
        }
    }
}
