// Copyright (c) 2013, 2015, 2017, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// Pinned elk-go/internal/mrtree/{model,options,provider}.go.
mod compaction;
mod placement;
mod routing;
mod topology;
use crate::property::{enum_value, flag, integer, number};
use crate::utility::{option, padding};
use crate::{ElkEdgeSection, ElkGraph, ElkOptions, ElkPoint, LayeringError};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Default, Debug)]
struct V {
    x: f64,
    y: f64,
}
impl V {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    fn add(self, b: Self) -> Self {
        Self::new(self.x + b.x, self.y + b.y)
    }
    fn point(self) -> ElkPoint {
        ElkPoint {
            x: self.x,
            y: self.y,
        }
    }
}
#[derive(Clone)]
struct Config {
    padding: [f64; 4],
    spacing: f64,
    edge_space: f64,
    texture: f64,
    aspect: f64,
    separate: bool,
    search: String,
    weight: String,
    direction: String,
    route: String,
    compact: bool,
    fixed: bool,
}
impl Config {
    fn new(o: &Value) -> Self {
        Self {
            padding: if option(o, "padding").is_some() {
                padding(o)
            } else {
                [20.; 4]
            },
            spacing: number(o, "spacing.nodeNode", 20.),
            edge_space: number(o, "spacing.edgeNode", 3.),
            texture: number(o, "mrtree.edgeEndTextureLength", 7.),
            aspect: number(o, "aspectRatio", 1.6),
            separate: flag(o, "separateConnectedComponents", true),
            search: enum_value(o, "mrtree.searchOrder", &["DFS", "BFS"], "DFS"),
            weight: enum_value(
                o,
                "mrtree.weighting",
                &["MODEL_ORDER", "DESCENDANTS", "FAN", "CONSTRAINT"],
                "MODEL_ORDER",
            ),
            direction: match enum_value(
                o,
                "direction",
                &["UNDEFINED", "RIGHT", "LEFT", "DOWN", "UP"],
                "UNDEFINED",
            )
            .as_str()
            {
                "UNDEFINED" => "DOWN".into(),
                s => s.into(),
            },
            route: enum_value(
                o,
                "mrtree.edgeRoutingMode",
                &["NONE", "MIDDLE_TO_MIDDLE", "AVOID_OVERLAP"],
                "AVOID_OVERLAP",
            ),
            compact: flag(o, "mrtree.compaction", false),
            fixed: flag(o, "nodeSize.fixedGraphSize", false),
        }
    }
    fn horizontal(&self) -> bool {
        matches!(self.direction.as_str(), "LEFT" | "RIGHT")
    }
}
#[derive(Default)]
struct Node {
    origin: Option<usize>,
    pos: V,
    size: V,
    props: Value,
    out: Vec<usize>,
    inc: Vec<usize>,
    root: bool,
    dummy: bool,
    fan: usize,
    desc: usize,
    left: Option<usize>,
    right: Option<usize>,
    left_sib: Option<usize>,
    right_sib: Option<usize>,
    position_index: usize,
    prelim: f64,
    modifier: f64,
    level_height: f64,
    level: usize,
    level_min: f64,
    level_max: f64,
    constraints: Vec<usize>,
    ascension: bool,
    coordinate: (i64, i64),
}
struct Edge {
    origin: Option<usize>,
    source: usize,
    target: usize,
    bends: Vec<V>,
}
struct Arena {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}
#[derive(Clone)]
struct Graph {
    nodes: Vec<usize>,
    edges: Vec<usize>,
    cfg: Config,
    min: V,
    max: V,
}
impl Graph {
    fn settings(&self) -> Self {
        Self {
            nodes: vec![],
            edges: vec![],
            cfg: self.cfg.clone(),
            min: V::default(),
            max: V::default(),
        }
    }
}
impl Arena {
    fn children(&self, n: usize) -> Vec<usize> {
        self.nodes[n]
            .out
            .iter()
            .map(|&e| self.edges[e].target)
            .collect()
    }
    fn parent(&self, n: usize) -> Option<usize> {
        self.nodes[n].inc.first().map(|&e| self.edges[e].source)
    }
    fn descendants(&self, n: usize) -> Vec<usize> {
        let mut v = vec![n];
        let mut i = 0;
        while i < v.len() {
            v.extend(self.children(v[i]));
            i += 1;
        }
        v
    }
    fn bounds(&self, g: &mut Graph) {
        if g.nodes.is_empty() {
            g.min = V::default();
            g.max = V::default();
            return;
        }
        g.min = V::new(f64::INFINITY, f64::INFINITY);
        g.max = V::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for &n in &g.nodes {
            let n = &self.nodes[n];
            g.min.x = g.min.x.min(n.pos.x);
            g.min.y = g.min.y.min(n.pos.y);
            g.max.x = g.max.x.max(n.pos.x + n.size.x);
            g.max.y = g.max.y.max(n.pos.y + n.size.y);
        }
    }
    fn offset(&mut self, g: &mut Graph, v: V) {
        for &n in &g.nodes {
            self.nodes[n].pos = self.nodes[n].pos.add(v)
        }
        for &e in &g.edges {
            for b in &mut self.edges[e].bends {
                *b = b.add(v)
            }
        }
        g.min = g.min.add(v);
        g.max = g.max.add(v);
    }
    fn pack(&mut self, mut components: Vec<Graph>) -> Graph {
        if components.is_empty() {
            unreachable!("empty providers bypassed by engine")
        }
        if components.len() == 1 {
            let mut g = components.remove(0);
            let v = V::new(g.cfg.padding[1] - g.min.x, g.cfg.padding[0] - g.min.y);
            self.offset(&mut g, v);
            return g;
        }
        // Source packing bounds include dummy roots but deliberately exclude routes.
        for g in &mut components {
            self.bounds(g)
        }
        let priority = |g: &Graph| -> i64 {
            g.nodes
                .iter()
                .map(|&n| integer(&self.nodes[n].props, "priority").unwrap_or(1) as i64)
                .sum()
        };
        let area = |g: &Graph| (g.max.x - g.min.x) * (g.max.y - g.min.y);
        components.sort_by(|a, b| {
            priority(b).cmp(&priority(a)).then_with(|| {
                area(a)
                    .partial_cmp(&area(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        let mut result = components[0].settings();
        let mut row_width: f64 = 0.;
        let mut area_sum: f64 = 0.;
        for g in &components {
            let w = g.max.x - g.min.x;
            let h = g.max.y - g.min.y;
            row_width = row_width.max(w);
            area_sum = w.mul_add(h, area_sum)
        }
        row_width = row_width.max((area_sum.sqrt() as f32 as f64) * result.cfg.aspect);
        let (mut x, mut y, mut highest) = (0., 0., 0.0_f64);
        for mut g in components {
            let w = g.max.x - g.min.x;
            let h = g.max.y - g.min.y;
            if x + w > row_width {
                x = 0.;
                y += highest + result.cfg.spacing;
                highest = 0.;
            }
            let offset = V::new(x - g.min.x, y - g.min.y);
            self.offset(&mut g, offset);
            result.nodes.extend(g.nodes);
            for e in g.edges {
                if !result.edges.contains(&e) {
                    result.edges.push(e)
                }
            }
            highest = highest.max(h);
            x += w + result.cfg.spacing;
        }
        self.bounds(&mut result);
        let v = V::new(
            result.cfg.padding[1] - result.min.x,
            result.cfg.padding[0] - result.min.y,
        );
        self.offset(&mut result, v);
        result
    }
}
pub(crate) fn layout(wire: &mut ElkGraph, options: ElkOptions) -> Result<(), LayeringError> {
    if wire.children.is_empty() {
        wire.x = Some(wire.x.unwrap_or(0.) + 0.);
        wire.y = Some(wire.y.unwrap_or(0.) + 0.);
        wire.width = Some(wire.width.unwrap_or(0.));
        wire.height = Some(wire.height.unwrap_or(0.));
        return Ok(());
    }
    let consumed = crate::utility::layout_default_children(wire, options)?;
    if !flag(&wire.layout_options, "omitNodeMicroLayout", false) {
        crate::rectpacking::micro::apply(wire, &consumed)
    }
    let cfg = Config::new(&wire.layout_options);
    let (mut arena, g) = Arena::import(wire, cfg);
    let mut components = arena.split(&g);
    for component in &mut components {
        arena.treeify(component);
        let root = arena.root(component);
        arena.fan(root);
        arena.levels(root);
        arena.order(root, &component.cfg.weight);
        arena.level_heights(root, component.cfg.horizontal());
        arena.neighbors(root);
        arena.place(component, root);
        arena.direction(component);
        arena.level_coordinates(component);
        arena.compact(component);
        arena.bounds(component);
        arena.route(component);
    }
    let g = arena.pack(components);
    let (mut minx, mut miny, mut maxx, mut maxy) = (
        i32::MAX as f64,
        i32::MAX as f64,
        i32::MIN as f64,
        i32::MIN as f64,
    );
    for &n in &g.nodes {
        let n = &arena.nodes[n];
        minx = minx.min(n.pos.x - n.size.x / 2.);
        miny = miny.min(n.pos.y - n.size.y / 2.);
        maxx = maxx.max(n.pos.x + n.size.x / 2.);
        maxy = maxy.max(n.pos.y + n.size.y / 2.);
        if let Some(i) = n.origin {
            wire.children[i].x = n.pos.x;
            wire.children[i].y = n.pos.y;
        }
    }
    for &e in &g.edges {
        let e = &arena.edges[e];
        let Some(i) = e.origin else { continue };
        if e.bends.len() < 2 {
            return Err(LayeringError::Provider {
                algorithm: "org.eclipse.elk.mrtree",
                message:
                    "mrtree: the vector chain must contain at least a source and a target point"
                        .into(),
            });
        }
        let edge = &mut wire.edges[i];
        if edge.sections.is_empty() {
            edge.sections.push(ElkEdgeSection {
                id: Some(format!("{}_s0", edge.id)),
                start: e.bends[0].point(),
                end: e.bends[e.bends.len() - 1].point(),
                bends: vec![],
                incoming_shape: None,
                outgoing_shape: None,
            })
        }
        let s = &mut edge.sections[0];
        s.start = e.bends[0].point();
        s.end = e.bends[e.bends.len() - 1].point();
        s.bends = if g.cfg.route == "MIDDLE_TO_MIDDLE" {
            &e.bends[..]
        } else {
            &e.bends[1..e.bends.len() - 1]
        }
        .iter()
        .map(|v| v.point())
        .collect();
    }
    if !g.cfg.fixed {
        wire.width = Some(maxx - minx + g.cfg.padding[1] + g.cfg.padding[3]);
        wire.height = Some(maxy - miny + g.cfg.padding[0] + g.cfg.padding[2]);
    } else {
        wire.width = Some(wire.width.unwrap_or(0.));
        wire.height = Some(wire.height.unwrap_or(0.));
    }
    wire.x = Some(wire.x.unwrap_or(0.) + 0.);
    wire.y = Some(wire.y.unwrap_or(0.) + 0.);
    Ok(())
}

#[cfg(test)]
mod tests;
