// Copyright (c) 2017 Kiel University and others.
// Copyright (c) 2017, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// Pinned elk-go/internal/radial/provider.go and options.go.
mod arena;
mod compaction;
mod placement;
use crate::property::{enum_value, flag, integer, number};
use crate::utility::{option, padding, point};
use crate::{ElkEdgeSection, ElkGraph, ElkNode, ElkOptions, ElkPoint, LayeringError};
use arena::Arena;
use serde_json::Value;
use std::collections::BTreeMap;
use std::f64::consts::PI;

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
    fn scale(self, k: f64) -> Self {
        Self {
            x: self.x * k,
            y: self.y * k,
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
struct Config {
    radius: f64,
    spacing: f64,
    sorter: String,
    wedge: String,
    optimizer: String,
    compactor: String,
    step: i32,
    rotate: bool,
    target_angle: f64,
    additional_wedge: bool,
    outgoing_angles: bool,
    center: bool,
    fixed: bool,
    padding: [f64; 4],
}
impl Config {
    fn new(o: &Value) -> Self {
        let pad = if option(o, "padding").is_some() {
            let p = padding(o);
            [p[0], p[3], p[2], p[1]]
        } else {
            [12.0; 4]
        };
        Self {
            radius: number(o, "radial.radius", 0.0),
            spacing: number(o, "spacing.nodeNode", 20.0),
            sorter: enum_value(
                o,
                "radial.sorter",
                &["NONE", "POLAR_COORDINATE", "ID"],
                "NONE",
            ),
            wedge: enum_value(
                o,
                "radial.wedgeCriteria",
                &["LEAF_NUMBER", "NODE_SIZE"],
                "NODE_SIZE",
            ),
            optimizer: enum_value(
                o,
                "radial.optimizationCriteria",
                &[
                    "NONE",
                    "EDGE_LENGTH",
                    "EDGE_LENGTH_BY_POSITION",
                    "CROSSING_MINIMIZATION_BY_POSITION",
                ],
                "NONE",
            ),
            compactor: enum_value(
                o,
                "radial.compactor",
                &["NONE", "RADIAL_COMPACTION", "WEDGE_COMPACTION"],
                "NONE",
            ),
            step: integer(o, "radial.compactionStepSize").unwrap_or(1),
            rotate: flag(o, "radial.rotate", false),
            target_angle: number(o, "radial.rotation.targetAngle", 0.0),
            additional_wedge: flag(o, "radial.rotation.computeAdditionalWedgeSpace", false),
            outgoing_angles: flag(o, "radial.rotation.outgoingEdgeAngles", false),
            center: flag(o, "radial.centerOnRoot", false),
            fixed: flag(o, "nodeSize.fixedGraphSize", false),
            padding: pad,
        }
    }
}
pub(crate) fn layout(graph: &mut ElkGraph, options: ElkOptions) -> Result<(), LayeringError> {
    if graph.children.is_empty() {
        graph.x = Some(graph.x.unwrap_or(0.0) + 0.0);
        graph.y = Some(graph.y.unwrap_or(0.0) + 0.0);
        graph.width = Some(graph.width.unwrap_or(0.0));
        graph.height = Some(graph.height.unwrap_or(0.0));
        return Ok(());
    }
    let consumed = crate::utility::layout_default_children(graph, options)?;
    let micro = !flag(&graph.layout_options, "omitNodeMicroLayout", false);
    if micro {
        crate::rectpacking::micro::apply(graph, &consumed)
    }
    let cfg = Config::new(&graph.layout_options);
    let mut a = Arena::import(graph, micro);
    let root = a.nodes[0]
        .children
        .iter()
        .copied()
        .find(|&n| a.nodes[n].incoming.is_empty())
        .ok_or_else(|| LayeringError::Provider {
            algorithm: "org.eclipse.elk.radial",
            message: "the given graph is not a tree".into(),
        })?;
    let radius = if cfg.radius == 0.0 {
        a.largest(0)
    } else {
        cfg.radius
    };
    a.position_graph(root, radius, &cfg);
    a.remove_overlaps(root, &cfg);
    match cfg.compactor.as_str() {
        "RADIAL_COMPACTION" => a.radial_compact(root, &cfg),
        "WEDGE_COMPACTION" => a.wedge_compact(root, &cfg),
        _ => {}
    }
    if cfg.rotate {
        a.rotate(root, &cfg)
    }
    let (width, height) = a.graph_size(root, &cfg);
    a.route(graph, root);
    if cfg.outgoing_angles {
        for e in a.nodes[root].outgoing.clone() {
            let wire = &graph.edges[e];
            if let (Some(section), Some(&target)) =
                (wire.sections.first(), a.edges[e].targets.first())
            {
                let angle = crate::go_math::atan2(
                    section.end.y - section.start.y,
                    section.end.x - section.start.x,
                );
                a.nodes[target].angle = angle;
            }
        }
    }
    a.publish(&mut graph.children);
    if !cfg.fixed {
        graph.width = Some(width);
        graph.height = Some(height)
    }
    // Codec.transferShape adds the zero parent-coordinate offset even at
    // the root. That addition publishes +0 for an authored -0 coordinate.
    graph.x = Some(graph.x.unwrap_or(0.0) + 0.0);
    graph.y = Some(graph.y.unwrap_or(0.0) + 0.0);
    Ok(())
}
fn clip_vector(v: V, size: V) -> V {
    let x = if v.x.abs() > size.x / 2.0 {
        size.x / 2.0 / v.x.abs()
    } else {
        1.0
    };
    let y = if v.y.abs() > size.y / 2.0 {
        size.y / 2.0 / v.y.abs()
    } else {
        1.0
    };
    v.scale(x.min(y))
}
fn clipped(mut source: V, mut target: V, source_size: V, target_size: V) -> (V, V) {
    let mut vector = target.sub(source);
    let c = clip_vector(vector, source_size);
    vector = vector.sub(c);
    source = target.sub(vector);
    let c = clip_vector(vector, target_size);
    vector = vector.sub(c);
    target = source.add(vector);
    (source, target)
}

#[cfg(test)]
mod tests;

pub(crate) fn wire_node_margins(graph: &ElkGraph, node: &ElkNode, spacing: f64) -> [f64; 4] {
    arena::node_margins(graph, node, spacing)
}
