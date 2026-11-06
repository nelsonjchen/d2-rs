// Copyright (c) 2011, 2015 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// elk-go/internal/force/model.go: particle identity, mutation and iteration order.
use super::*;
impl Arena {
    fn avoid_same_position(&mut self, a: usize, b: usize, cfg: &mut Config) {
        while self.particles[a].position.x - self.particles[b].position.x == 0.0
            && self.particles[a].position.y - self.particles[b].position.y == 0.0
        {
            if let (Kind::Bend(ae), Kind::Bend(be)) =
                (self.particles[a].kind, self.particles[b].kind)
            {
                let vector = self.clip(ae, false).sub(self.clip(ae, true));
                let len = vector.length();
                self.particles[a].position = self.particles[a].position.add(V {
                    x: vector.x / len * 2.0,
                    y: -vector.y / len * 2.0,
                });
                let vector = self.clip(be, false).sub(self.clip(be, true));
                let len = vector.length();
                self.particles[a].position = self.particles[a].position.add(V {
                    x: vector.x / len * 2.0,
                    y: -vector.y / len * 2.0,
                });
            } else {
                self.particles[a].position.x += cfg.random.next_double() - 0.5;
                self.particles[a].position.y += cfg.random.next_double() - 0.5;
                self.particles[b].position.x += cfg.random.next_double() - 0.5;
                self.particles[b].position.y += cfg.random.next_double() - 0.5;
            }
        }
    }
    pub(super) fn force(&mut self, c: &mut Component, cfg: &mut Config) {
        let mut adjacency = vec![vec![0_i64; self.nodes]; self.nodes];
        for &e in &c.edges {
            let e = &self.edges[e];
            adjacency[e.source][e.target] += i64::from(e.priority);
        }
        let bound = ((c.nodes.len() * 16 + c.edges.len()) as f64).max(256.0);
        if !cfg.interactive {
            for &n in &c.nodes {
                self.particles[n].position = V {
                    x: cfg.random.next_double() * c.nodes.len() as f64,
                    y: cfg.random.next_double() * c.nodes.len() as f64,
                };
            }
        }
        for &e in &c.edges {
            for _ in 0..self.edges[e].power {
                let id = self.particles.len();
                self.particles.push(Particle {
                    position: V::default(),
                    size: V::default(),
                    displacement: V::default(),
                    priority: 1,
                    kind: Kind::Bend(e),
                    inline: false,
                    spacing: 0.0,
                });
                self.edges[e].bends.push(id);
                c.bends.push(id);
            }
            self.distribute(e);
        }
        let threshold = cfg.temperature / cfg.iterations as f64;
        let mut temperature = cfg.temperature;
        let mut width = 0.0;
        let mut height = 0.0;
        for &n in &c.nodes {
            width += self.particles[n].size.x;
            height += self.particles[n].size.y;
        }
        let area = width * height;
        let k = (area / (2 * c.nodes.len()) as f64).sqrt() * (cfg.spacing * 0.01);
        let eades = cfg.model == "EADES";
        let mut iteration = 0;
        while if eades {
            iteration < cfg.iterations
        } else {
            temperature > 0.0
        } {
            for &e in &c.edges {
                for l in self.edges[e].labels.clone() {
                    self.refresh(l);
                }
                self.distribute(e);
            }
            if !eades {
                temperature -= threshold;
            }
            let particles: Vec<_> = c
                .nodes
                .iter()
                .chain(&c.labels)
                .chain(&c.bends)
                .copied()
                .collect();
            for &forcee in &particles {
                for &forcer in &particles {
                    if forcer == forcee {
                        continue;
                    }
                    self.avoid_same_position(forcer, forcee, cfg);
                    let first = &self.particles[forcer];
                    let second = &self.particles[forcee];
                    let displacement = second.position.sub(first.position);
                    let length = displacement.length();
                    let distance =
                        (length - first.size.length() / 2.0 - second.size.length() / 2.0).max(0.0);
                    let connection = match (first.kind, second.kind) {
                        (Kind::Node(_), Kind::Node(_)) => {
                            adjacency[forcer][forcee] + adjacency[forcee][forcer]
                        }
                        (Kind::Bend(a), Kind::Bend(b)) if a == b => {
                            i64::from(self.edges[a].priority)
                        }
                        _ => 0,
                    };
                    let scale = if eades {
                        let force = if connection > 0 {
                            -(if distance > 0.0 {
                                super::logarithm::log(distance / cfg.spacing)
                            } else {
                                -100.0
                            }) * connection as f64
                        } else {
                            (if distance > 0.0 {
                                cfg.repulsion / (distance * distance)
                            } else {
                                cfg.repulsion * 100.0
                            }) * first.priority as f64
                        };
                        force / length
                    } else {
                        let ks = k * k;
                        let repulsion = if distance > 0.0 {
                            ks / distance
                        } else {
                            ks * 100.0
                        };
                        let mut force = repulsion * first.priority as f64;
                        if connection > 0 {
                            let attraction = ((distance * distance) / k) * connection as f64;
                            force -= attraction;
                        }
                        let scaled = force * temperature;
                        scaled / length
                    };
                    let delta = displacement.scale(scale);
                    self.particles[forcee].displacement =
                        self.particles[forcee].displacement.add(delta);
                }
            }
            for id in particles {
                let d = self.particles[id].displacement;
                let limit = |v: f64| {
                    if v.is_nan() {
                        v
                    } else {
                        v.max(-bound).min(bound)
                    }
                };
                self.particles[id].position = self.particles[id].position.add(V {
                    x: limit(d.x),
                    y: limit(d.y),
                });
                self.particles[id].displacement = V::default();
            }
            iteration += 1;
        }
    }
}
