// Copyright (c) 2013, 2015, 2017, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// Original mrtree/provider.go: placeNodes through computeLevelCoordinates.
use super::*;
impl Arena {
    fn leftmost(&self, mut level: Vec<usize>, mut depth: i32) -> Option<usize> {
        loop {
            if level.is_empty() {
                return None;
            }
            let next = level
                .iter()
                .flat_map(|&n| self.children(n))
                .collect::<Vec<_>>();
            if depth > 1 {
                level = next;
                depth -= 1;
                continue;
            }
            if depth < 0 && !next.is_empty() {
                level = next;
                continue;
            }
            return Some(level[0]);
        }
    }
    fn mean_width(&self, left: usize, right: usize, horizontal: bool) -> f64 {
        let l = self.nodes[left].size;
        let r = self.nodes[right].size;
        if horizontal {
            l.y / 2. + r.y / 2.
        } else {
            l.x / 2. + r.x / 2.
        }
    }
    fn apportion(&mut self, n: usize, c: &Config) {
        let mut leftmost = self.children(n).first().copied();
        let mut neighbor = leftmost.and_then(|n| self.nodes[n].left);
        let mut depth = 1;
        while let (Some(l), Some(ng)) = (leftmost, neighbor) {
            let (mut left_sum, mut right_sum) = (0., 0.);
            let (mut al, mut an) = (l, ng);
            for _ in 0..depth {
                let (Some(l), Some(n)) = (self.parent(al), self.parent(an)) else {
                    return;
                };
                al = l;
                an = n;
                right_sum += self.nodes[al].modifier;
                left_sum += self.nodes[an].modifier;
            }
            let mut distance = self.nodes[ng].prelim
                + left_sum
                + c.spacing
                + self.mean_width(l, ng, c.horizontal())
                - self.nodes[l].prelim
                - right_sum;
            if distance > 0. {
                let mut sibling = Some(n);
                let mut count = 0;
                while let Some(s) = sibling {
                    if s == an {
                        break;
                    }
                    count += 1;
                    sibling = self.nodes[s].left_sib;
                }
                if sibling.is_none() {
                    return;
                }
                let portion = distance / count as f64;
                sibling = Some(n);
                while let Some(s) = sibling {
                    if s == an {
                        break;
                    }
                    self.nodes[s].prelim += distance;
                    self.nodes[s].modifier += distance;
                    distance -= portion;
                    sibling = self.nodes[s].left_sib;
                }
            }
            depth += 1;
            leftmost = if self.nodes[l].out.is_empty() {
                self.leftmost(self.children(n), depth)
            } else {
                self.children(l).first().copied()
            };
            neighbor = leftmost.and_then(|l| self.nodes[l].left);
        }
    }
    fn first_walk(&mut self, n: usize, c: &Config) {
        self.nodes[n].modifier = 0.;
        let left = self.nodes[n].left_sib;
        let children = self.children(n);
        if children.is_empty() {
            self.nodes[n].prelim = left
                .map(|l| self.nodes[l].prelim + c.spacing + self.mean_width(l, n, c.horizontal()))
                .unwrap_or(0.);
            return;
        }
        for &child in &children {
            self.first_walk(child, c)
        }
        let middle =
            (self.nodes[*children.last().unwrap()].prelim + self.nodes[children[0]].prelim) / 2.;
        if let Some(l) = left {
            self.nodes[n].prelim =
                self.nodes[l].prelim + c.spacing + self.mean_width(l, n, c.horizontal());
            self.nodes[n].modifier = self.nodes[n].prelim - middle;
            self.apportion(n, c)
        } else {
            self.nodes[n].prelim = middle
        }
    }
    fn second_walk(&mut self, n: usize, y: f64, modifier: f64, spacing: f64) {
        let x = self.nodes[n].prelim + modifier;
        let yy = y + self.nodes[n].level_height / 2.;
        self.nodes[n].coordinate = ((x + 0.5).floor() as i64, (yy + 0.5).floor() as i64);
        if let Some(&child) = self.children(n).first() {
            self.second_walk(
                child,
                y + self.nodes[n].level_height + spacing,
                modifier + self.nodes[n].modifier,
                spacing,
            )
        }
        if let Some(right) = self.nodes[n].right_sib {
            self.second_walk(right, y, modifier, spacing)
        }
    }
    pub(super) fn place(&mut self, g: &Graph, root: usize) {
        self.first_walk(root, &g.cfg);
        self.second_walk(root, -self.nodes[root].level_height / 2., 0., g.cfg.spacing);
        for &n in &g.nodes {
            let (x, y) = self.nodes[n].coordinate;
            self.nodes[n].pos = V::new(x as f64, y as f64)
        }
    }
    pub(super) fn direction(&mut self, g: &Graph) {
        for &n in &g.nodes {
            let n = &mut self.nodes[n];
            let (x, y) = n.coordinate;
            let (x, y) = match g.cfg.direction.as_str() {
                "UP" => (x, -y),
                "RIGHT" => (y, x),
                "LEFT" => (-y, x),
                _ => (x, y),
            };
            n.coordinate = (x, y);
            n.pos = V::new(x as f64 - n.size.x / 2., y as f64 - n.size.y / 2.);
        }
    }
    pub(super) fn level_coordinates(&mut self, g: &Graph) {
        let mut levels = BTreeMap::<usize, (f64, f64)>::new();
        for &n in &g.nodes {
            let n = &self.nodes[n];
            let (p, s) = if g.cfg.horizontal() {
                (n.pos.x, n.size.x)
            } else {
                (n.pos.y, n.size.y)
            };
            let entry = levels
                .entry(n.level)
                .or_insert((f64::INFINITY, f64::NEG_INFINITY));
            entry.0 = entry.0.min(p);
            entry.1 = entry.1.max(p + s);
        }
        for &n in &g.nodes {
            let n = &mut self.nodes[n];
            (n.level_min, n.level_max) = levels[&n.level];
        }
    }
}
