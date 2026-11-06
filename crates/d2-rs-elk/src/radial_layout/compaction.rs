// Copyright (c) 2017, 2020 Kiel University and others. EPL-2.0.
// elk-go/internal/radial/provider.go: source radial mover and compactors.
use super::*;
impl Arena {
    fn contract_layer(&mut self, root: usize, nodes: &[usize], contracting: bool, step: i32) {
        let center = self.nodes[root].center();
        for &n in nodes {
            let c = self.nodes[n].center();
            let delta = c.sub(center);
            let factor = step as f64 / delta.length();
            let direction = if contracting { -1.0 } else { 1.0 };
            let c = V {
                x: crate::js_trig::product_sum(direction * delta.x, factor, c.x),
                y: crate::js_trig::product_sum(direction * delta.y, factor, c.y),
            };
            self.nodes[n].position = c.sub(self.nodes[n].size.scale(0.5));
        }
    }
    fn move_node(&mut self, root: usize, n: usize, distance: f64) {
        let delta = self.nodes[n].center().sub(self.nodes[root].center());
        let len = delta.length();
        self.nodes[n].position = V {
            x: crate::js_trig::product_sum(delta.x / len, distance, self.nodes[n].position.x),
            y: crate::js_trig::product_sum(delta.y / len, distance, self.nodes[n].position.y),
        };
    }
    fn overlap(&self, a: usize, b: usize, spacing: f64) -> bool {
        let a = &self.nodes[a];
        let b = &self.nodes[b];
        let x1 = a.position.x - spacing / 2.0;
        let x2 = b.position.x - spacing / 2.0;
        let y1 = a.position.y - spacing / 2.0;
        let y2 = b.position.y - spacing / 2.0;
        let w1 = a.size.x + spacing;
        let w2 = b.size.x + spacing;
        let h1 = a.size.y + spacing;
        let h2 = b.size.y + spacing;
        x1 < x2 + w2 && x2 < x1 && y1 < y2 + h2 && y2 < y1
            || x2 < x1 + w1 && x1 < x2 && y2 < y1 + h1 && y1 < y2
            || x1 < x2 + w2 && x2 < x1 && y1 < y2 && y2 < y1 + h1
            || x2 < x1 + w1 && x1 < x2 && y1 < y2 + h2 && y2 < y1
    }
    fn overlap_layer(&self, nodes: &[usize], spacing: f64) -> bool {
        nodes.len() > 1
            && (0..nodes.len())
                .any(|i| self.overlap(nodes[i], nodes[(i + 1) % nodes.len()], spacing))
    }
    pub fn remove_overlaps(&mut self, root: usize, cfg: &Config) {
        let nodes = self.successors(root);
        self.extend(root, nodes, cfg)
    }
    fn extend(&mut self, root: usize, nodes: Vec<usize>, cfg: &Config) {
        if nodes.is_empty() {
            return;
        }
        let old = self.nodes[nodes[0]].position;
        while self.overlap_layer(&nodes, cfg.spacing) {
            self.contract_layer(root, &nodes, false, 1)
        }
        let distance = self.nodes[nodes[0]].position.sub(old).length();
        let mut next = vec![];
        for &n in &nodes {
            for c in self.successors(n) {
                if !next.contains(&c) {
                    next.push(c)
                }
            }
        }
        for &n in &next {
            self.move_node(root, n, distance)
        }
        self.sort_nodes(root, &mut next, &cfg.sorter);
        self.extend(root, next, cfg)
    }
    pub fn radial_compact(&mut self, root: usize, cfg: &Config) {
        let mut last_radius = 0.0;
        let nodes = self.successors(root);
        self.compact_layer(root, nodes, cfg, &mut last_radius)
    }
    fn radius(&self, root: usize, n: usize) -> f64 {
        self.nodes[n]
            .position
            .sub(self.nodes[root].position)
            .length()
    }
    fn compact_layer(
        &mut self,
        root: usize,
        mut nodes: Vec<usize>,
        cfg: &Config,
        last_radius: &mut f64,
    ) {
        if nodes.is_empty() {
            return;
        }
        self.sort_nodes(root, &mut nodes, &cfg.sorter);
        let mut contracted = false;
        loop {
            let overlapping = self.overlap_layer(&nodes, cfg.spacing)
                || nodes.iter().any(|&n| {
                    self.tree_parent(n)
                        .is_some_and(|p| self.overlap(n, p, cfg.spacing))
                        || self.radius(root, n) - cfg.spacing <= *last_radius
                });
            if overlapping {
                break;
            }
            self.contract_layer(root, &nodes, true, cfg.step);
            contracted = true;
        }
        if contracted {
            self.contract_layer(root, &nodes, false, cfg.step)
        }
        let next = nodes.iter().flat_map(|&n| self.successors(n)).collect();
        *last_radius = self.radius(root, nodes[0]);
        self.compact_layer(root, next, cfg, last_radius)
    }
    pub fn wedge_compact(&mut self, root: usize, cfg: &Config) {
        let mut first = self.successors(root);
        self.sort_nodes(root, &mut first, &cfg.sorter);
        let mut left = BTreeMap::<usize, Vec<usize>>::new();
        let mut right = BTreeMap::<usize, Vec<usize>>::new();
        for &parent in &first {
            let mut lc = vec![parent];
            let mut rc = vec![parent];
            let mut level = self.successors(parent);
            while !level.is_empty() {
                self.sort_nodes(root, &mut level, &cfg.sorter);
                lc.push(level[0]);
                rc.push(*level.last().unwrap());
                level = level.iter().flat_map(|&n| self.successors(n)).collect();
            }
            left.insert(parent, lc);
            right.insert(parent, rc);
        }
        for _ in 0..2 {
            for (i, &n) in first.iter().enumerate() {
                let l = first[(i + first.len() - 1) % first.len()];
                let rr = first[(i + 1) % first.len()];
                self.compact_wedge(root, vec![n], cfg, &right[&l], &left[&rr]);
            }
        }
    }
    fn compact_wedge(
        &mut self,
        root: usize,
        mut nodes: Vec<usize>,
        cfg: &Config,
        left: &[usize],
        right: &[usize],
    ) {
        if nodes.is_empty() {
            return;
        }
        self.sort_nodes(root, &mut nodes, &cfg.sorter);
        let mut predecessors = vec![];
        for &n in &nodes {
            if let Some(p) = self.tree_parent(n) {
                if !predecessors.contains(&p) {
                    predecessors.push(p)
                }
            }
        }
        let mut contracted = false;
        loop {
            let overlapping = left.iter().any(|&n| self.overlap(nodes[0], n, cfg.spacing))
                || right
                    .iter()
                    .any(|&n| self.overlap(*nodes.last().unwrap(), n, cfg.spacing))
                || self.overlap_layer(&nodes, cfg.spacing)
                || nodes.iter().any(|&n| {
                    predecessors
                        .iter()
                        .any(|&p| self.overlap(n, p, cfg.spacing))
                });
            if overlapping {
                break;
            }
            self.contract_layer(root, &nodes, true, cfg.step);
            contracted = true;
        }
        if contracted {
            self.contract_layer(root, &nodes, false, cfg.step)
        }
        let next = nodes.iter().flat_map(|&n| self.successors(n)).collect();
        self.compact_wedge(root, next, cfg, left, right)
    }
}
