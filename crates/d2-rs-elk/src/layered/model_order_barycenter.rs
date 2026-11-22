// Copyright (c) 2021, 2025 Kiel University and others.
// Copyright (c) 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// Port of p3order/model_order_barycenter_heuristic.go and
// intermediate/preserveorder/cm_group_model_order.go at elk-go 8b285820520e.
use super::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Options {
    pub enabled: bool,
    needs_model_order: bool,
    strategy: String,
    enforced: Vec<i64>,
    maximum: i64,
}

pub(super) fn property<'a>(options: &'a Value, suffix: &str) -> Option<&'a Value> {
    options
        .get(format!("elk.layered.{suffix}"))
        .or_else(|| options.get(format!("org.eclipse.elk.layered.{suffix}")))
}

impl Options {
    pub fn parse(options: &Value) -> Self {
        let uses = |suffix: &str, choices: &[&str]| {
            property(options, suffix)
                .and_then(Value::as_str)
                .is_some_and(|v| choices.contains(&v))
        };
        let force = property(options, "crossingMinimization.forceNodeModelOrder")
            .is_some_and(|v| v.as_bool() == Some(true) || v.as_str() == Some("true"));
        let needs_model_order = force
            || property(options, "considerModelOrder.strategy")
                .and_then(Value::as_str)
                .is_some_and(|v| v != "NONE")
            || uses(
                "cycleBreaking.strategy",
                &[
                    "MODEL_ORDER",
                    "GREEDY_MODEL_ORDER",
                    "SCC_CONNECTIVITY",
                    "SCC_NODE_TYPE",
                    "DFS_NODE_ORDER",
                    "BFS_NODE_ORDER",
                ],
            )
            || uses(
                "layering.strategy",
                &["BREADTH_FIRST_MODEL_ORDER", "DEPTH_FIRST_MODEL_ORDER"],
            )
            || uses(
                "layering.nodePromotion.strategy",
                &["MODEL_ORDER_LEFT_TO_RIGHT", "MODEL_ORDER_RIGHT_TO_LEFT"],
            )
            || property(options, "considerModelOrder.components")
                .and_then(Value::as_str)
                .is_some_and(|v| v != "NONE")
            || [
                "considerModelOrder.crossingCounterNodeInfluence",
                "considerModelOrder.crossingCounterPortInfluence",
            ]
            .iter()
            .any(|key| {
                property(options, key)
                    .and_then(parse_option_double)
                    .is_some_and(|v| v != 0.)
            });
        Self {
            needs_model_order,
            enabled: property(options, "crossingMinimization.forceNodeModelOrder")
                .is_some_and(|v| v.as_bool() == Some(true) || v.as_str() == Some("true")),
            strategy: property(
                options,
                "considerModelOrder.groupModelOrder.cmGroupOrderStrategy",
            )
            .and_then(Value::as_str)
            .unwrap_or("ONLY_WITHIN_GROUP")
            .to_owned(),
            enforced: property(
                options,
                "considerModelOrder.groupModelOrder.cmEnforcedGroupOrders",
            )
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_else(|| vec![1, 2, 6, 7, 10, 11]),
            maximum: 0,
        }
    }
}

pub(super) fn import_orders(arena: &mut LayeredArena, root: LGraphId) {
    let mut graphs = vec![root];
    let mut index = 0;
    let mut order = 0;
    while index < graphs.len() {
        let graph = graphs[index];
        for node in arena.graphs[graph as usize].layerless_nodes.clone() {
            if arena.graphs[graph as usize]
                .forced_model_order
                .needs_model_order
            {
                if let Some((value, _)) = arena.node_model_order.get_mut(&node) {
                    *value = order;
                    order += 1;
                }
            } else {
                arena.node_model_order.remove(&node);
            }
            if let Some(nested) = arena.nodes[node as usize].nested_graph {
                graphs.push(nested);
            }
        }
        index += 1;
    }
    // Hierarchical importer sets the maximum on the top graph only.
    arena.graphs[root as usize].forced_model_order.maximum = order;
}

pub(super) fn sort(
    arena: &LayeredArena,
    graph: LGraphId,
    nodes: &mut [LNodeId],
    barycenters: &BTreeMap<LNodeId, f64>,
) {
    let config = &arena.graphs[graph as usize].forced_model_order;
    let mut bigger = BTreeMap::<LNodeId, BTreeSet<LNodeId>>::new();
    let mut smaller = bigger.clone();
    let mut compare = |first: LNodeId, second: LNodeId| {
        for node in [first, second] {
            bigger.entry(node).or_default();
            smaller.entry(node).or_default();
        }
        if bigger[&first].contains(&second) {
            return Ordering::Greater;
        }
        if bigger[&second].contains(&first) {
            return Ordering::Less;
        }
        if smaller[&first].contains(&second) {
            return Ordering::Less;
        }
        if smaller[&second].contains(&first) {
            return Ordering::Greater;
        }
        let mut result = Ordering::Equal;
        if let (Some(&(a, ag)), Some(&(b, bg))) = (
            arena.node_model_order.get(&first),
            arena.node_model_order.get(&second),
        ) {
            let group_offset = if config.strategy == "ENFORCED"
                && config.enforced.contains(&ag)
                && config.enforced.contains(&bg)
            {
                config.maximum
            } else {
                0
            };
            result = (a + group_offset * ag).cmp(&(b + group_offset * bg));
            if config.strategy == "ONLY_WITHIN_GROUP" && ag != bg {
                result = Ordering::Equal;
            }
        }
        if result == Ordering::Equal {
            result = match (barycenters.get(&first), barycenters.get(&second)) {
                (Some(a), Some(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                _ => Ordering::Equal,
            };
        }
        if result != Ordering::Equal {
            // Names and closure propagation deliberately follow the source:
            // update(bigger, smaller) records a comparator result of less.
            let (a, b) = if result == Ordering::Less {
                (first, second)
            } else {
                (second, first)
            };
            let b_bigger = bigger[&b].clone();
            let a_smaller = smaller[&a].clone();
            bigger.get_mut(&a).unwrap().insert(b);
            smaller.get_mut(&b).unwrap().insert(a);
            for small in &b_bigger {
                bigger.get_mut(&a).unwrap().insert(*small);
                smaller.get_mut(small).unwrap().insert(a);
                smaller
                    .get_mut(small)
                    .unwrap()
                    .extend(a_smaller.iter().copied());
            }
            for big in &a_smaller {
                smaller.get_mut(&b).unwrap().insert(*big);
                bigger.get_mut(big).unwrap().insert(b);
                bigger
                    .get_mut(big)
                    .unwrap()
                    .extend(b_bigger.iter().copied());
            }
        }
        result
    };
    for index in 1..nodes.len() {
        let insert = nodes[index];
        let mut position = index;
        while position > 0 && compare(nodes[position - 1], insert) == Ordering::Greater {
            nodes[position] = nodes[position - 1];
            position -= 1;
        }
        nodes[position] = insert;
    }
    // Transitive state is cleared after every forced-order layer sort.
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn group_order_and_barycenter_ties_follow_source_comparator() {
        // preserveorder/TestCalculateModelOrderOrGroupModelOrderEnforcesConfiguredGroups,
        // plus p3order's fallback when model orders compare equal.
        let wire: ElkGraph = serde_json::from_value(json!({"id":"root",
            "layoutOptions": {
                "elk.layered.crossingMinimization.forceNodeModelOrder":true,
                "elk.layered.considerModelOrder.groupModelOrder.cmGroupOrderStrategy":"ENFORCED"
            },
            "children":[{"id":"a","width":60,"height":40},{"id":"b","width":60,"height":40},{"id":"dummy","width":0,"height":0}]
        }))
        .unwrap();
        let (mut arena, root) = LayeredArena::from_elk_graph(&wire).unwrap();
        arena.node_model_order.insert(0, (1, 2));
        arena.node_model_order.insert(1, (7, 1));
        arena.node_model_order.remove(&2);
        arena.graphs[root as usize].forced_model_order.maximum = 8;
        let values = BTreeMap::from([(0, 0.), (1, 1.), (2, -1.)]);
        let mut nodes = vec![0, 1];
        sort(&arena, root, &mut nodes, &values);
        assert_eq!(nodes, [1, 0]); // 8*1+7 < 8*2+1
        arena.graphs[root as usize].forced_model_order.enforced = vec![1];
        sort(&arena, root, &mut nodes, &values);
        assert_eq!(nodes, [0, 1]); // Both groups must be enforced.
        arena.node_model_order.insert(1, (1, 2));
        let values = BTreeMap::from([(0, 1.), (1, 0.), (2, -1.)]);
        nodes = vec![0, 1, 2];
        sort(&arena, root, &mut nodes, &values);
        assert_eq!(nodes, [2, 1, 0]); // Equal/absent model order uses barycenters.
    }
}
