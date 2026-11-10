// Copyright (c) 2017, 2020 Kiel University and others. EPL-2.0.
// elk-go/internal/radial/provider.go: wedge allocation, ordering and transforms.
use super::*;
use crate::js_trig::{cos, product_sum, sin};
use d2_rs_core::stable_sort::StableSort;
impl Arena {
    fn leaf_count(&self, n: usize) -> usize {
        let children = self.successors(n);
        if children.is_empty() {
            1
        } else {
            children.into_iter().map(|c| self.leaf_count(c)).sum()
        }
    }
    fn wedge_space(&self, n: usize, strategy: &str) -> f64 {
        if strategy == "LEAF_NUMBER" {
            return self.leaf_count(n) as f64;
        }
        let size = self.nodes[n].size;
        // Pinned arm64 Go rounds X² before fusing Y² + X². The two
        // product orders are observably different for fractional sizes.
        let space = product_sum(size.y, size.y, size.x * size.x).sqrt();
        let children = self
            .successors(n)
            .into_iter()
            .map(|c| self.wedge_space(c, strategy))
            .sum::<f64>();
        children.max(space)
    }
    pub fn position_graph(&mut self, root: usize, radius: f64, cfg: &Config) {
        let mut offset = 0.0;
        if cfg.optimizer != "NONE" {
            let mut best = f64::MAX;
            for degree in 0..360 {
                let candidate = degree as f64 * PI / 180.0;
                self.position_nodes(root, root, radius, cfg, 0.0, 0.0, 2.0 * PI, candidate);
                let value = self.evaluate(root, &cfg.optimizer);
                if value < best {
                    best = value;
                    offset = candidate
                }
            }
        }
        self.position_nodes(root, root, radius, cfg, 0.0, 0.0, 2.0 * PI, offset)
    }
    fn position_nodes(
        &mut self,
        root: usize,
        n: usize,
        radius: f64,
        cfg: &Config,
        current_radius: f64,
        min: f64,
        max: f64,
        offset: f64,
    ) {
        let alpha = (min + max) / 2.0 + offset;
        // Go keeps these two products rounded before subtracting the node
        // half-size (the intervening trig call also separates their SSA use).
        let center_x = current_radius * cos(alpha);
        let center_y = current_radius * sin(alpha);
        self.nodes[n].position = V {
            x: center_x - self.nodes[n].size.x / 2.0,
            y: center_y - self.nodes[n].size.y / 2.0,
        };
        let total = self.wedge_space(n, &cfg.wedge);
        let step = (max - min) / total;
        let mut alpha = min;
        let mut children = self.successors(n);
        self.sort_nodes(root, &mut children, &cfg.sorter);
        for child in children {
            let space = self.wedge_space(child, &cfg.wedge);
            let end = product_sum(step, space, alpha);
            self.position_nodes(
                root,
                child,
                radius,
                cfg,
                current_radius + radius,
                alpha,
                end,
                offset,
            );
            alpha = product_sum(step, space, alpha);
        }
    }
    pub fn sort_nodes(&mut self, root: usize, nodes: &mut [usize], strategy: &str) {
        if strategy == "POLAR_COORDINATE" {
            let mut first = self.successors(root);
            self.polar_sort(&mut first, 0.0);
            self.set_polar_ids(&first, 0);
        }
        if strategy == "ID" || strategy == "POLAR_COORDINATE" {
            nodes.stable_sort_by_key(|&n| self.nodes[n].order)
        }
    }
    fn set_polar_ids(&mut self, nodes: &[usize], offset: i32) -> i32 {
        let mut id = offset;
        let mut next_layer = 0;
        for &n in nodes {
            self.nodes[n].order = id;
            id = id.wrapping_add(1);
            let mut children = self.successors(n);
            let center = self.nodes[n].center();
            let mut arc = crate::go_math::atan2(center.y, center.x);
            if arc < 0.0 {
                arc += 2.0 * PI
            }
            let offset = if arc < PI / 4.0 || arc > 7.0 * PI / 4.0 {
                PI
            } else if arc <= 7.0 * PI / 4.0 && arc > 5.0 * PI / 4.0 {
                3.0 * PI / 2.0
            } else if arc <= 5.0 * PI / 4.0 && arc > 3.0 * PI / 4.0 {
                0.0
            } else {
                PI / 2.0
            };
            self.polar_sort(&mut children, offset);
            next_layer = self.set_polar_ids(&children, next_layer);
        }
        id
    }
    fn polar_sort(&self, nodes: &mut [usize], offset: f64) {
        let arc = |n: usize| {
            let p = self.nodes[n].original;
            let mut a = crate::go_math::atan2(p.y, p.x);
            if a < 0.0 {
                a += 2.0 * PI
            }
            a += offset;
            if a > 2.0 * PI {
                a -= 2.0 * PI
            }
            a
        };
        nodes.stable_sort_by(|&a, &b| {
            let (a, b) = (arc(a), arc(b));
            if (a - b).abs() <= 1e-10 {
                std::cmp::Ordering::Equal
            } else if a < b {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        })
    }
    fn evaluate(&mut self, root: usize, strategy: &str) -> f64 {
        let children = self.successors(root);
        let mut result = 0.0;
        match strategy {
            "EDGE_LENGTH" => {
                for child in children {
                    let (a, b) = clipped(
                        self.nodes[root].center(),
                        self.nodes[child].center(),
                        self.nodes[root].size,
                        self.nodes[child].size,
                    );
                    let v = b.sub(a);
                    result += product_sum(v.y, v.y, v.x * v.x).sqrt();
                }
            }
            "EDGE_LENGTH_BY_POSITION" => {
                for child in children {
                    let p = self.nodes[child].original;
                    let root = &self.nodes[root];
                    let source = V {
                        x: root.position.x + p.x + root.size.x / 2.0,
                        y: root.position.y + p.y + root.size.y,
                    };
                    result += self.nodes[child].center().sub(source).length();
                }
            }
            "CROSSING_MINIMIZATION_BY_POSITION" => {
                for first in 0..children.len() {
                    for second in first + 1..children.len() {
                        if self.crossing(root, children[first], children[second]) {
                            result += 1.0
                        }
                    }
                }
            }
            _ => {}
        }
        result
    }
    fn crossing(&mut self, root: usize, first: usize, second: usize) -> bool {
        let root_x = self.nodes[root].center().x;
        let mut line = |n: usize| {
            let center = self.nodes[n].center();
            self.nodes[n].original = self.nodes[n].original.add(V {
                x: root_x,
                y: root_x,
            });
            let p = self.nodes[n].original;
            let slope = (center.y - p.y) / (center.x - p.x);
            let intercept = product_sum(-slope, center.x, center.y);
            (center, p, slope, intercept)
        };
        let (c1, p1, s1, i1) = line(first);
        let (c2, p2, s2, i2) = line(second);
        let x = (i1 - i2) / (s2 - s1);
        !(p1.x < x && c1.x < x
            || x < p1.x && x < c1.x
            || p2.x < x && c2.x < x
            || x < p2.x && x < c2.x)
    }
    pub fn rotate(&mut self, root: usize, cfg: &Config) {
        let mut target = cfg.target_angle;
        if cfg.additional_wedge {
            let outgoing = &self.nodes[root].outgoing;
            if let (Some(&a), Some(&b)) = (outgoing.first(), outgoing.last()) {
                if let (Some(&first), Some(&last)) =
                    (self.edges[a].targets.first(), self.edges[b].targets.first())
                {
                    let first = self.nodes[first].center();
                    let last = self.nodes[last].center();
                    let mut alpha = target;
                    if alpha <= 0.0 {
                        alpha += 2.0 * PI
                    }
                    let dot = product_sum(last.x, first.x, last.y * first.y);
                    let mut wedge = crate::go_math::acos(
                        dot / (crate::go_math::hypot(last.x, last.y)
                            * crate::go_math::hypot(first.x, first.y)),
                    );
                    if wedge <= 0.0 {
                        wedge += 2.0 * PI
                    }
                    let mut alignment = crate::go_math::atan2(last.y, last.x);
                    if alignment <= 0.0 {
                        alignment += 2.0 * PI
                    }
                    target = PI - (alignment - alpha + wedge / 2.0);
                }
            }
        }
        let sine = sin(target);
        let cosine = cos(target);
        for n in self.nodes[0].children.clone() {
            let c = self.nodes[n].center();
            let xc = c.x * cosine;
            let ys = c.y * sine;
            let xs = c.x * sine;
            let yc = c.y * cosine;
            self.nodes[n].position = V {
                x: (xc - ys) - self.nodes[n].size.x / 2.0,
                y: (xs + yc) - self.nodes[n].size.y / 2.0,
            };
        }
    }
    pub fn graph_size(&mut self, root: usize, cfg: &Config) -> (f64, f64) {
        let mut min = V {
            x: f64::MAX,
            y: f64::MAX,
        };
        let mut max = V {
            x: f64::from_bits(1),
            y: f64::from_bits(1),
        };
        for &n in &self.nodes[0].children {
            let node = &self.nodes[n];
            let m = node.margins;
            min.x = min.x.min(node.position.x - m[3]);
            min.y = min.y.min(node.position.y - m[0]);
            max.x = max.x.max(node.position.x + node.size.x + m[1]);
            max.y = max.y.max(node.position.y + node.size.y + m[2]);
        }
        let p = cfg.padding;
        let mut offset = V {
            x: min.x - p[3],
            y: min.y - p[0],
        };
        let mut width = max.x - min.x + p[3] + p[1];
        let mut height = max.y - min.y + p[0] + p[2];
        if cfg.center {
            let n = &self.nodes[root];
            let m = n.margins;
            let x = n.position.x + n.size.x / 2.0 + (m[3] + m[1]) / 2.0 - offset.x;
            let y = n.position.y + n.size.y / 2.0 + (m[0] + m[2]) / 2.0 - offset.y;
            let dx = width - x;
            let dy = height - y;
            if dx < width / 2.0 {
                let additional = dx - x;
                width += additional;
                offset.x -= additional
            } else {
                width += x - dx
            }
            if dy < height / 2.0 {
                let additional = dy - y;
                height += additional;
                offset.y -= additional
            } else {
                height += y - dy
            }
        }
        for n in self.nodes[0].children.clone() {
            self.nodes[n].position = self.nodes[n].position.sub(offset)
        }
        (width, height)
    }
}
