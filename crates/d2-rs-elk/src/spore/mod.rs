// Copyright (c) 2017 Kiel University and others; 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// Pinned elk-go/internal/spore/{provider,importer,model,options,register}.go.
mod execution;
mod geometry;
mod random;
mod structure;
use crate::property::{enum_value, flag, integer, number};
use crate::utility::{minimum, option, padding};
use crate::{ElkAlgorithm, ElkEdgeSection, ElkGraph, ElkOptions, ElkPoint, LayeringError};
use geometry::*;
use serde_json::Value;
use std::collections::BTreeMap;
type Edge = (usize, usize);
struct Tree {
    root: usize,
    children: Vec<Vec<usize>>,
}
struct Node {
    original: V,
    vertex: V,
    rect: Rect,
}
impl Node {
    fn translate(&mut self, by: V) {
        self.vertex = self.vertex.add(by);
        self.rect = self.rect.moved(by)
    }
}
struct Config {
    overlap: bool,
    cost: String,
    maximum: bool,
    orthogonal: bool,
    root: usize,
    iterations: i32,
    scanline: bool,
}
fn cost(nodes: &[Node], e: Edge, c: &Config) -> f64 {
    let (a, b) = (&nodes[e.0], &nodes[e.1]);
    let value = match c.cost.as_str() {
        "CENTER_DISTANCE" => a.original.distance(b.original),
        "CIRCLE_UNDERLAP" => {
            a.original.distance(b.original)
                - a.original.distance(a.rect.p)
                - b.original.distance(b.rect.p)
        }
        "RECTANGLE_UNDERLAP" => underlap(a.rect, b.rect),
        "INVERTED_OVERLAP" => {
            let d = shortest(a.rect, b.rect);
            if d >= 0. {
                d
            } else {
                -(overlap(a.rect, b.rect) - 1.) * b.rect.center().sub(a.rect.center()).go_length()
            }
        }
        "MINIMUM_ROOT_DISTANCE" => a
            .original
            .distance(nodes[c.root].vertex)
            .min(b.original.distance(nodes[c.root].vertex)),
        _ => unreachable!("validated cost"),
    };
    if c.maximum { -value } else { value }
}
fn underlying(w: &mut ElkGraph, o: ElkOptions) -> Result<Option<ElkAlgorithm>, LayeringError> {
    let Some(name) = option(&w.layout_options, "underlyingLayoutAlgorithm").and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let Ok(alg) = ElkAlgorithm::parse(name) else {
        return Ok(None);
    };
    match alg {
        ElkAlgorithm::Layered => crate::layout_elk_graph_with_layered_pipeline(
            w,
            ElkOptions {
                algorithm: ElkAlgorithm::Layered,
                node_spacing: 20.,
                edge_node_spacing: 10.,
                edge_edge_spacing: 10.,
                padding: 12.,
                self_loop_spacing: 10.,
            },
        ),
        ElkAlgorithm::Fixed => crate::fixed_layout::layout(w, o),
        ElkAlgorithm::Box => crate::box_layout::layout(w, o),
        ElkAlgorithm::Rectpacking => crate::rectpacking::layout(w, o),
        ElkAlgorithm::Random => crate::layout_elk_graph_with_random_pipeline(w, o),
        ElkAlgorithm::Force => crate::layout_elk_graph_with_force_pipeline(w, o),
        ElkAlgorithm::Stress => crate::layout_elk_graph_with_stress_pipeline(w, o),
        ElkAlgorithm::Radial => crate::layout_elk_graph_with_radial_pipeline(w, o),
        ElkAlgorithm::MrTree => crate::layout_elk_graph_with_mrtree_pipeline(w, o),
        ElkAlgorithm::SporeOverlap => layout(w, o, true),
        ElkAlgorithm::SporeCompaction => layout(w, o, false),
    }?;
    Ok(Some(alg))
}
pub(crate) fn layout(
    w: &mut ElkGraph,
    o: ElkOptions,
    overlap_mode: bool,
) -> Result<(), LayeringError> {
    if w.children.is_empty() {
        w.x = Some(w.x.unwrap_or(0.) + 0.);
        w.y = Some(w.y.unwrap_or(0.) + 0.);
        w.width = Some(w.width.unwrap_or(0.));
        w.height = Some(w.height.unwrap_or(0.));
        return Ok(());
    }
    let delegated = underlying(w, o).map_err(|e| match e {
        LayeringError::Provider { message, .. } => LayeringError::Provider {
            algorithm: if overlap_mode {
                "org.eclipse.elk.sporeOverlap"
            } else {
                "org.eclipse.elk.sporeCompaction"
            },
            message,
        },
        other => other,
    })?;
    if delegated.is_none() {
        crate::utility::layout_default_children(w, o)?;
    }
    let options = &w.layout_options;
    let spacing = number(options, "spacing.nodeNode", 8.);
    let label_spacing = number(options, "spacing.labelNode", 5.);
    let pad = if option(options, "padding").is_some() {
        padding(options)
    } else {
        // Only cloneable defaults materialize on graph.PropertyMap.Get.
        // Scalar node spacing remains SPoRE's own default after delegation.
        let cached = match delegated {
            Some(ElkAlgorithm::Fixed) if flag(options, "nodeSize.fixedGraphSize", false) => 8.,
            Some(
                ElkAlgorithm::Fixed
                | ElkAlgorithm::Box
                | ElkAlgorithm::Rectpacking
                | ElkAlgorithm::Random,
            ) => 15.,
            Some(ElkAlgorithm::Force | ElkAlgorithm::Stress) => 50.,
            Some(ElkAlgorithm::MrTree) => 20.,
            Some(ElkAlgorithm::Radial) => 12.,
            _ => 8.,
        };
        [cached; 4]
    };
    let mut nodes: Vec<Node> = vec![];
    // Source runtime ChaCha8 / math/rand.Float64, with unseeded per-thread
    // state. elk.randomSeed does not control coincident-center perturbations.
    for wire in &w.children {
        let half = V::new(wire.width / 2., wire.height / 2.);
        let mut p = V::new(wire.x + half.x, wire.y + half.y);
        while nodes.iter().any(|n| n.original == p) {
            p.x = (random::float64() - 0.5).mul_add(0.001, p.x);
            p.y = (random::float64() - 0.5).mul_add(0.001, p.y);
        }
        let m = crate::radial_layout::wire_node_margins(w, wire, label_spacing);
        let rect = Rect {
            p: V::new(
                p.x - half.x - spacing / 2. - m[3],
                p.y - half.y - spacing / 2. - m[0],
            ),
            size: V::new(
                wire.width + spacing + (m[3] + m[1]),
                wire.height + spacing + (m[0] + m[2]),
            ),
        };
        #[cfg(feature = "diagnostic-traces")]
        if trace_active() {
            eprintln!(
                "{}",
                serde_json::json!({"trace":"spore-import","node":nodes.len()+1,"shape":{"X":wire.x,"Y":wire.y,"Width":wire.width,"Height":wire.height},"root_width":w.width.unwrap_or(0.),"root_height":w.height.unwrap_or(0.),"original":{"X":p.x,"Y":p.y},"rect":{"X":rect.p.x,"Y":rect.p.y,"Width":rect.size.x,"Height":rect.size.y}})
            );
        }
        nodes.push(Node {
            original: p,
            vertex: p,
            rect,
        });
    }
    let selection = if overlap_mode {
        "CENTER_NODE".into()
    } else {
        enum_value(
            options,
            "processingOrder.rootSelection",
            &["FIXED", "CENTER_NODE"],
            "CENTER_NODE",
        )
    };
    let root = if selection == "FIXED" {
        if let Some(name) = option(options, "processingOrder.preferredRoot").and_then(Value::as_str)
        {
            w.children.iter().rposition(|n| n.id == name).unwrap_or(0)
        } else {
            0
        }
    } else {
        let center = V::new(
            w.x.unwrap_or(0.) + w.width.unwrap_or(0.) / 2.,
            w.y.unwrap_or(0.) + w.height.unwrap_or(0.) / 2.,
        );
        let mut root = 0;
        let mut closest = f64::INFINITY;
        for (i, n) in nodes.iter().enumerate() {
            let d = n.original.distance(center);
            if d < closest {
                root = i;
                closest = d
            }
        }
        root
    };
    let cfg = Config {
        overlap: overlap_mode,
        cost: if overlap_mode {
            "INVERTED_OVERLAP".into()
        } else {
            enum_value(
                options,
                "processingOrder.spanningTreeCostFunction",
                &[
                    "CENTER_DISTANCE",
                    "CIRCLE_UNDERLAP",
                    "RECTANGLE_UNDERLAP",
                    "INVERTED_OVERLAP",
                    "MINIMUM_ROOT_DISTANCE",
                ],
                "CIRCLE_UNDERLAP",
            )
        },
        maximum: !overlap_mode
            && enum_value(
                options,
                "processingOrder.treeConstruction",
                &["MINIMUM_SPANNING_TREE", "MAXIMUM_SPANNING_TREE"],
                "MINIMUM_SPANNING_TREE",
            ) == "MAXIMUM_SPANNING_TREE",
        orthogonal: flag(options, "compaction.orthogonal", false),
        root,
        iterations: integer(options, "overlapRemoval.maxIterations").unwrap_or(64),
        scanline: flag(options, "overlapRemoval.runScanline", true),
    };
    if cfg.overlap {
        let mut existed = true;
        for _ in 0..cfg.iterations.max(0) {
            if !existed {
                break;
            }
            let mut edges = if cfg.scanline {
                let edges = structure::scanline(&nodes);
                if edges.is_empty() {
                    break;
                }
                edges
            } else {
                vec![]
            };
            for e in structure::triangulate(&nodes.iter().map(|n| n.original).collect::<Vec<_>>()) {
                structure::add_edge(&mut edges, e)
            }
            let t = structure::tree(&nodes, &edges, cfg.root, |e| cost(&nodes, e, &cfg));
            existed = execution::grow(&mut nodes, &t);
            for n in &mut nodes {
                n.original = n.rect.center();
            }
        }
    } else if nodes.len() > 1 {
        let edges = structure::triangulate(&nodes.iter().map(|n| n.original).collect::<Vec<_>>());
        let t = structure::tree(&nodes, &edges, cfg.root, |e| cost(&nodes, e, &cfg));
        execution::compact(&mut nodes, &t, cfg.orthogonal);
    }
    let (mut min, mut max) = (
        V::new(f64::INFINITY, f64::INFINITY),
        V::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for (n, wire) in nodes.iter().zip(&mut w.children) {
        wire.x = n.rect.p.x;
        wire.y = n.rect.p.y;
        min.x = min.x.min(wire.x);
        min.y = min.y.min(wire.y);
        max.x = max.x.max(wire.x + wire.width);
        max.y = max.y.max(wire.y + wire.height);
    }
    // ResizeNode clears the root's constraints in these source providers.
    // Mr.Tree/Radial use SetDimensions and retain that property instead.
    let resized = match delegated {
        Some(
            ElkAlgorithm::Fixed
            | ElkAlgorithm::Rectpacking
            | ElkAlgorithm::Force
            | ElkAlgorithm::Stress,
        ) => !flag(&w.layout_options, "nodeSize.fixedGraphSize", false),
        Some(ElkAlgorithm::Box | ElkAlgorithm::Layered | ElkAlgorithm::Random) => true,
        _ => false,
    };
    let (minw, minh) = if resized {
        (0., 0.)
    } else {
        minimum(&w.layout_options)
    };
    w.width = Some(minw.max(max.x - min.x + (pad[1] + pad[3])));
    w.height = Some(minh.max(max.y - min.y + (pad[0] + pad[2])));
    let offset = V::new(-min.x + pad[1], -min.y + pad[0]);
    for wire in &mut w.children {
        wire.x += offset.x;
        wire.y += offset.y;
    }
    let contained = crate::edge_container_ids(w, &w.edges)
        .into_iter()
        .map(|id| id == w.id)
        .collect::<Vec<_>>();
    for (e, is_contained) in w.edges.iter_mut().zip(&contained) {
        if !is_contained {
            continue;
        }
        for s in &mut e.sections {
            s.start.x += offset.x;
            s.start.y += offset.y;
            s.end.x += offset.x;
            s.end.y += offset.y;
            for p in &mut s.bends {
                p.x += offset.x;
                p.y += offset.y;
            }
        }
        for l in &mut e.labels {
            l.x += offset.x;
            l.y += offset.y;
        }
        for p in &mut e.junction_points {
            p.x += offset.x;
            p.y += offset.y;
        }
    }
    let mut shapes = BTreeMap::new();
    fn add(shapes: &mut BTreeMap<String, (V, V)>, n: &crate::ElkNode) {
        let shape = (V::new(n.x, n.y), V::new(n.width, n.height));
        shapes.insert(n.id.clone(), shape);
        for p in &n.ports {
            shapes.insert(p.id.clone(), shape);
        }
        for n in &n.children {
            add(shapes, n)
        }
    }
    for n in &w.children {
        add(&mut shapes, n)
    }
    shapes.insert(
        w.id.clone(),
        (
            V::new(w.x.unwrap_or(0.), w.y.unwrap_or(0.)),
            V::new(w.width.unwrap_or(0.), w.height.unwrap_or(0.)),
        ),
    );
    for (e, is_contained) in w.edges.iter_mut().zip(&contained) {
        if !is_contained {
            continue;
        }
        let Some((s, t)) = e
            .sources
            .first()
            .and_then(|s| shapes.get(s))
            .zip(e.targets.first().and_then(|s| shapes.get(s)))
        else {
            continue;
        };
        let (mut start, mut end) = (s.0.add(s.1.scale(0.5)), t.0.add(t.1.scale(0.5)));
        start = start.add(clip(end.sub(start), s.1));
        end = end.add(clip(start.sub(end), t.1));
        let point = |v: V| ElkPoint { x: v.x, y: v.y };
        if e.sections.is_empty() {
            e.sections.push(ElkEdgeSection {
                id: Some(format!("{}_s0", e.id)),
                start: point(start),
                end: point(end),
                bends: vec![],
                incoming_shape: None,
                outgoing_shape: None,
            });
        } else {
            e.sections[0].start = point(start);
            e.sections[0].end = point(end);
        }
    }
    w.x = Some(w.x.unwrap_or(0.) + 0.);
    w.y = Some(w.y.unwrap_or(0.) + 0.);
    Ok(())
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests;

#[cfg(feature = "diagnostic-traces")]
thread_local! {static TRACE_ACTIVE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(feature = "diagnostic-traces")]
fn trace_active() -> bool {
    TRACE_ACTIVE.with(|v| v.get())
}
