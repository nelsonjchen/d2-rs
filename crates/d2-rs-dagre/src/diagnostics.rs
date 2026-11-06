// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Diagnostic-only adapter route snapshots. This module and its allocations
//! are absent unless the explicit diagnostic-traces feature is enabled.

use d2_rs_core::{EdgeId, Graph, Point};
use std::collections::{BTreeMap, BTreeSet};

#[allow(clippy::too_many_arguments)]
pub(super) fn rank_operation(
    phase: &str,
    iteration: isize,
    nodes: &[d2_rs_core::NodeId],
    constraints: &[super::RankConstraint],
    ranks: &BTreeMap<d2_rs_core::NodeId, i32>,
    tree_order: &[(d2_rs_core::NodeId, d2_rs_core::NodeId)],
    cuts: &BTreeMap<(d2_rs_core::NodeId, d2_rs_core::NodeId), i32>,
    parents: &BTreeMap<d2_rs_core::NodeId, d2_rs_core::NodeId>,
    low: &BTreeMap<d2_rs_core::NodeId, usize>,
    lim: &BTreeMap<d2_rs_core::NodeId, usize>,
    aliases: &BTreeMap<d2_rs_core::NodeId, String>,
    leave_index: Option<usize>,
    enter_index: Option<usize>,
) {
    if std::env::var_os("DAGRE_TRACE_RANK_OPS").is_none() {
        return;
    }
    let alias = |id: d2_rs_core::NodeId| {
        aliases
            .get(&id)
            .expect("rank trace has a stable identity for every node")
            .clone()
    };
    let bits = |value: f64| format!("{:016x}", value.to_bits());
    let nodes = nodes
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": alias(*id),
                "rank_bits": ranks.get(id).map(|rank| bits(f64::from(*rank))),
                "parent": parents.get(id).map(|parent| alias(*parent)),
                "low_bits": low.get(id).map(|value| bits(*value as f64)),
                "lim_bits": lim.get(id).map(|value| bits(*value as f64)),
            })
        })
        .collect::<Vec<_>>();
    let constraint_values = constraints
        .iter()
        .map(|edge| {
            serde_json::json!({
                "source": alias(edge.source),
                "target": alias(edge.target),
                "minlen_bits": bits(f64::from(edge.minlen)),
                "weight_bits": bits(f64::from(edge.weight)),
                "cut_bits": serde_json::Value::Null,
            })
        })
        .collect::<Vec<_>>();
    let tree = tree_order
        .iter()
        .map(|(source_id, target_id)| {
            let cut_bits = cuts
                .get(&super::rank_tree_key(*source_id, *target_id))
                .map(|cut| bits(f64::from(*cut)));
            let mut source = alias(*source_id);
            let mut target = alias(*target_id);
            if target < source {
                std::mem::swap(&mut source, &mut target);
            }
            serde_json::json!({
                "source": source,
                "target": target,
                "minlen_bits": "",
                "weight_bits": "",
                "cut_bits": cut_bits,
            })
        })
        .collect::<Vec<_>>();
    let leave = leave_index.map(|index| {
        let (source, target) = tree_order[index];
        let mut source = alias(source);
        let mut target = alias(target);
        if target < source {
            std::mem::swap(&mut source, &mut target);
        }
        serde_json::json!({"source": source, "target": target})
    });
    let enter = enter_index.map(|index| {
        let edge = constraints[index];
        serde_json::json!({"source": alias(edge.source), "target": alias(edge.target)})
    });
    let candidates = if let Some(index) = leave_index {
        let (tree_source, tree_target) = tree_order[index];
        let (source, target) = if constraints
            .iter()
            .any(|edge| edge.source == tree_source && edge.target == tree_target)
        {
            (tree_source, tree_target)
        } else {
            (tree_target, tree_source)
        };
        let flip = lim.get(&source).copied().unwrap_or_default()
            > lim.get(&target).copied().unwrap_or_default();
        let tail = if flip { target } else { source };
        let is_descendant = |node: d2_rs_core::NodeId| {
            let node_lim = lim.get(&node).copied().unwrap_or_default();
            let ancestor_low = low.get(&tail).copied().unwrap_or_default();
            let ancestor_lim = lim.get(&tail).copied().unwrap_or_default();
            ancestor_low <= node_lim && node_lim <= ancestor_lim
        };
        constraints
            .iter()
            .map(|edge| {
                let source_descendant = is_descendant(edge.source);
                let target_descendant = is_descendant(edge.target);
                let eligible = flip == source_descendant && flip != target_descendant;
                let slack = ranks[&edge.target] - ranks[&edge.source] - edge.minlen;
                serde_json::json!({
                    "source": alias(edge.source),
                    "target": alias(edge.target),
                    "eligible": eligible,
                    "slack_bits": bits(f64::from(slack)),
                })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let payload = serde_json::json!({
        "phase": phase,
        "iteration": iteration,
        "nodes": nodes,
        "constraints": constraint_values,
        "tree": tree,
        "candidates": candidates,
        "leave": leave,
        "enter": enter,
    });
    eprintln!(
        "RUST_RANK_OP {}",
        serde_json::to_string(&payload).expect("rank operation trace serializes")
    );
}

/// Emit the compound-order graph-order map and each sweep in a normalized
/// form that can be compared directly with Dagro's Graphlib node order.
/// Every lookup and serialization below is compiled out of ordinary builds.
pub(super) fn compound_order_nodes(all_nodes: &[d2_rs_core::NodeId], graph: &Graph) {
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("DAGRE_ORDER_TRACE").is_some() {
        for (index, (_, node)) in graph.nodes().enumerate() {
            let external_id =
                serde_json::to_string(&node.external_id).expect("identity serializes");
            eprintln!("RUST_DAGRE_GRAPH_INSERTION index={index} external_id={external_id}");
        }
        for (index, id) in graph.node_order_override.iter().enumerate() {
            if let Some(node) = graph.node(*id) {
                let external_id =
                    serde_json::to_string(&node.external_id).expect("identity serializes");
                eprintln!("RUST_DAGRE_SOURCE_ORDER index={index} external_id={external_id}");
            }
        }
        for (index, id) in all_nodes.iter().enumerate() {
            let external_id = graph
                .node(*id)
                .map(|node| serde_json::to_string(&node.external_id).expect("identity serializes"))
                .unwrap_or_else(|| "null".to_owned());
            eprintln!("RUST_DAGRE_ORDER_NODE index={index} id={}", id.0);
            eprintln!("RUST_DAGRE_OBJECT index={index} external_id={external_id}");
        }
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    let _ = (all_nodes, graph);
}

pub(super) fn compound_order_layers(
    phase: &str,
    iteration: isize,
    crossings: Option<usize>,
    layers: &[Vec<d2_rs_core::NodeId>],
    all_nodes: &[d2_rs_core::NodeId],
) {
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("DAGRE_ORDER_TRACE").is_some() {
        let indexes = all_nodes
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect::<BTreeMap<_, _>>();
        let normalized = layers
            .iter()
            .map(|layer| {
                layer
                    .iter()
                    .map(|id| indexes.get(id).copied().expect("ordered node has an index"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let crossing_bits = crossings
            .map(|value| format!("{:016x}", (value as f64).to_bits()))
            .unwrap_or_else(|| "none".to_owned());
        let layers = serde_json::to_string(&normalized).expect("normalized layers serialize");
        eprintln!(
            "RUST_DAGRE_LAYERS phase={phase} iteration={iteration} crossings_bits={crossing_bits} layers={layers}"
        );
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    let _ = (phase, iteration, crossings, layers, all_nodes);
}

/// Emit one Brandes–Köpf horizontal-compaction coordinate map in the same
/// graph-registration order used by the Go Dagro probe. This is diagnostic
/// only; it preserves the exact f64 result before global alignment.
pub(super) fn bk_positions(
    alignment: &str,
    positions: &BTreeMap<d2_rs_core::NodeId, f64>,
    all_nodes: &[d2_rs_core::NodeId],
) {
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("DAGRE_POS_TRACE").is_some() {
        for (index, id) in all_nodes.iter().enumerate() {
            if let Some(value) = positions.get(id) {
                eprintln!(
                    "RUST_DAGRE_POS alignment={alignment} index={index} id={} x_bits={:016x}",
                    id.0,
                    value.to_bits()
                );
            }
        }
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    let _ = (alignment, positions, all_nodes);
}

/// Emit the exact cross-axis spans and classifications used to construct
/// Brandes–Köpf separation constraints. This is diagnostic-only and lets the
/// parity harness distinguish incorrect geometry inputs from compaction bugs.
pub(super) fn bk_model(model: &super::BkModel, graph: &Graph) {
    if std::env::var_os("DAGRE_POS_TRACE").is_none() {
        return;
    }
    for (index, id) in model.all_nodes.iter().enumerate() {
        let external_id = graph
            .node(*id)
            .map(|node| node.external_id.as_str())
            .unwrap_or("");
        let span_bits = model
            .spans
            .get(id)
            .map(|value| format!("{:016x}", value.to_bits()))
            .unwrap_or_else(|| "missing".to_owned());
        let flow_span_bits = model
            .flow_spans
            .get(id)
            .map(|value| format!("{:016x}", value.to_bits()))
            .unwrap_or_else(|| "missing".to_owned());
        eprintln!(
            "RUST_BK_MODEL index={index} id={} external_id={:?} span_bits={span_bits} flow_span_bits={flow_span_bits} dummy={} original={}",
            id.0,
            external_id,
            model.dummies.contains(id),
            model.originals.contains(id),
        );
    }
}

/// Emit one layer-adjacency spacing constraint used by BK compaction.
#[allow(clippy::too_many_arguments)]
pub(super) fn bk_separation(
    model: &super::BkModel,
    layer_index: usize,
    pair_index: usize,
    left: d2_rs_core::NodeId,
    right: d2_rs_core::NodeId,
    left_root: d2_rs_core::NodeId,
    right_root: d2_rs_core::NodeId,
    distance: f64,
    block_distance: f64,
) {
    if std::env::var_os("DAGRE_POS_TRACE").is_none() {
        return;
    }
    let bits = |value: f64| format!("{:016x}", value.to_bits());
    eprintln!(
        "RUST_BK_SEP layer={layer_index} pair={pair_index} left={} left_root={} left_span_bits={} left_dummy={} right={} right_root={} right_span_bits={} right_dummy={} distance_bits={} block_distance_bits={}",
        left.0,
        left_root.0,
        bits(model.spans.get(&left).copied().unwrap_or(0.0)),
        model.dummies.contains(&left),
        right.0,
        right_root.0,
        bits(model.spans.get(&right).copied().unwrap_or(0.0)),
        model.dummies.contains(&right),
        bits(distance),
        bits(block_distance),
    );
}

/// Emit the Brandes–Köpf vertical-alignment roots in normalized node order.
pub(super) fn bk_roots(
    alignment: &str,
    roots: &BTreeMap<d2_rs_core::NodeId, d2_rs_core::NodeId>,
    all_nodes: &[d2_rs_core::NodeId],
) {
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("DAGRE_POS_TRACE").is_some() {
        let indexes = all_nodes
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect::<BTreeMap<_, _>>();
        for (index, id) in all_nodes.iter().enumerate() {
            if let Some(root) = roots.get(id) {
                let root_index = indexes.get(root).expect("BK root has a registration index");
                eprintln!(
                    "RUST_BK_ROOT alignment={alignment} index={index} id={} root_index={root_index} root={}",
                    id.0, root.0
                );
            }
        }
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    let _ = (alignment, roots, all_nodes);
}

/// Emit each unordered BK conflict pair in graph-registration order.
pub(super) fn bk_conflicts(
    kind: &str,
    conflicts: &BTreeSet<(d2_rs_core::NodeId, d2_rs_core::NodeId)>,
    all_nodes: &[d2_rs_core::NodeId],
) {
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("DAGRE_POS_TRACE").is_some() {
        let indexes = all_nodes
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect::<BTreeMap<_, _>>();
        let mut pairs = conflicts
            .iter()
            .map(|(left, right)| {
                let left = *indexes
                    .get(left)
                    .expect("conflict node has a registration index");
                let right = *indexes
                    .get(right)
                    .expect("conflict node has a registration index");
                (left.min(right), left.max(right))
            })
            .collect::<Vec<_>>();
        pairs.sort_unstable();
        let count = pairs.len();
        for (first, second) in pairs {
            eprintln!("RUST_BK_CONFLICT kind={kind} first_index={first} second_index={second}");
        }
        eprintln!("RUST_BK_CONFLICT_END kind={kind} count={count}");
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    let _ = (kind, conflicts, all_nodes);
}

/// Preserve the queue discovery order used by the cross-rank ancestor-growth
/// loop. Go ranges this set through a randomized map, so its order is observed
/// separately from the stable layout semantics.
pub(super) fn cross_seen_order(
    graph: &Graph,
    root: d2_rs_core::NodeId,
    start: f64,
    distance: f64,
    horizontal: bool,
    margin: bool,
    seen: &[d2_rs_core::NodeId],
) {
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("DAGRE_TRACE_CROSS_MAP").is_some() {
        let alias = |id: d2_rs_core::NodeId| {
            graph
                .node(id)
                .map(|node| node.external_id.clone())
                .unwrap_or_else(|| format!("<missing:{}>", id.0))
        };
        let root = serde_json::to_string(&alias(root)).expect("cross-map root serializes");
        eprintln!(
            "RUST_CROSS_MAP_BEGIN root={root} start_bits={:016x} distance_bits={:016x} horizontal={horizontal} margin={margin} count={}",
            start.to_bits(),
            distance.to_bits(),
            seen.len(),
        );
        for id in seen {
            let object = serde_json::to_string(&alias(*id)).expect("cross-map object serializes");
            eprintln!("RUST_CROSS_MAP_SEEN object={object}");
        }
        eprintln!("RUST_CROSS_MAP_END");
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    let _ = (graph, root, start, distance, horizontal, margin, seen);
}

pub(super) fn routes(graph: &Graph, routes: &BTreeMap<EdgeId, Vec<Point>>) {
    if !super::trace_enabled() {
        return;
    }
    eprintln!("RUST_EDGESET count={}", graph.edges().count());
    let mut ordinals = BTreeMap::<(&str, &str), usize>::new();
    for (id, edge) in graph.edges() {
        let source = graph.node(edge.source).expect("validated edge source");
        let target = graph.node(edge.target).expect("validated edge target");
        let ordinal = ordinals
            .entry((&source.external_id, &target.external_id))
            .or_default();
        let identity = format!(
            "{}:{} -> {}:{} #{}",
            source.external_id.len(),
            source.external_id,
            target.external_id.len(),
            target.external_id,
            ordinal,
        );
        *ordinal += 1;
        let identity = serde_json::to_string(&identity).expect("string serialization");
        let Some(points) = routes.get(&id) else {
            // Missing data must fail the gate instead of masquerading as an
            // empty route or silently reducing the declared edge inventory.
            eprintln!("RUST_EDGE {identity} <missing-route>");
            continue;
        };
        eprint!("RUST_EDGE {identity} point_count={} bits=", points.len());
        for (index, point) in points.iter().enumerate() {
            if index != 0 {
                eprint!(";");
            }
            eprint!("{:016x},{:016x}", point.x.to_bits(), point.y.to_bits());
        }
        eprintln!();
    }
}
