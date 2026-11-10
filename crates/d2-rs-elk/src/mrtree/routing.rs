// Copyright (c) 2013, 2015, 2017, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// Original mrtree/provider.go: routeEdges through lastRoutePoint and toNodeBorder.
use super::*;
use d2_rs_core::stable_sort::StableSort;
#[derive(Default)]
struct Gap {
    one: Option<usize>,
    two: Option<usize>,
    entries: Vec<(usize, usize)>,
    first: bool,
    last: bool,
}
fn border(center: &mut V, next: V, size: V) {
    let (dx, dy) = (next.x - center.x, next.y - center.y);
    let (hw, hh) = (size.x / 2., size.y / 2.);
    let xs = if dx.abs() > hw { hw / dx.abs() } else { 1. };
    let ys = if dy.abs() > hh { hh / dy.abs() } else { 1. };
    let scale = xs.min(ys);
    let ox = scale * dx;
    let oy = scale * dy;
    center.x += ox;
    center.y += oy;
}
impl Arena {
    fn first_point(&self, e: usize) -> V {
        self.edges[e]
            .bends
            .first()
            .copied()
            .unwrap_or(self.nodes[self.edges[e].target].pos)
    }
    fn last_point(&self, e: usize) -> V {
        self.edges[e]
            .bends
            .last()
            .copied()
            .unwrap_or(self.nodes[self.edges[e].source].pos)
    }
    fn middle_route(&mut self, e: usize) {
        let edge = &self.edges[e];
        let s = &self.nodes[edge.source];
        let t = &self.nodes[edge.target];
        let a = V::new(s.pos.x + s.size.x / 2., s.pos.y + s.size.y / 2.);
        let b = V::new(t.pos.x + t.size.x / 2., t.pos.y + t.size.y / 2.);
        let (ss, ts) = (s.size, t.size);
        self.edges[e].bends.insert(0, a);
        self.edges[e].bends.push(b);
        let points = &mut self.edges[e].bends;
        let last = points.len() - 1;
        let next = points[1];
        border(&mut points[0], next, ss);
        let prev = points[last - 1];
        border(&mut points[last], prev, ts);
    }
    pub(super) fn route(&mut self, g: &Graph) {
        for &e in &g.edges {
            self.edges[e].bends.clear()
        }
        match g.cfg.route.as_str() {
            "NONE" => (),
            "MIDDLE_TO_MIDDLE" => {
                for &e in &g.edges {
                    self.middle_route(e)
                }
            }
            _ => {
                self.starts(g);
                self.special(g);
                self.ends(g);
                for &e in &g.edges {
                    if self.edges[e].bends.len() < 2 {
                        self.middle_route(e)
                    }
                }
            }
        }
    }
    fn starts(&mut self, g: &Graph) {
        for &n in &g.nodes {
            if self.nodes[n].dummy {
                continue;
            }
            let mut outgoing = g
                .edges
                .iter()
                .copied()
                .filter(|&e| {
                    let e = &self.edges[e];
                    e.source == n && self.nodes[e.source].level != self.nodes[e.target].level
                })
                .collect::<Vec<_>>();
            outgoing.stable_sort_by(|&a, &b| {
                let (a, b) = (self.first_point(a), self.first_point(b));
                let (a, b) = if g.cfg.horizontal() {
                    (a.y, b.y)
                } else {
                    (a.x, b.x)
                };
                a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
            });
            for (index, &e) in outgoing.iter().enumerate() {
                let node = &self.nodes[n];
                if node.ascension && self.nodes[self.edges[e].target].level > node.level {
                    continue;
                }
                let t = if outgoing.len() == 1 {
                    0.5
                } else {
                    (index + 1) as f64 / (outgoing.len() + 1) as f64
                };
                let p = node.pos;
                let s = node.size;
                let (start, end) = match g.cfg.direction.as_str() {
                    "LEFT" => {
                        let y = crate::js_trig::product_sum(s.y, t, p.y);
                        (
                            V::new(p.x, y),
                            V::new(node.level_min.min(p.x - g.cfg.edge_space), y),
                        )
                    }
                    "RIGHT" => {
                        let y = crate::js_trig::product_sum(s.y, t, p.y);
                        (
                            V::new(p.x + s.x, y),
                            V::new(node.level_max + g.cfg.edge_space, y),
                        )
                    }
                    "UP" => {
                        let x = crate::js_trig::product_sum(s.x, t, p.x);
                        (
                            V::new(x, p.y),
                            V::new(x, (p.y - g.cfg.edge_space).min(node.level_min)),
                        )
                    }
                    _ => {
                        let x = crate::js_trig::product_sum(s.x, t, p.x);
                        (
                            V::new(x, p.y + s.y),
                            V::new(x, node.level_max + g.cfg.edge_space),
                        )
                    }
                };
                self.edges[e].bends.splice(0..0, [start, end]);
            }
        }
    }
    fn steep(&self, e: usize, n: usize, c: &Config) -> bool {
        if c.texture <= 0. || self.edges[e].bends.is_empty() {
            return false;
        }
        let last = self.last_point(e);
        let node = &self.nodes[n];
        let next = V::new(node.pos.x + node.size.x / 2., node.pos.y + node.size.y / 2.);
        if c.horizontal() {
            (last.y - next.y).abs() / ((last.x - next.x).abs() / 40.) > 50.
        } else {
            (last.x - next.x).abs() / ((last.y - next.y).abs() / 40.) > 50.
        }
    }
    fn ends(&mut self, g: &Graph) {
        for &n in &g.nodes {
            if self.nodes[n].dummy {
                continue;
            }
            let mut incoming = g
                .edges
                .iter()
                .copied()
                .filter(|&e| {
                    let e = &self.edges[e];
                    e.target == n && self.nodes[e.source].level != self.nodes[e.target].level
                })
                .collect::<Vec<_>>();
            incoming.stable_sort_by(|&a, &b| {
                let (a, b) = (self.last_point(a), self.last_point(b));
                let (a, b) = if g.cfg.horizontal() {
                    (a.y, b.y)
                } else {
                    (a.x, b.x)
                };
                a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
            });
            for (index, &e) in incoming.iter().enumerate() {
                let node = &self.nodes[n];
                let t = if incoming.len() == 1 {
                    0.5
                } else {
                    (index + 1) as f64 / (incoming.len() + 1) as f64
                };
                let p = node.pos;
                let s = node.size;
                let c = &g.cfg;
                let last = self.last_point(e);
                let mut points = vec![];
                match c.direction.as_str() {
                    "LEFT" | "RIGHT" => {
                        let y = crate::js_trig::product_sum(s.y, t, p.y);
                        let left = c.direction == "LEFT";
                        if if left {
                            p.x + s.x + c.texture < node.level_max
                        } else {
                            p.x - c.texture > node.level_min
                        } {
                            points.push(V::new(
                                if left {
                                    node.level_max + c.edge_space
                                } else {
                                    node.level_min - c.edge_space
                                },
                                y,
                            ));
                        } else if self.steep(e, n, c) {
                            let bend = y + if p.y + s.y / 2. > last.y {
                                -c.texture / 2.
                            } else {
                                c.texture / 2.
                            };
                            points.push(V::new(
                                if left {
                                    p.x + s.x + c.texture / 5.3
                                } else {
                                    p.x - c.texture / 5.3
                                },
                                bend,
                            ))
                        }
                        points.push(V::new(if left { p.x + s.x } else { p.x }, y));
                    }
                    "UP" => {
                        let x = crate::js_trig::product_sum(s.x, t, p.x);
                        if p.y + s.y + c.texture < node.level_max {
                            points.push(V::new(x, node.level_max + c.edge_space))
                        } else if self.steep(e, n, c) {
                            let bend = x + if p.x + s.x / 2. > last.x {
                                -c.texture / 2.
                            } else {
                                c.texture / 2.
                            };
                            points.push(V::new(bend, p.y + s.y + c.texture / 5.3))
                        }
                        points.push(V::new(x, p.y + s.y));
                    }
                    _ => {
                        let x = crate::js_trig::product_sum(s.x, t, p.x);
                        if self.nodes[self.edges[e].source].level > node.level {
                            points.push(V::new(x, last.y))
                        } else if p.y - c.texture > node.level_min {
                            points.push(V::new(x, node.level_min - c.edge_space))
                        } else if self.steep(e, n, c) {
                            let bend = x + if p.x + s.x / 2. > last.x {
                                -c.texture / 2.
                            } else {
                                c.texture / 2.
                            };
                            points.push(V::new(bend, p.y - c.texture / 5.3))
                        }
                        points.push(V::new(x, p.y));
                    }
                }
                self.edges[e].bends.extend(points);
            }
        }
    }
    fn update_gap(&mut self, g: &Graph, gap: &mut Gap) {
        let horizontal = g.cfg.horizontal();
        for (index, &(e, first)) in gap.entries.iter().enumerate() {
            let t = (index + 1) as f64 / (gap.entries.len() + 1) as f64;
            let (node, coordinate) = match (gap.one, gap.two) {
                (None, None) => return,
                (Some(n), None) => {
                    gap.last = true;
                    let nnode = &self.nodes[n];
                    let end = if horizontal {
                        nnode.pos.y + nnode.size.y
                    } else {
                        nnode.pos.x + nnode.size.x
                    };
                    (n, g.cfg.edge_space.mul_add((index + 1) as f64, end))
                }
                (Some(one), Some(two)) => {
                    let a = &self.nodes[one];
                    let b = &self.nodes[two];
                    let coordinate = if horizontal {
                        (a.pos.y + a.size.y).mul_add(1. - t, b.pos.y * t)
                    } else {
                        (a.pos.x + a.size.x).mul_add(1. - t, b.pos.x * t)
                    };
                    (one, coordinate)
                }
                (None, Some(n)) => {
                    gap.first = true;
                    let node = &self.nodes[n];
                    let p = if horizontal { node.pos.y } else { node.pos.x };
                    (n, (-g.cfg.edge_space).mul_add((index + 1) as f64, p))
                }
            };
            let node = &self.nodes[node];
            let (low, high) = (
                node.level_min - g.cfg.edge_space,
                node.level_max + g.cfg.edge_space,
            );
            let (a, b) = match g.cfg.direction.as_str() {
                "LEFT" => (V::new(high, coordinate), V::new(low, coordinate)),
                "RIGHT" => (V::new(low, coordinate), V::new(high, coordinate)),
                "UP" => (V::new(coordinate, high), V::new(coordinate, low)),
                _ => (V::new(coordinate, low), V::new(coordinate, high)),
            };
            self.edges[e].bends[first] = a;
            self.edges[e].bends[first + 1] = b;
        }
    }
    fn skip_gap(&self, e: usize, nodes: &[usize], index: usize, horizontal: bool) -> bool {
        if nodes.is_empty() {
            return false;
        }
        let edge = &self.edges[e];
        let start = edge
            .bends
            .last()
            .copied()
            .unwrap_or(self.nodes[edge.source].pos);
        let last = &self.nodes[*nodes.last().unwrap()];
        let last = last.pos.add(last.size);
        let first = &self.nodes[nodes[0]];
        let first = first.pos.add(first.size);
        let target = self.nodes[edge.target].pos;
        if horizontal {
            if index >= nodes.len() - 1 && start.y > last.y && target.y > last.y {
                return true;
            }
            return index == 0 && start.y < first.x && target.y < first.y;
        }
        if index >= nodes.len() - 1 && start.x > last.x && target.x > last.x {
            return true;
        }
        index == 0 && start.x < first.x && target.x < first.x
    }
    fn special(&mut self, g: &Graph) {
        let max = g
            .nodes
            .iter()
            .map(|&n| self.nodes[n].level)
            .max()
            .unwrap_or(0);
        let mut outs = vec![0; max + 1];
        let mut ins = vec![0; max + 1];
        let mut gaps = BTreeMap::<(usize, usize), Gap>::new();
        let (mut side_one, mut side_two) = (0, 0);
        for &e in &g.edges {
            let (s, t) = (self.edges[e].source, self.edges[e].target);
            let (sl, tl) = (self.nodes[s].level, self.nodes[t].level);
            if tl > sl + 1 {
                for level in sl + 1..tl {
                    let horizontal = g.cfg.horizontal();
                    let mut nodes = g
                        .nodes
                        .iter()
                        .copied()
                        .filter(|&n| self.nodes[n].level == level)
                        .collect::<Vec<_>>();
                    nodes.stable_sort_by(|&a, &b| {
                        let a = self.nodes[a].pos;
                        let b = self.nodes[b].pos;
                        let (a, b) = if horizontal { (a.y, b.y) } else { (a.x, b.x) };
                        a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    let interpolation = (level - sl) as f64 / (tl - sl) as f64;
                    let sp = self.nodes[s].pos;
                    let tp = self.nodes[t].pos;
                    let desired = if horizontal {
                        tp.y.mul_add(interpolation, sp.y * (1. - interpolation))
                    } else {
                        tp.x.mul_add(interpolation, sp.x * (1. - interpolation))
                    };
                    let index = nodes
                        .iter()
                        .position(|&n| {
                            if horizontal {
                                self.nodes[n].pos.y > desired
                            } else {
                                self.nodes[n].pos.x > desired
                            }
                        })
                        .unwrap_or(nodes.len());
                    if self.skip_gap(e, &nodes, index, horizontal) {
                        continue;
                    }
                    let first = self.edges[e].bends.len();
                    self.edges[e].bends.extend([V::default(), V::default()]);
                    let gap = gaps.entry((level, index)).or_insert_with(|| Gap {
                        one: index.checked_sub(1).map(|i| nodes[i]),
                        two: nodes.get(index).copied(),
                        ..Gap::default()
                    });
                    gap.entries.push((e, first));
                    gap.entries.stable_sort_by(|&(a, _), &(b, _)| {
                        let a = self.nodes[self.edges[a].target].pos;
                        let b = self.nodes[self.edges[b].target].pos;
                        let (a, b) = if horizontal { (a.y, b.y) } else { (a.x, b.x) };
                        a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    self.update_gap(g, gap);
                    if let Some(n) = gap.two {
                        let node = &self.nodes[n];
                        if gap.first
                            && if horizontal {
                                node.pos.y <= g.min.y
                            } else {
                                node.pos.x <= g.min.x
                            }
                        {
                            side_one += 1
                        }
                    }
                    if let Some(n) = gap.one {
                        let node = &self.nodes[n];
                        if gap.last
                            && if horizontal {
                                node.pos.y + node.size.y >= g.max.y
                            } else {
                                node.pos.x + node.size.x >= g.max.x
                            }
                        {
                            side_two += 1
                        }
                    }
                }
            } else if tl == sl {
                self.middle_route(e)
            } else if tl < sl {
                outs[sl] += 1;
                ins[tl] += 1;
                (side_one, side_two) = self.cycle(g, e, side_one, side_two, ins[tl], outs[sl]);
            }
        }
    }
    fn cycle(
        &mut self,
        g: &Graph,
        e: usize,
        mut side_one: usize,
        mut side_two: usize,
        ins: usize,
        outs: usize,
    ) -> (usize, usize) {
        let c = &g.cfg;
        let horizontal = c.horizontal();
        let mut middle = 0.;
        for &n in &g.nodes {
            let n = &self.nodes[n];
            middle += if horizontal {
                n.pos.y + n.size.y / 2.
            } else {
                n.pos.x + n.size.x / 2.
            }
        }
        middle /= g.nodes.len() as f64;
        let source = &self.nodes[self.edges[e].source];
        let source_middle = if horizontal {
            source.pos.y + source.size.y / 2.
        } else {
            source.pos.x + source.size.x / 2.
        };
        let mut bend;
        if source_middle > middle {
            side_two += 1;
            bend = f64::NEG_INFINITY;
            for &n in &g.nodes {
                let n = &self.nodes[n];
                let end = if horizontal {
                    n.pos.y + n.size.y
                } else {
                    n.pos.x + n.size.x
                };
                bend = bend.max(c.edge_space.mul_add(side_two as f64, end))
            }
        } else {
            side_one += 1;
            bend = f64::INFINITY;
            for &n in &g.nodes {
                let n = &self.nodes[n];
                let p = if horizontal { n.pos.y } else { n.pos.x };
                bend = bend.min((-c.edge_space).mul_add(side_one as f64, p))
            }
        }
        let s = &self.nodes[self.edges[e].source];
        let t = &self.nodes[self.edges[e].target];
        let points = match c.direction.as_str() {
            "LEFT" => vec![
                V::new(s.level_min - c.edge_space, bend),
                V::new(t.pos.x + t.size.x + c.edge_space + c.texture, bend),
                V::new(
                    t.pos.x + t.size.x + c.edge_space + c.texture,
                    t.pos.y + t.size.y / 2.,
                ),
                V::new(t.pos.x + t.size.x, t.pos.y + t.size.y / 2.),
            ],
            "RIGHT" => vec![
                V::new(s.level_max + c.edge_space, s.pos.y + s.size.y / 2.),
                V::new(s.pos.x + s.size.x + c.edge_space, bend),
                V::new(t.pos.x - c.edge_space - c.texture, bend),
                V::new(t.pos.x - c.edge_space - c.texture, t.pos.y + t.size.y / 2.),
                V::new(t.pos.x, t.pos.y + t.size.y / 2.),
            ],
            "UP" => vec![
                V::new(bend, s.level_min - c.edge_space),
                V::new(bend, t.pos.y + t.size.y + c.edge_space + c.texture),
                V::new(
                    t.pos.x + t.size.x / 2.,
                    t.pos.y + t.size.y + c.edge_space + c.texture,
                ),
                V::new(t.pos.x + t.size.x / 2., t.pos.y + t.size.y + c.edge_space),
            ],
            _ => {
                let first = c.edge_space.mul_add(outs as f64, s.level_max);
                let last = (-c.edge_space).mul_add(ins as f64, t.pos.y) - c.texture;
                if let Some(p) = self.edges[e].bends.last_mut() {
                    p.y = first
                }
                vec![V::new(bend, first), V::new(bend, last)]
            }
        };
        self.edges[e].bends.extend(points);
        (side_one, side_two)
    }
}
