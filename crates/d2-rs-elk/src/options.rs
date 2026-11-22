// SPDX-License-Identifier: MPL-2.0
//! D2 d2elklayout/options.go: textual option validation and parent profiles.
use crate::{ElkAlgorithm, ElkGraph, ElkOptions, GraphPadding};
use d2_rs_core::{EdgeId, Graph, LayoutError, LayoutResult, Point};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Layered controls applied to parent graphs; `None` preserves provider defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayeredOptions {
    /// Within-layer node clearance.
    pub node_node: Option<f64>,
    /// Within-layer edge to node clearance.
    pub edge_node: Option<f64>,
    /// Within-layer edge clearance.
    pub edge_edge: Option<f64>,
    /// ORTHOGONAL or POLYLINE routing.
    pub edge_routing: Option<&'static str>,
    /// OFF, SINGLE_EDGE or MULTI_EDGE wrapping.
    pub wrapping_strategy: Option<&'static str>,
    /// Preserve node model order at the root.
    pub force_node_model_order: bool,
    /// Route feedback edges around the graph.
    pub feedback_edges: bool,
    /// Share unconstrained input/output ports.
    pub merge_edges: bool,
}
/// Controls for the algorithm selected at the diagram root.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AlgorithmOptions {
    /// FRUCHTERMAN_REINGOLD or EADES.
    pub force_model: Option<&'static str>,
    /// Nonnegative force repulsion.
    pub force_repulsion: Option<f64>,
    /// Positive desired stress edge length.
    pub stress_desired_edge_length: Option<f64>,
    /// DFS or BFS.
    pub mrtree_search_order: Option<&'static str>,
    /// MODEL_ORDER, DESCENDANTS or FAN.
    pub mrtree_weighting: Option<&'static str>,
    /// Nonnegative radial radius.
    pub radial_radius: Option<f64>,
    /// NODE_SIZE or LEAF_NUMBER.
    pub radial_wedge_criteria: Option<&'static str>,
    /// NONE, RADIAL_COMPACTION or WEDGE_COMPACTION.
    pub radial_compactor: Option<&'static str>,
}
fn error(name: &str, message: impl std::fmt::Display) -> LayoutError {
    LayoutError::EngineValidation(format!("{name}: {message}"))
}
fn number(name: &str, raw: &str) -> Result<f64, LayoutError> {
    let n = d2_rs_core::parse_d2_float_result(raw).map_err(|e| error(name, e))?;
    validate_number(name, n)?;
    Ok(n)
}
fn validate_number(name: &str, n: f64) -> Result<(), LayoutError> {
    if !n.is_finite() {
        return Err(error(name, "expected a finite number"));
    }
    if name == "elk-stressDesiredEdgeLength" && n <= 0. {
        return Err(error(name, "desired edge length must be positive"));
    }
    if n < 0. {
        return Err(error(name, "expected a nonnegative number"));
    }
    Ok(())
}
fn enumeration(
    name: &str,
    raw: &str,
    choices: &[&'static str],
) -> Result<&'static str, LayoutError> {
    choices
        .iter()
        .copied()
        .find(|v| v.eq_ignore_ascii_case(raw))
        .ok_or_else(|| {
            error(
                name,
                format!("expected {}", choices.join(" or ").to_lowercase()),
            )
        })
}
fn boolean(name: &str, raw: &str) -> Result<bool, LayoutError> {
    match raw.to_lowercase().as_str() {
        "1" | "t" | "true" => Ok(true),
        "0" | "f" | "false" => Ok(false),
        _ => Err(error(name, "expected a boolean")),
    }
}
pub(crate) fn algorithm_name(raw: &str) -> String {
    let mut id = raw.to_lowercase();
    let start = id.rfind('.').map_or(0, |p| p + 1);
    let suffix = match &id[start..] {
        "sporeoverlap" => "sporeOverlap",
        "sporecompaction" => "sporeCompaction",
        _ => return id,
    };
    id.replace_range(start.., suffix);
    id
}
impl ElkOptions {
    /// Sets a D2 ELK option by CLI name, without changing the options on error.
    pub fn set_option(&mut self, name: &str, value: &str) -> Result<(), LayoutError> {
        let mut o = *self;
        let l = &mut o.layered;
        let a = &mut o.algorithm_options;
        match name {
            "elk-algorithm" => {
                o.algorithm = if value.is_empty() {
                    ElkAlgorithm::Layered
                } else {
                    ElkAlgorithm::parse(&algorithm_name(value))?
                }
            }
            "elk-nodeNode" => l.node_node = Some(number(name, value)?),
            "elk-edgeNode" => l.edge_node = Some(number(name, value)?),
            "elk-edgeEdge" => l.edge_edge = Some(number(name, value)?),
            "elk-edgeRouting" => {
                l.edge_routing = Some(enumeration(name, value, &["ORTHOGONAL", "POLYLINE"])?)
            }
            "elk-wrappingStrategy" => {
                l.wrapping_strategy = Some(enumeration(
                    name,
                    value,
                    &["OFF", "SINGLE_EDGE", "MULTI_EDGE"],
                )?)
            }
            "elk-forceNodeModelOrder" => l.force_node_model_order = boolean(name, value)?,
            "elk-feedbackEdges" => l.feedback_edges = boolean(name, value)?,
            "elk-mergeEdges" => l.merge_edges = boolean(name, value)?,
            "elk-forceModel" => {
                a.force_model = Some(enumeration(
                    name,
                    value,
                    &["FRUCHTERMAN_REINGOLD", "EADES"],
                )?)
            }
            "elk-forceRepulsion" => a.force_repulsion = Some(number(name, value)?),
            "elk-stressDesiredEdgeLength" => {
                a.stress_desired_edge_length = Some(number(name, value)?)
            }
            "elk-mrtreeSearchOrder" => {
                a.mrtree_search_order = Some(enumeration(name, value, &["DFS", "BFS"])?)
            }
            "elk-mrtreeWeighting" => {
                a.mrtree_weighting = Some(enumeration(
                    name,
                    value,
                    &["MODEL_ORDER", "DESCENDANTS", "FAN"],
                )?)
            }
            "elk-radialRadius" => a.radial_radius = Some(number(name, value)?),
            "elk-radialWedgeCriteria" => {
                a.radial_wedge_criteria =
                    Some(enumeration(name, value, &["NODE_SIZE", "LEAF_NUMBER"])?)
            }
            "elk-radialCompactor" => {
                a.radial_compactor = Some(enumeration(
                    name,
                    value,
                    &["NONE", "RADIAL_COMPACTION", "WEDGE_COMPACTION"],
                )?)
            }
            "elk-padding" => o.padding_sides = Some(read_padding(value)?),
            "elk-nodeNodeBetweenLayers"
            | "elk-edgeNodeBetweenLayers"
            | "elk-edgeEdgeBetweenLayers"
            | "elk-nodeSelfLoop" => {
                let n = value
                    .parse::<i64>()
                    .map_err(|_| error(name, "expected a nonnegative integer"))?;
                if n < 0 {
                    return Err(error(name, "expected a nonnegative integer"));
                }
                match name {
                    "elk-nodeNodeBetweenLayers" => o.node_spacing = n as f64,
                    "elk-edgeNodeBetweenLayers" => o.edge_node_spacing = n as f64,
                    "elk-edgeEdgeBetweenLayers" => o.edge_edge_spacing = n as f64,
                    _ => o.self_loop_spacing = n as f64,
                }
            }
            _ => return Err(error(name, "unknown ELK option")),
        }
        *self = o;
        Ok(())
    }
}
pub(crate) fn read_padding(raw: &str) -> Result<GraphPadding, LayoutError> {
    let mut p = GraphPadding::default();
    for part in raw
        .trim_matches(|c: char| "()[]{}\"' \t\r\n".contains(c))
        .split([',', ';'])
        .filter(|s| !s.is_empty())
    {
        let (name, raw) = part
            .split_once('=')
            .ok_or_else(|| error("elk-padding", "expected side=nonnegative-number"))?;
        let n = number("elk-padding", raw.trim())?;
        match name.trim().to_lowercase().as_str() {
            "top" => p.top = n,
            "right" => p.right = n,
            "bottom" => p.bottom = n,
            "left" => p.left = n,
            _ => return Err(error("elk-padding", format!("unknown side {name:?}"))),
        }
    }
    Ok(p)
}
const NAMES: &[&str] = &[
    "elk-algorithm",
    "elk-forceModel",
    "elk-forceRepulsion",
    "elk-stressDesiredEdgeLength",
    "elk-mrtreeSearchOrder",
    "elk-mrtreeWeighting",
    "elk-radialRadius",
    "elk-radialWedgeCriteria",
    "elk-radialCompactor",
    "elk-nodeNode",
    "elk-edgeNode",
    "elk-edgeEdge",
    "elk-edgeRouting",
    "elk-wrappingStrategy",
    "elk-forceNodeModelOrder",
    "elk-feedbackEdges",
    "elk-mergeEdges",
    "elk-nodeNodeBetweenLayers",
    "elk-edgeNodeBetweenLayers",
    "elk-edgeEdgeBetweenLayers",
    "elk-nodeSelfLoop",
    "elk-padding",
];
pub(crate) fn graph_options(graph: &Graph, mut o: ElkOptions) -> Result<ElkOptions, LayoutError> {
    if let Some(p) = &graph.layout_engine_options.elk_padding {
        o.set_option("elk-padding", p)?;
    }
    for &name in NAMES {
        if let Some(v) = graph.d2_config_data.get(name) {
            let raw = match v {
                Value::String(s) => s.clone(),
                Value::Bool(_) | Value::Number(_) => v.to_string(),
                _ => return Err(error(name, "expects a scalar value")),
            };
            // Keep elk-go's empty-graph algorithm resolution behavior.
            if name == "elk-algorithm" && graph.nodes().next().is_none() {
                continue;
            }
            o.set_option(name, &raw)?;
        }
    }
    for (name, n) in [
        ("elk-nodeNode", o.layered.node_node),
        ("elk-edgeNode", o.layered.edge_node),
        ("elk-edgeEdge", o.layered.edge_edge),
        ("elk-forceRepulsion", o.algorithm_options.force_repulsion),
        (
            "elk-stressDesiredEdgeLength",
            o.algorithm_options.stress_desired_edge_length,
        ),
        ("elk-radialRadius", o.algorithm_options.radial_radius),
    ] {
        if let Some(n) = n {
            validate_number(name, n)?;
        }
    }
    // Typed callers receive the same enum validation and normalization.
    for (name, raw) in [
        ("elk-edgeRouting", o.layered.edge_routing),
        ("elk-wrappingStrategy", o.layered.wrapping_strategy),
        ("elk-forceModel", o.algorithm_options.force_model),
        (
            "elk-mrtreeSearchOrder",
            o.algorithm_options.mrtree_search_order,
        ),
        ("elk-mrtreeWeighting", o.algorithm_options.mrtree_weighting),
        (
            "elk-radialWedgeCriteria",
            o.algorithm_options.radial_wedge_criteria,
        ),
        ("elk-radialCompactor", o.algorithm_options.radial_compactor),
    ] {
        if let Some(raw) = raw {
            o.set_option(name, raw)?;
        }
    }
    Ok(o)
}
pub(crate) fn parent_options(o: &ElkOptions, root: bool, value: &mut Value) {
    let l = o.layered;
    let m = value.as_object_mut().unwrap();
    m.remove("elk.spacing.edgeNode");
    for (key, n) in [
        ("elk.spacing.nodeNode", l.node_node),
        (
            "elk.spacing.edgeNode",
            l.edge_node
                .or_else(|| (!root || o.algorithm == ElkAlgorithm::Layered).then_some(40.)),
        ),
        ("elk.spacing.edgeEdge", l.edge_edge),
    ] {
        if let Some(n) = n {
            m.insert(key.into(), json!(n));
        }
    }
    for (key, s) in [
        ("elk.edgeRouting", l.edge_routing),
        ("elk.layered.wrapping.strategy", l.wrapping_strategy),
    ] {
        if let Some(s) = s {
            m.insert(key.into(), json!(s));
        }
    }
    for (key, b) in [
        (
            "elk.layered.crossingMinimization.forceNodeModelOrder",
            root && l.force_node_model_order,
        ),
        ("elk.layered.feedbackEdges", l.feedback_edges),
        ("elk.layered.mergeEdges", l.merge_edges),
    ] {
        if b {
            m.insert(key.into(), json!(b));
        }
    }
    if root {
        let a = o.algorithm_options;
        for (key, s) in [
            ("elk.force.model", a.force_model),
            ("elk.mrtree.searchOrder", a.mrtree_search_order),
            ("elk.mrtree.weighting", a.mrtree_weighting),
            ("elk.radial.wedgeCriteria", a.radial_wedge_criteria),
            ("elk.radial.compactor", a.radial_compactor),
        ] {
            if let Some(s) = s {
                m.insert(key.into(), json!(s));
            }
        }
        for (key, n) in [
            ("elk.force.repulsion", a.force_repulsion),
            ("elk.stress.desiredEdgeLength", a.stress_desired_edge_length),
            ("elk.radial.radius", a.radial_radius),
        ] {
            if let Some(n) = n {
                m.insert(key.into(), json!(n));
            }
        }
    }
}
pub(crate) fn mr_tree_routes(graph: &Graph, elk: &ElkGraph, o: ElkOptions) -> BTreeSet<EdgeId> {
    if o.algorithm != ElkAlgorithm::MrTree {
        return BTreeSet::new();
    }
    graph
        .edges()
        .filter_map(|(id, _)| {
            elk.edges
                .get(id.0 as usize)
                .is_some_and(|e| e.container.is_empty())
                .then_some(id)
        })
        .collect()
}
pub(crate) fn trim_mr_tree_routes(
    graph: &Graph,
    result: &mut LayoutResult,
    keep: &BTreeSet<EdgeId>,
) {
    for &id in keep {
        let e = &graph.edges[id.0 as usize];
        if e.source == e.target
            || graph.node(e.source).unwrap().parent != graph.node(e.target).unwrap().parent
        {
            continue;
        }
        let Some(route) = result.routes.get_mut(&id).filter(|p| p.len() > 2) else {
            continue;
        };
        let contains = |node, p: Point| {
            let b = result.boxes[&node];
            p.x >= b.origin.x
                && p.x <= b.origin.x + b.size.width
                && p.y >= b.origin.y
                && p.y <= b.origin.y + b.size.height
        };
        let mut first = 1;
        let mut end = route.len() - 1;
        for (i, &p) in route.iter().enumerate().take(end).skip(1) {
            if contains(e.target, p) {
                end = i;
                break;
            }
            if contains(e.source, p) {
                first = i + 1;
            }
        }
        let mut bends = route[first..end].to_vec();
        bends.dedup();
        let last = *route.last().unwrap();
        route.truncate(1);
        route.extend(bends);
        route.push(last);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_rs_core::{Direction, Edge, LayoutOptions, Node, Size};
    fn graph() -> Graph {
        let mut g = Graph::default();
        for name in ["Root", "API", "Cache", "Store", "Web"] {
            g.add_node(Node::new(
                name,
                Size {
                    width: 60.,
                    height: 40.,
                },
            ));
        }
        for (s, t) in [(0, 1), (1, 2), (2, 3), (0, 3), (0, 4)] {
            g.add_edge(Edge {
                source: d2_rs_core::NodeId(s),
                target: d2_rs_core::NodeId(t),
            });
        }
        g
    }
    // Upstream options_test.go TestAlgorithmOptionValidation and
    // TestELKPaddingWithOmittedSides; all settings are D2-input reachable.
    #[test]
    fn validates_options_atomically() {
        for (key, value) in [
            ("elk-stressDesiredEdgeLength", "0"),
            ("elk-stressDesiredEdgeLength", "-1"),
            ("elk-forceRepulsion", "NaN"),
            ("elk-radialRadius", "+Inf"),
            ("elk-radialRadius", "-1"),
            ("elk-forceModel", "spring"),
            ("elk-mrtreeSearchOrder", "shortest"),
            ("elk-mrtreeWeighting", "constraint"),
            ("elk-radialWedgeCriteria", "area"),
            ("elk-radialCompactor", "tight"),
            ("elk-nodeNode", "-1"),
            ("elk-edgeEdge", "inf"),
            ("elk-edgeRouting", "splines"),
            ("elk-wrappingStrategy", "fold"),
            ("elk-feedbackEdges", "yes"),
            ("elk-nodeSelfLoop", "1.5"),
            ("elk-edgeNodeBetweenLayers", "-1"),
            ("elk-padding", "[top=-1]"),
            ("elk-padding", "[side=3]"),
        ] {
            let mut o = ElkOptions::default();
            let before = o;
            assert!(
                o.set_option(key, value)
                    .unwrap_err()
                    .to_string()
                    .contains(key),
                "{key}"
            );
            assert_eq!(o, before);
        }
        let mut o = ElkOptions::default();
        o.algorithm_options.stress_desired_edge_length = Some(0.);
        assert!(
            graph_options(&graph(), o)
                .unwrap_err()
                .to_string()
                .contains("elk-stressDesiredEdgeLength")
        );
        for input in [
            "[top=12.5,left=9]",
            "(top=12.5; left=9)",
            "top=12.5,left=9",
            "[ToP=12.5,LeFt=9]",
        ] {
            let mut o = ElkOptions::default();
            o.set_option("elk-padding", input).unwrap();
            assert_eq!(
                o.padding_sides,
                Some(GraphPadding {
                    top: 12.5,
                    left: 9.,
                    bottom: 0.,
                    right: 0.
                })
            );
        }
    }
    #[test]
    fn preserves_parent_profiles_and_algorithm_defaults() {
        let mut o = ElkOptions::default();
        for (k, v) in [
            ("elk-forceModel", "eAdEs"),
            ("elk-forceRepulsion", "0"),
            ("elk-stressDesiredEdgeLength", "200"),
            ("elk-mrtreeSearchOrder", "bFs"),
            ("elk-mrtreeWeighting", "fan"),
            ("elk-radialRadius", "0"),
            ("elk-radialWedgeCriteria", "leaf_number"),
            ("elk-radialCompactor", "wedge_compaction"),
            ("elk-forceNodeModelOrder", "TrUe"),
            ("elk-edgeNode", "0"),
            ("elk-edgeEdge", "0"),
        ] {
            o.set_option(k, v).unwrap();
        }
        let mut root = json!({});
        parent_options(&o, true, &mut root);
        assert_eq!(root["elk.force.model"], "EADES");
        assert_eq!(root["elk.force.repulsion"], 0.);
        assert_eq!(root["elk.stress.desiredEdgeLength"], 200.);
        assert_eq!(root["elk.mrtree.searchOrder"], "BFS");
        assert_eq!(root["elk.mrtree.weighting"], "FAN");
        assert_eq!(root["elk.radial.radius"], 0.);
        assert_eq!(root["elk.radial.wedgeCriteria"], "LEAF_NUMBER");
        assert_eq!(root["elk.radial.compactor"], "WEDGE_COMPACTION");
        assert_eq!(root["elk.spacing.edgeNode"], 0.);
        assert_eq!(root["elk.spacing.edgeEdge"], 0.);
        assert_eq!(
            root["elk.layered.crossingMinimization.forceNodeModelOrder"],
            true
        );
        let mut child = json!({});
        parent_options(&o, false, &mut child);
        assert!(child.get("elk.force.model").is_none());
        assert!(
            child
                .get("elk.layered.crossingMinimization.forceNodeModelOrder")
                .is_none()
        );
        for spelling in ["mrtree", "ElK.MrTrEe", "ORG.ECLIPSE.ELK.MRTREE"] {
            let mut o = ElkOptions::default();
            o.set_option("elk-algorithm", spelling).unwrap();
            let mut root = json!({});
            let mut child = json!({});
            parent_options(&o, true, &mut root);
            parent_options(&o, false, &mut child);
            assert!(root.get("elk.spacing.edgeNode").is_none());
            assert_eq!(child["elk.spacing.edgeNode"], 40.);
        }
        assert_eq!(
            algorithm_name("ORG.ECLIPSE.ELK.SPORECOMPACTION"),
            "org.eclipse.elk.sporeCompaction"
        );
    }
    #[test]
    fn graph_data_overrides_defaults_and_rejects_nonscalars() {
        let mut g = graph();
        g.d2_config_data
            .insert("elk-mrtreeSearchOrder".into(), json!("bFs"));
        g.d2_config_data
            .insert("elk-forceRepulsion".into(), json!(0));
        g.d2_config_data
            .insert("elk-mergeEdges".into(), json!(true));
        let o = graph_options(&g, ElkOptions::default()).unwrap();
        assert_eq!(o.algorithm_options.mrtree_search_order, Some("BFS"));
        assert_eq!(o.algorithm_options.force_repulsion, Some(0.));
        assert!(o.layered.merge_edges);
        g.d2_config_data.insert("elk-edgeNode".into(), json!([2]));
        assert!(
            graph_options(&g, ElkOptions::default())
                .unwrap_err()
                .to_string()
                .contains("scalar")
        );
    }
    // f3da1baea endpoint contract, in every direction and search order.
    // Full all-shapes collision coverage remains open: the existing provider
    // skips some multilevel gaps (see mrtree::routing::Arena::skip_gap).
    #[test]
    fn mr_tree_attachments_avoid_endpoint_shapes() {
        for direction in [
            Direction::Right,
            Direction::Left,
            Direction::Down,
            Direction::Up,
        ] {
            for search in ["dfs", "bfs"] {
                let mut g = graph();
                g.direction = direction;
                g.d2_config_data
                    .insert("elk-algorithm".into(), json!("MrTrEe"));
                g.d2_config_data
                    .insert("elk-mrtreeSearchOrder".into(), json!(search));
                let result = crate::layout(&g, &LayoutOptions::default()).unwrap();
                for (id, e) in g.edges() {
                    let route = &result.routes[&id];
                    assert!(route.len() >= 2);
                    let contains = |n, p: Point| {
                        let r = result.boxes[&n];
                        p.x >= r.origin.x
                            && p.x <= r.origin.x + r.size.width
                            && p.y >= r.origin.y
                            && p.y <= r.origin.y + r.size.height
                    };
                    assert!(
                        !contains(e.source, route[1]),
                        "{direction:?}/{search}/{id:?} {route:?}"
                    );
                    assert!(
                        !contains(e.target, route[route.len() - 2]),
                        "{direction:?}/{search}/{id:?} {route:?}"
                    );
                    for p in route.windows(2) {
                        assert_ne!(p[0], p[1]);
                    }
                }
            }
        }
    }
    #[test]
    fn merged_edge_attachments_survive_d2_bend_cleanup() {
        let mut g = graph();
        g.d2_config_data
            .insert("elk-mergeEdges".into(), json!(true));
        let result = crate::layout(&g, &LayoutOptions::default()).unwrap();
        assert_eq!(result.routes[&EdgeId(0)][0], result.routes[&EdgeId(3)][0]);
        assert_eq!(result.routes[&EdgeId(0)][0], result.routes[&EdgeId(4)][0]);
        assert_eq!(
            result.routes[&EdgeId(2)].last(),
            result.routes[&EdgeId(3)].last()
        );
    }

    #[test]
    fn algorithm_controls_change_geometry() {
        let mut g = graph();
        g.d2_config_data
            .insert("elk-algorithm".into(), json!("stress"));
        g.d2_config_data
            .insert("elk-stressDesiredEdgeLength".into(), json!(80));
        let compact = crate::layout(&g, &LayoutOptions::default()).unwrap();
        g.d2_config_data
            .insert("elk-stressDesiredEdgeLength".into(), json!(240));
        let spacious = crate::layout(&g, &LayoutOptions::default()).unwrap();
        let length = |r: &LayoutResult| {
            let a = r.boxes[&d2_rs_core::NodeId(0)].center();
            let b = r.boxes[&d2_rs_core::NodeId(1)].center();
            (a.x - b.x).hypot(a.y - b.y)
        };
        assert!(length(&spacious) > length(&compact) * 2.);
    }
}
