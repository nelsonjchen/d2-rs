// Copyright (c) 2008, 2020; 2011, 2015; 2011, 2020; 2015, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// elk-go v0.2.0/internal/force/{graph,importer,components,provider}.go.
mod logarithm;
mod model;
mod stress;
use crate::layered::JavaRandom;
use crate::property::{enum_value, flag, integer, number};
use crate::utility::{layout_default_children, minimum, option, padding};
use crate::{ElkEdgeSection, ElkGraph, ElkOptions, ElkPoint, LayeringError};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Default, Debug)]
struct V {
    x: f64,
    y: f64,
}
impl V {
    fn add(self, b: Self) -> Self {
        Self {
            x: self.x + b.x,
            y: self.y + b.y,
        }
    }
    fn sub(self, b: Self) -> Self {
        Self {
            x: self.x - b.x,
            y: self.y - b.y,
        }
    }
    fn scale(self, s: f64) -> Self {
        Self {
            x: self.x * s,
            y: self.y * s,
        }
    }
    fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    fn point(self) -> ElkPoint {
        ElkPoint {
            x: self.x,
            y: self.y,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Node(usize),
    Label(usize, usize),
    Bend(usize),
}
struct Particle {
    position: V,
    size: V,
    displacement: V,
    priority: i32,
    kind: Kind,
    inline: bool,
    spacing: f64,
}
struct Edge {
    origin: usize,
    source: usize,
    target: usize,
    priority: i32,
    power: i32,
    labels: Vec<usize>,
    bends: Vec<usize>,
}
#[derive(Default)]
struct Component {
    nodes: Vec<usize>,
    edges: Vec<usize>,
    labels: Vec<usize>,
    bends: Vec<usize>,
}
struct Arena {
    particles: Vec<Particle>,
    edges: Vec<Edge>,
    nodes: usize,
}
struct Config {
    model: String,
    iterations: i32,
    temperature: f64,
    repulsion: f64,
    spacing: f64,
    aspect: f64,
    separate: bool,
    interactive: bool,
    padding: [f64; 4],
    random: JavaRandom,
}
impl Config {
    fn new(options: &Value) -> Self {
        let seed = integer(options, "randomSeed").unwrap_or(1);
        let seed = if seed == 0 {
            web_time::SystemTime::now()
                .duration_since(web_time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as i64)
                .unwrap_or_else(|e| -(e.duration().as_nanos() as i64))
        } else {
            i64::from(seed)
        };
        Self {
            model: enum_value(
                options,
                "force.model",
                &["EADES", "FRUCHTERMAN_REINGOLD"],
                "FRUCHTERMAN_REINGOLD",
            ),
            iterations: integer(options, "force.iterations").unwrap_or(300),
            temperature: number(options, "force.temperature", 0.001),
            repulsion: number(options, "force.repulsion", 5.0),
            spacing: number(options, "spacing.nodeNode", 80.0),
            aspect: number(options, "aspectRatio", 1.6),
            separate: flag(options, "separateConnectedComponents", true),
            interactive: flag(options, "interactive", false),
            padding: if option(options, "padding").is_some() {
                padding(options)
            } else {
                [50.0; 4]
            },
            random: JavaRandom::with_stream(seed, "elk-force"),
        }
    }
}
fn priority(options: Option<&Value>) -> i32 {
    options.and_then(|v| integer(v, "priority")).unwrap_or(1)
}
impl Arena {
    fn import(graph: &ElkGraph) -> Result<Self, LayeringError> {
        let mut arena = Self {
            particles: vec![],
            edges: vec![],
            nodes: graph.children.len(),
        };
        let mut ids = BTreeMap::new();
        for (i, node) in graph.children.iter().enumerate() {
            ids.insert(node.id.as_str(), i);
            for p in &node.ports {
                ids.insert(p.id.as_str(), i);
            }
            arena.particles.push(Particle {
                position: V {
                    x: node.x + node.width / 2.0,
                    y: node.y + node.height / 2.0,
                },
                size: V {
                    x: node.width.max(1.0),
                    y: node.height.max(1.0),
                },
                displacement: V::default(),
                priority: priority(node.layout_options.as_ref()),
                kind: Kind::Node(i),
                inline: false,
                spacing: 0.0,
            });
        }
        for source in 0..graph.children.len() {
            for (origin, edge) in graph.edges.iter().enumerate() {
                if edge
                    .sources
                    .first()
                    .and_then(|s| ids.get(s.as_str()))
                    .copied()
                    != Some(source)
                {
                    continue;
                }
                if edge.sources.len() != 1 || edge.targets.len() != 1 {
                    return Err(LayeringError::Import(
                        "graph must not contain hyperedges".into(),
                    ));
                }
                let Some(&target) = ids.get(edge.targets[0].as_str()) else {
                    continue;
                };
                if source == target {
                    continue;
                }
                let index = arena.edges.len();
                let mut labels = vec![];
                for (li, label) in edge.labels.iter().enumerate() {
                    let pid = arena.particles.len();
                    arena.particles.push(Particle {
                        position: V::default(),
                        size: V {
                            x: label.width.max(1.0),
                            y: label.height.max(1.0),
                        },
                        displacement: V::default(),
                        priority: priority(label.layout_options.as_ref()),
                        kind: Kind::Label(index, li),
                        inline: label
                            .layout_options
                            .as_ref()
                            .is_some_and(|o| flag(o, "edgeLabels.inline", false)),
                        spacing: edge
                            .layout_options
                            .as_ref()
                            .map(|o| number(o, "spacing.edgeLabel", 5.0))
                            .unwrap_or(5.0),
                    });
                    labels.push(pid);
                }
                arena.edges.push(Edge {
                    origin,
                    source,
                    target,
                    priority: priority(edge.layout_options.as_ref()),
                    power: edge
                        .layout_options
                        .as_ref()
                        .and_then(|o| integer(o, "force.repulsivePower"))
                        .unwrap_or(0),
                    labels,
                    bends: vec![],
                });
                for label in arena.edges[index].labels.clone() {
                    arena.refresh(label);
                }
            }
        }
        Ok(arena)
    }
    fn clip(&self, edge: usize, source: bool) -> V {
        let e = &self.edges[edge];
        let (a, b) = if source {
            (e.source, e.target)
        } else {
            (e.target, e.source)
        };
        let p = &self.particles[a];
        let vector = self.particles[b].position.sub(p.position);
        let sx = if vector.x.abs() > p.size.x / 2.0 {
            (p.size.x / 2.0) / vector.x.abs()
        } else {
            1.0
        };
        let sy = if vector.y.abs() > p.size.y / 2.0 {
            (p.size.y / 2.0) / vector.y.abs()
        } else {
            1.0
        };
        p.position.add(vector.scale(sx.min(sy)))
    }
    fn refresh(&mut self, label: usize) {
        let Kind::Label(e, _) = self.particles[label].kind else {
            unreachable!()
        };
        let edge = &self.edges[e];
        let source = self.particles[edge.source].position;
        let target = self.particles[edge.target].position;
        let p = &mut self.particles[label];
        p.position = if p.inline {
            source
                .add(target.sub(source).scale(0.5))
                .sub(p.size.scale(0.5))
        } else {
            let spacing = p.spacing;
            let (x, y) = if source.x >= target.x {
                if source.y >= target.y {
                    (
                        target.x + (source.x - target.x) / 2.0 + spacing,
                        target.y + (source.y - target.y) / 2.0 - spacing - p.size.y,
                    )
                } else {
                    (
                        target.x + (source.x - target.x) / 2.0 + spacing,
                        source.y + (target.y - source.y) / 2.0 + spacing,
                    )
                }
            } else if source.y >= target.y {
                (
                    source.x + (target.x - source.x) / 2.0 + spacing,
                    target.y + (source.y - target.y) / 2.0 + spacing,
                )
            } else {
                (
                    source.x + (target.x - source.x) / 2.0 + spacing,
                    source.y + (target.y - source.y) / 2.0 - spacing - p.size.y,
                )
            };
            V { x, y }
        };
    }
    fn distribute(&mut self, edge: usize) {
        let e = &self.edges[edge];
        if e.bends.is_empty() {
            return;
        }
        let factor = 1.0 / (e.bends.len() + 1) as f64;
        let step = self.particles[e.target]
            .position
            .sub(self.particles[e.source].position)
            .scale(factor);
        let mut p = self.particles[e.source].position;
        for &b in &e.bends {
            p = p.add(step);
            self.particles[b].position = p;
        }
    }
    fn split(&self, separate: bool) -> Vec<Component> {
        if !separate {
            return vec![Component {
                nodes: (0..self.nodes).collect(),
                edges: (0..self.edges.len()).collect(),
                labels: self
                    .edges
                    .iter()
                    .flat_map(|e| e.labels.iter().copied())
                    .collect(),
                bends: vec![],
            }];
        }
        let mut incidence = vec![vec![]; self.nodes];
        for (i, e) in self.edges.iter().enumerate() {
            incidence[e.source].push(i);
            incidence[e.target].push(i);
        }
        fn dfs(
            a: &Arena,
            n: usize,
            last: Option<usize>,
            incidence: &[Vec<usize>],
            seen: &mut [bool],
            c: &mut Component,
        ) {
            if seen[n] {
                return;
            }
            seen[n] = true;
            c.nodes.push(n);
            for &id in &incidence[n] {
                let e = &a.edges[id];
                if Some(e.source) == last || Some(e.target) == last {
                    continue;
                }
                if e.source != n {
                    dfs(a, e.source, Some(n), incidence, seen, c);
                }
                if e.target != n {
                    dfs(a, e.target, Some(n), incidence, seen, c);
                }
                c.edges.push(id);
                c.labels.extend_from_slice(&e.labels);
            }
        }
        let mut seen = vec![false; self.nodes];
        let mut out = vec![];
        for n in 0..self.nodes {
            if !seen[n] {
                let mut c = Component::default();
                dfs(self, n, None, &incidence, &mut seen, &mut c);
                out.push(c);
            }
        }
        out
    }
    fn bounds(&self, c: &Component, with_bends: bool) -> (V, V) {
        let (mut min, mut max) = (
            V {
                x: i32::MAX as f64,
                y: i32::MAX as f64,
            },
            V {
                x: i32::MIN as f64,
                y: i32::MIN as f64,
            },
        );
        for &id in c.nodes.iter().chain(if with_bends {
            c.bends.iter()
        } else {
            [].iter()
        }) {
            let p = &self.particles[id];
            min.x = min.x.min(p.position.x - p.size.x / 2.0);
            min.y = min.y.min(p.position.y - p.size.y / 2.0);
            max.x = max.x.max(p.position.x + p.size.x / 2.0);
            max.y = max.y.max(p.position.y + p.size.y / 2.0);
        }
        (min, max)
    }
    fn recombine(&mut self, mut components: Vec<Component>, cfg: &Config) -> Component {
        if components.len() == 1 {
            return components.pop().unwrap();
        }
        if components.is_empty() {
            return Component::default();
        }
        let mut infos: Vec<_> = components
            .into_iter()
            .map(|c| {
                let (min, max) = self.bounds(&c, false);
                let priority: i64 = c
                    .nodes
                    .iter()
                    .map(|&n| i64::from(self.particles[n].priority))
                    .sum();
                (c, min, max.sub(min), priority)
            })
            .collect();
        infos.sort_by(|a, b| {
            b.3.cmp(&a.3).then_with(|| {
                (a.2.x * a.2.y)
                    .partial_cmp(&(b.2.x * b.2.y))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        let (mut max_width, mut area) = (0.0_f64, 0.0);
        for (_, _, size, _) in &infos {
            max_width = max_width.max(size.x);
            area += size.x * size.y;
        }
        max_width = max_width.max((area.sqrt() as f32) as f64 * cfg.aspect);
        let (mut x, mut y, mut highest) = (0.0, 0.0, 0.0_f64);
        let mut result = Component::default();
        for (c, min, size, _) in infos {
            if x + size.x > max_width {
                x = 0.0;
                y += highest + cfg.spacing;
                highest = 0.0;
            }
            let offset = V { x, y }.sub(min);
            for &n in &c.nodes {
                self.particles[n].position = self.particles[n].position.add(offset);
            }
            for &e in &c.edges {
                for &b in &self.edges[e].bends {
                    self.particles[b].position = self.particles[b].position.add(offset);
                }
            }
            for &l in &c.labels {
                self.particles[l].position = self.particles[l].position.add(offset);
            }
            highest = highest.max(size.y);
            x += size.x + cfg.spacing;
            result.nodes.extend(c.nodes);
            result.edges.extend(c.edges);
            result.labels.extend(c.labels);
        }
        result
    }
    fn apply(&self, graph: &mut ElkGraph, c: &Component, cfg: &Config, honor_minimum: bool) {
        let (min, max) = self.bounds(c, true);
        let offset = V {
            x: cfg.padding[1] - min.x,
            y: cfg.padding[0] - min.y,
        };
        for &n in &c.nodes {
            let Kind::Node(origin) = self.particles[n].kind else {
                unreachable!()
            };
            let position = self.particles[n].position.add(offset);
            let node = &mut graph.children[origin];
            node.x = position.x - node.width / 2.0;
            node.y = position.y - node.height / 2.0;
        }
        for &e in &c.edges {
            let fe = &self.edges[e];
            let start = self.clip(e, true).add(offset).point();
            let end = self.clip(e, false).add(offset).point();
            let edge = &mut graph.edges[fe.origin];
            if edge.sections.is_empty() {
                edge.sections.push(ElkEdgeSection {
                    id: Some(format!("{}_s0", edge.id)),
                    start,
                    end,
                    bends: vec![],
                    incoming_shape: None,
                    outgoing_shape: None,
                });
            }
            let s = &mut edge.sections[0];
            s.start = start;
            s.end = end;
            s.bends = fe
                .bends
                .iter()
                .map(|&b| self.particles[b].position.add(offset).point())
                .collect();
        }
        for &l in &c.labels {
            let Kind::Label(e, index) = self.particles[l].kind else {
                unreachable!()
            };
            let p = self.particles[l].position.add(offset);
            let label = &mut graph.edges[self.edges[e].origin].labels[index];
            label.x = p.x;
            label.y = p.y;
        }
        if !flag(&graph.layout_options, "nodeSize.fixedGraphSize", false) {
            let minimum = if honor_minimum {
                minimum(&graph.layout_options)
            } else {
                (0.0, 0.0)
            };
            graph.width = Some((max.x - min.x + (cfg.padding[1] + cfg.padding[3])).max(minimum.0));
            graph.height = Some((max.y - min.y + (cfg.padding[0] + cfg.padding[2])).max(minimum.1));
        }
    }
}
pub(crate) fn layout(
    graph: &mut ElkGraph,
    options: ElkOptions,
    is_stress: bool,
) -> Result<(), LayeringError> {
    if graph.children.is_empty() {
        graph.x = Some(graph.x.unwrap_or(0.0));
        graph.y = Some(graph.y.unwrap_or(0.0));
        graph.width = Some(graph.width.unwrap_or(0.0));
        graph.height = Some(graph.height.unwrap_or(0.0));
        return Ok(());
    }
    let consumed = layout_default_children(graph, options)?;
    let interactive = flag(&graph.layout_options, "interactive", false);
    if !flag(&graph.layout_options, "omitNodeMicroLayout", false) {
        crate::rectpacking::micro::apply(graph, &consumed);
    }
    if !is_stress || !interactive {
        let mut cfg = Config::new(&graph.layout_options);
        let mut a = Arena::import(graph)?;
        let mut components = a.split(cfg.separate);
        for c in &mut components {
            a.force(c, &mut cfg);
        }
        let combined = a.recombine(components, &cfg);
        a.apply(graph, &combined, &cfg, true);
    }
    if is_stress {
        let cfg = Config::new(&graph.layout_options);
        let mut a = Arena::import(graph)?;
        let components = a.split(cfg.separate);
        for c in &components {
            if c.nodes.len() > 1 {
                a.majorize(c, graph);
                for &l in &c.labels {
                    a.refresh(l);
                }
            }
        }
        let combined = a.recombine(components, &cfg);
        a.apply(graph, &combined, &cfg, interactive);
    }
    graph.x = Some(graph.x.unwrap_or(0.0));
    graph.y = Some(graph.y.unwrap_or(0.0));
    Ok(())
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests;
