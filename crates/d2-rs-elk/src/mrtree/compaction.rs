// Copyright (c) 2013, 2015, 2017, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// Original mrtree/provider.go: compactGraph and scan-line dependency helpers.
use super::*;
use d2_rs_core::stable_sort::StableSort;
fn projection(c: &Config, p: V) -> f64 {
    match c.direction.as_str() {
        "LEFT" => -p.x,
        "RIGHT" => p.x,
        "UP" => -p.y,
        _ => p.y,
    }
}
fn perpendicular(c: &Config, p: V) -> f64 {
    if c.horizontal() { p.y } else { p.x }
}
impl Arena {
    fn set_index(&self, set: &[usize], n: usize, c: &Config) -> usize {
        let value = projection(c, self.nodes[n].pos);
        set.partition_point(|&i| projection(c, self.nodes[i].pos) < value)
    }
    fn constraints(&mut self, g: &Graph) {
        for &n in &g.nodes {
            self.nodes[n].constraints.clear()
        }
        let pad = g.cfg.spacing / 4.;
        let mut points = vec![];
        for start in [true, false] {
            for &n in &g.nodes {
                let node = &self.nodes[n];
                if node.dummy {
                    continue;
                }
                let p = if start {
                    V::new(node.pos.x - pad, node.pos.y - pad)
                } else {
                    V::new(
                        node.pos.x + node.size.x + pad,
                        node.pos.y + node.size.y + pad,
                    )
                };
                points.push((n, p, start));
            }
        }
        points.stable_sort_by(|a, b| {
            perpendicular(&g.cfg, a.1)
                .partial_cmp(&perpendicular(&g.cfg, b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut active = vec![];
        let mut candidate = vec![None; self.nodes.len()];
        for (n, _, start) in points {
            let mut i = self.set_index(&active, n, &g.cfg);
            let value = projection(&g.cfg, self.nodes[n].pos);
            if start {
                if i == active.len() || projection(&g.cfg, self.nodes[active[i]].pos) != value {
                    active.insert(i, n)
                }
                i = self.set_index(&active, n, &g.cfg);
                if i > 0 {
                    candidate[n] = Some(active[i - 1])
                }
                if i + 1 < active.len() {
                    candidate[active[i + 1]] = Some(n)
                }
                continue;
            }
            if i > 0 && candidate[n] == Some(active[i - 1]) {
                self.nodes[n].constraints.push(active[i - 1])
            }
            if i + 1 < active.len() && candidate[active[i + 1]] == Some(n) {
                self.nodes[active[i + 1]].constraints.push(n)
            }
            if i < active.len() && projection(&g.cfg, self.nodes[active[i]].pos) == value {
                active.remove(i);
            }
        }
    }
    fn lowest_parent(&self, n: usize, g: &Graph) -> Option<usize> {
        let parents = g
            .edges
            .iter()
            .filter_map(|&e| {
                let e = &self.edges[e];
                (e.target == n).then_some(e.source)
            })
            .collect::<Vec<_>>();
        let mut result = None;
        let mut best = f64::NEG_INFINITY;
        for &c in &g.nodes {
            if !parents.contains(&c) {
                continue;
            }
            let cnode = &self.nodes[c];
            let center = V::new(
                cnode.pos.x + cnode.size.x / 2.,
                cnode.pos.y + cnode.size.y / 2.,
            );
            let p = projection(&g.cfg, center);
            if result.is_none() || p > best {
                result = Some(c);
                best = p
            }
        }
        result
    }
    fn lowest_dependent(&self, n: usize, c: &Config) -> Option<usize> {
        let mut result = *self.nodes[n].constraints.first()?;
        for &candidate in self.nodes[n].constraints.iter().skip(1) {
            let a = &self.nodes[candidate];
            let b = &self.nodes[result];
            let better = match c.direction.as_str() {
                "LEFT" => a.pos.x < b.pos.x,
                "RIGHT" => a.pos.x + a.size.x > b.pos.x + b.size.x,
                "UP" => a.pos.y < b.pos.y,
                _ => a.pos.y + a.size.y > b.pos.y + b.size.y,
            };
            if better {
                result = candidate
            }
        }
        Some(result)
    }
    pub(super) fn compact(&mut self, g: &Graph) {
        if !g.cfg.compact || g.nodes.is_empty() {
            return;
        }
        let mut levels = vec![];
        for &n in &g.nodes {
            let n = &self.nodes[n];
            if levels.len() <= n.level {
                levels.resize(n.level + 1, (f64::INFINITY, f64::NEG_INFINITY))
            }
            let (p, s) = if g.cfg.horizontal() {
                (n.pos.x, n.size.x)
            } else {
                (n.pos.y, n.size.y)
            };
            levels[n.level].0 = levels[n.level].0.min(p);
            levels[n.level].1 = levels[n.level].1.max(p + s);
        }
        self.constraints(g);
        let mut nodes = g.nodes.clone();
        nodes.stable_sort_by(|&a, &b| {
            projection(&g.cfg, self.nodes[a].pos)
                .partial_cmp(&projection(&g.cfg, self.nodes[b].pos))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for n in nodes {
            if self.nodes[n].root {
                continue;
            }
            let dependent = self.lowest_dependent(n, &g.cfg);
            let parent = self.lowest_parent(n, g);
            let (mut new, mut end) = (0., 0.);
            if let Some(d) = dependent {
                let d = &self.nodes[d];
                let p =
                    &self.nodes[parent.expect("source dereferences parent of constrained nonroot")];
                let s = self.nodes[n].size;
                match g.cfg.direction.as_str() {
                    "LEFT" => {
                        new = d.pos.x - g.cfg.spacing - s.x;
                        if p.pos.x - g.cfg.spacing - s.x < new {
                            new = p.pos.x - g.cfg.spacing - s.x
                        }
                        end = new + s.x
                    }
                    "RIGHT" => {
                        new = d.pos.x + d.size.x + g.cfg.spacing;
                        if p.pos.x - g.cfg.spacing - s.x < new {
                            new = p.pos.x + p.size.x + g.cfg.spacing
                        }
                        end = new + s.x
                    }
                    "UP" => {
                        new = d.pos.y - g.cfg.spacing - s.y;
                        if p.pos.y - g.cfg.spacing - s.y < new {
                            new = p.pos.y - g.cfg.spacing - s.y
                        }
                        end = new + s.y
                    }
                    _ => {
                        new = d.pos.y + d.size.y + g.cfg.spacing;
                        if p.pos.y + g.cfg.spacing > new {
                            new = p.pos.y + p.size.y + g.cfg.spacing
                        }
                        end = new + s.y
                    }
                }
            } else if let Some(p) = parent {
                let p = &self.nodes[p];
                let s = self.nodes[n].size;
                match g.cfg.direction.as_str() {
                    "LEFT" => {
                        new = p.pos.x - g.cfg.spacing - s.x;
                        end = new + s.x
                    }
                    "RIGHT" => {
                        new = p.pos.x + p.size.x + g.cfg.spacing;
                        end = new + s.x
                    }
                    "UP" => {
                        new = p.pos.y - g.cfg.spacing - s.y;
                        end = new + s.y
                    }
                    _ => {
                        new = p.pos.y + p.size.y + g.cfg.spacing;
                        end = new + s.y
                    }
                }
            }
            let mut chosen = None;
            if g.cfg.route != "AVOID_OVERLAP" {
                chosen = Some(new)
            } else {
                let mut index = levels
                    .iter()
                    .position(|&(min, max)| min <= new && max >= end);
                if index.is_none() {
                    index = (1..levels.len()).find(|&i| {
                        if matches!(g.cfg.direction.as_str(), "LEFT" | "UP") {
                            levels[i].0 <= new
                        } else {
                            levels[i].0 >= new
                        }
                    })
                }
                if let Some(i) = index {
                    chosen = Some(levels[i].0);
                    if i > 0 && i != self.nodes[n].level {
                        self.nodes[n].ascension = true;
                        self.nodes[n].level = i;
                    }
                }
            }
            if let Some(v) = chosen {
                if g.cfg.horizontal() {
                    self.nodes[n].pos.x = v
                } else {
                    self.nodes[n].pos.y = v
                }
            }
        }
    }
}
