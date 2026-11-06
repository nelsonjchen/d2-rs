// Copyright (c) 2016, 2020 Kiel University and others.
// Rust translation and modifications: Copyright 2026 Nelson Chen.
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// https://www.eclipse.org/legal/epl-2.0.
// SPDX-License-Identifier: EPL-2.0

//! Source ForsterConstraintResolver group graph, violation queue and merges.
use super::{
    LGraphId, LNodeId, LayeredArena, LayeredNodeType, PortSide, SourceBarycenterState,
    port_constraints_fix_side,
};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Constraints {
    units: BTreeMap<LNodeId, Vec<LNodeId>>,
    successors: BTreeMap<LNodeId, Vec<LNodeId>>,
}

impl Constraints {
    pub(super) fn from_graph(arena: &LayeredArena, graph: LGraphId) -> Self {
        let mut result = Self::default();
        let mut unit_of = BTreeMap::new();
        for (id, node) in arena.nodes.iter().enumerate() {
            if node.graph == graph
                && node.node_type == LayeredNodeType::Normal
                && port_constraints_fix_side(node.port_constraints)
            {
                unit_of.insert(id as LNodeId, id as LNodeId);
            }
        }
        for dummy in arena
            .north_south_port_dummies
            .iter()
            .filter(|d| d.graph == graph)
        {
            let port = &arena.ports[dummy.origin_port as usize];
            let owner = port.node;
            unit_of.insert(dummy.node, owner);
            if arena.non_flow_switch_ports.contains(&dummy.origin_port) {
                continue;
            }
            let (first, second) = if port.side == PortSide::North {
                (dummy.node, owner)
            } else {
                (owner, dummy.node)
            };
            result.successors.entry(first).or_default().push(second);
        }
        for layer in &arena.graphs[graph as usize].layers {
            for node in &arena.layers[*layer as usize].nodes {
                if let Some(owner) = unit_of.get(node) {
                    result.units.entry(*owner).or_default().push(*node);
                }
            }
        }
        result
    }

    pub(super) fn process(
        &self,
        arena: &LayeredArena,
        nodes: &mut Vec<LNodeId>,
        states: &mut [SourceBarycenterState],
    ) {
        let mut groups = nodes
            .iter()
            .map(|n| Group {
                nodes: vec![*n],
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let group_of = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, i))
            .collect::<BTreeMap<_, _>>();
        let mut list = (0..nodes.len()).collect::<Vec<_>>();
        let mut last_normal = None;
        for group in 0..groups.len() {
            let node = groups[group].nodes[0];
            if let Some(successors) = self.successors.get(&node) {
                for successor in successors {
                    if let Some(target) = group_of.get(successor) {
                        add_constraint(&mut groups, group, *target);
                    }
                }
            }
            if arena.nodes[node as usize].node_type == LayeredNodeType::Normal {
                if let Some(previous) = last_normal {
                    if let (Some(previous_unit), Some(current_unit)) =
                        (self.units.get(&previous), self.units.get(&node))
                    {
                        for before in previous_unit {
                            for after in current_unit {
                                if let (Some(a), Some(b)) =
                                    (group_of.get(before), group_of.get(after))
                                {
                                    add_constraint(&mut groups, *a, *b);
                                }
                            }
                        }
                    }
                }
                last_normal = Some(node);
            }
        }
        while let Some((first, second)) = find_violation(&mut groups, &list, states) {
            let merged = merge_groups(&mut groups, first, second, states);
            let merged_barycenter = barycenter(&groups, merged, states);
            let mut inserted = false;
            let mut index = 0;
            while index < list.len() {
                let group = list[index];
                if group == first || group == second {
                    list.remove(index);
                    continue;
                }
                if !inserted
                    && barycenter(&groups, group, states)
                        .zip(merged_barycenter)
                        .is_some_and(|(old, new)| old > new)
                {
                    list.insert(index, merged);
                    inserted = true;
                    index += 1;
                    continue;
                }
                let first_removed = remove_first(&mut groups[group].outgoing, first);
                let second_removed = remove_first(&mut groups[group].outgoing, second);
                if first_removed || second_removed {
                    groups[group].outgoing.get_or_insert_default().push(merged);
                    groups[merged].incoming_count += 1;
                }
                index += 1;
            }
            if !inserted {
                list.push(merged);
            }
        }
        nodes.clear();
        for group in list {
            let value = barycenter(&groups, group, states);
            for node in &groups[group].nodes {
                nodes.push(*node);
                states[*node as usize].barycenter = value;
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Group {
    summed_weight: f64,
    degree: usize,
    nodes: Vec<LNodeId>,
    // Preserve Go nil versus a nonnil empty slice when merging constraints.
    outgoing: Option<Vec<usize>>,
    incoming: Vec<usize>,
    incoming_count: usize,
}
fn add_constraint(groups: &mut [Group], first: usize, second: usize) {
    groups[first].outgoing.get_or_insert_default().push(second);
    groups[second].incoming_count += 1;
}
fn barycenter(groups: &[Group], group: usize, states: &[SourceBarycenterState]) -> Option<f64> {
    states[groups[group].nodes[0] as usize].barycenter
}
fn find_violation(
    groups: &mut [Group],
    list: &[usize],
    states: &[SourceBarycenterState],
) -> Option<(usize, usize)> {
    let mut active = VecDeque::new();
    for group in list {
        groups[*group].incoming.clear();
        if groups[*group]
            .outgoing
            .as_ref()
            .is_some_and(|v| !v.is_empty())
            && groups[*group].incoming_count == 0
        {
            active.push_back(*group);
        }
    }
    while let Some(group) = active.pop_front() {
        let value = barycenter(groups, group, states);
        for previous in &groups[group].incoming {
            if let (Some(before), Some(after)) = (barycenter(groups, *previous, states), value) {
                if before as f32 == after as f32 {
                    if list.iter().position(|v| v == previous)
                        > list.iter().position(|v| *v == group)
                    {
                        return Some((*previous, group));
                    }
                } else if before > after {
                    return Some((*previous, group));
                }
            }
        }
        for successor in groups[group].outgoing.clone().unwrap_or_default() {
            groups[successor].incoming.insert(0, group);
            if groups[successor].incoming_count == groups[successor].incoming.len() {
                active.push_back(successor);
            }
        }
    }
    None
}
fn remove_first(values: &mut Option<Vec<usize>>, remove: usize) -> bool {
    if let Some(values) = values
        && let Some(i) = values.iter().position(|v| *v == remove)
    {
        values.remove(i);
        true
    } else {
        false
    }
}
fn copied_slice(values: &Option<Vec<usize>>) -> Option<Vec<usize>> {
    values.as_ref().filter(|v| !v.is_empty()).cloned()
}
fn merge_groups(
    groups: &mut Vec<Group>,
    first: usize,
    second: usize,
    states: &mut [SourceBarycenterState],
) -> usize {
    let a = groups[first].clone();
    let b = groups[second].clone();
    let mut nodes = a.nodes.clone();
    nodes.extend_from_slice(&b.nodes);
    let mut outgoing = None;
    if a.outgoing.is_some() {
        outgoing = copied_slice(&a.outgoing);
        remove_first(&mut outgoing, second);
        for candidate in b.outgoing.as_deref().unwrap_or_default() {
            if *candidate == first {
                continue;
            }
            if outgoing.as_ref().is_some_and(|v| v.contains(candidate)) {
                groups[*candidate].incoming_count -= 1;
            } else {
                outgoing.get_or_insert_default().push(*candidate);
            }
        }
    } else if b.outgoing.is_some() {
        outgoing = copied_slice(&b.outgoing);
        remove_first(&mut outgoing, first);
    }
    // Source groups start at zero, independently of node state weights.
    // Preserve its unweighted barycenter fallback rather than copying degree.
    let summed_weight = a.summed_weight + b.summed_weight;
    let degree = a.degree + b.degree;
    let value = if degree > 0 {
        Some(summed_weight / degree as f64)
    } else {
        match (
            barycenter(groups, first, states),
            barycenter(groups, second, states),
        ) {
            (Some(a), Some(b)) => Some((a + b) / 2.0),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    };
    if let Some(value) = value {
        for node in &nodes {
            states[*node as usize].barycenter = Some(value);
        }
    }
    let id = groups.len();
    groups.push(Group {
        summed_weight,
        degree,
        nodes,
        outgoing,
        ..Default::default()
    });
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    fn two_nodes() -> (LayeredArena, LGraphId) {
        let graph = serde_json::from_value(
            serde_json::json!({"id":"root","children":[{"id":"first","width":40,"height":30},{"id":"second","width":40,"height":30}]}),
        )
        .unwrap();
        super::super::LayeredArena::from_elk_graph(&graph).unwrap()
    }
    #[test]
    fn original_go_forster_merges_violated_groups_with_unweighted_fallback() {
        let (arena, _) = two_nodes();
        let constraints = Constraints {
            successors: BTreeMap::from([(1, vec![0])]),
            ..Default::default()
        };
        let mut states = vec![
            SourceBarycenterState {
                barycenter: Some(0.0),
                degree: 1,
                ..Default::default()
            },
            SourceBarycenterState {
                barycenter: Some(1.0),
                summed_weight: 100.0,
                degree: 100,
                ..Default::default()
            },
        ];
        let mut order = vec![0, 1];
        constraints.process(&arena, &mut order, &mut states);
        assert_eq!(order, vec![1, 0]);
        assert_eq!(states[0].barycenter, Some(0.5));
        assert_eq!(states[1].barycenter, Some(0.5));
    }
    #[test]
    fn original_go_forster_equal_float32_uses_list_order_before_f64_order() {
        let (arena, _) = two_nodes();
        let constraints = Constraints {
            successors: BTreeMap::from([(1, vec![0])]),
            ..Default::default()
        };
        let a = 1.00000002_f64;
        let b = 1.0_f64;
        assert!(b < a);
        assert_eq!((a as f32).to_bits(), (b as f32).to_bits());
        let mut states = vec![
            SourceBarycenterState {
                barycenter: Some(a),
                ..Default::default()
            },
            SourceBarycenterState {
                barycenter: Some(b),
                ..Default::default()
            },
        ];
        let mut order = vec![0, 1];
        constraints.process(&arena, &mut order, &mut states);
        assert_eq!(order, vec![1, 0]);
        assert_eq!(states[0].barycenter, Some((a + b) / 2.0));
    }
}
