// Copyright (c) 2016 Kiel University.
// Copyright (c) 2026 The elk-go Authors.
// Rust translation and modifications: Copyright 2026 Nelson Chen.
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// https://www.eclipse.org/legal/epl-2.0.
// SPDX-License-Identifier: EPL-2.0

//! Two-sided GreedyPortDistributor and its adjacent-port crossing decisions.
use super::*;
use d2_rs_core::stable_sort::StableSort;

pub(super) struct GreedyPortDistributor {
    counter: P3CrossingsCounter,
}

impl GreedyPortDistributor {
    pub(super) fn new(arena: &LayeredArena) -> Self {
        Self {
            counter: P3CrossingsCounter {
                port_positions: vec![0; arena.ports.len()],
            },
        }
    }

    pub(super) fn distribute(
        &mut self,
        arena: &mut LayeredArena,
        graph: LGraphId,
        current: usize,
        forward: bool,
    ) -> bool {
        let layers = arena.graphs[graph as usize].layers.clone();
        let nodes = arena.layers[layers[current] as usize].nodes.clone();
        let side = if forward {
            PortSide::West
        } else {
            PortSide::East
        };
        let mut ports = Vec::new();
        if forward && current > 0 {
            self.counter.init_positions(
                arena,
                &arena.layers[layers[current - 1] as usize].nodes,
                PortSide::East,
                true,
                &mut ports,
            );
            self.counter
                .init_positions(arena, &nodes, PortSide::West, false, &mut ports);
        } else if !forward && current + 1 < layers.len() {
            self.counter
                .init_positions(arena, &nodes, PortSide::East, true, &mut ports);
            self.counter.init_positions(
                arena,
                &arena.layers[layers[current + 1] as usize].nodes,
                PortSide::West,
                false,
                &mut ports,
            );
        } else {
            self.counter
                .init_positions(arena, &nodes, side, true, &mut ports);
        }
        let tree_size = ports.len();
        let mut improved = false;
        for node in nodes {
            if matches!(
                arena.nodes[node as usize].port_constraints,
                PortConstraints::FixedOrder
                    | PortConstraints::FixedRatio
                    | PortConstraints::FixedPosition
            ) {
                continue;
            }
            let mut side_ports = p3_node_ports_on_side(arena, node, side);
            if matches!(side, PortSide::South | PortSide::West) {
                side_ports.reverse();
            }
            loop {
                let mut changed = false;
                for i in 0..side_ports.len().saturating_sub(1) {
                    let upper = side_ports[i];
                    let lower = side_ports[i + 1];
                    let (mut before, mut after) =
                        self.count_both_orders(arena, upper, lower, tree_size);
                    if let Some(child) = arena.nodes[node as usize].nested_graph {
                        let (inner_before, inner_after) =
                            hierarchical_costs(arena, child, upper, lower, forward);
                        before += inner_before;
                        after += inner_after;
                    }
                    if before > after {
                        self.counter
                            .port_positions
                            .swap(upper as usize, lower as usize);
                        let all = &mut arena.nodes[node as usize].ports;
                        let upper_index = all
                            .iter()
                            .position(|p| *p == upper)
                            .expect("port belongs to node");
                        let lower_index = all
                            .iter()
                            .position(|p| *p == lower)
                            .expect("port belongs to node");
                        all.swap(upper_index, lower_index);
                        side_ports.swap(i, i + 1);
                        changed = true;
                        improved = true;
                    }
                }
                if !changed {
                    break;
                }
            }
        }
        improved
    }

    fn count_both_orders(
        &mut self,
        arena: &LayeredArena,
        upper: LPortId,
        lower: LPortId,
        tree_size: usize,
    ) -> (usize, usize) {
        // Source connectedPortsSortedByPosition de-duplicates by scratch
        // position, retaining the last encountered port for each position.
        let mut by_position = BTreeMap::new();
        for port in [upper, lower] {
            by_position.insert(self.counter.port_positions[port as usize], port);
            for edge in p3_connected_edges(arena, port) {
                let edge = &arena.edges[edge as usize];
                if edge.source_port == edge.target_port {
                    continue;
                }
                if let Some(other) = p3_other_port(edge, port) {
                    by_position.insert(self.counter.port_positions[other as usize], other);
                }
            }
        }
        let mut ports = by_position.into_values().collect::<Vec<_>>();
        let before = p3_count_crossings_on_ports(
            arena,
            &self.counter.port_positions,
            &ports,
            &mut P3BinaryIndexedTree::new(tree_size),
        );
        self.counter
            .port_positions
            .swap(upper as usize, lower as usize);
        ports.stable_sort_by_key(|p| self.counter.port_positions[*p as usize]);
        let after = p3_count_crossings_on_ports(
            arena,
            &self.counter.port_positions,
            &ports,
            &mut P3BinaryIndexedTree::new(tree_size),
        );
        self.counter
            .port_positions
            .swap(lower as usize, upper as usize);
        (before, after)
    }
}

fn hierarchical_costs(
    arena: &LayeredArena,
    child: LGraphId,
    upper: LPortId,
    lower: LPortId,
    forward: bool,
) -> (usize, usize) {
    let dummy = |port| {
        arena
            .external_port_origins
            .iter()
            .find_map(|(node, origin)| {
                (*origin == port && arena.nodes[*node as usize].graph == child).then_some(*node)
            })
    };
    let (Some(upper), Some(lower)) = (dummy(upper), dummy(lower)) else {
        return (0, 0);
    };
    let layers = &arena.graphs[child as usize].layers;
    let free = if forward { 0 } else { layers.len() - 1 };
    let mut result = (0, 0);
    for side in [PortSide::West, PortSide::East] {
        let neighbor = if side == PortSide::West {
            free.checked_sub(1)
        } else {
            (free + 1 < layers.len()).then_some(free + 1)
        };
        let Some(neighbor) = neighbor else {
            continue;
        };
        let neighbor_side = if side == PortSide::West {
            PortSide::East
        } else {
            PortSide::West
        };
        let mut positions = BTreeMap::new();
        for node in &arena.layers[layers[neighbor] as usize].nodes {
            let mut ports = p3_node_ports_on_side(arena, *node, neighbor_side);
            if matches!(neighbor_side, PortSide::South | PortSide::West) {
                ports.reverse();
            }
            for port in ports {
                let position = positions.len();
                positions.insert(port, position);
            }
        }
        let adjacent = |node| {
            let mut values = Vec::new();
            for port in p3_node_ports_on_side(arena, node, side) {
                let p = &arena.ports[port as usize];
                let edges = if side == PortSide::West {
                    &p.incoming_edges
                } else {
                    &p.outgoing_edges
                };
                for edge in edges {
                    let edge = &arena.edges[*edge as usize];
                    if !edge.active
                        || edge.source == edge.target
                        || p3_edge_is_in_layer(arena, edge)
                    {
                        continue;
                    }
                    let other = p3_other_port(edge, port).expect("connected edge endpoint");
                    values.push(positions[&other]);
                }
            }
            values
        };
        let upper_positions = adjacent(upper);
        let lower_positions = adjacent(lower);
        // These products preserve portAdjacencyList cardinalities, including
        // repeated endpoints. Equal positions do not cross in either order.
        for upper in &upper_positions {
            for lower in &lower_positions {
                result.0 += usize::from(upper > lower);
                result.1 += usize::from(lower > upper);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn original_go_adjacent_port_swap_stops_at_optimum_and_respects_fixed_order() {
        for fixed in [false, true] {
            let mut input = json!({
                "id":"root", "children":[
                    {"id":"a","ports":[{"id":"a.p","layoutOptions":{"elk.port.side":"EAST"}}]},
                    {"id":"b","ports":[{"id":"b.p","layoutOptions":{"elk.port.side":"EAST"}}]},
                    {"id":"r","ports":[
                        {"id":"r.first","layoutOptions":{"elk.port.side":"WEST"}},
                        {"id":"r.second","layoutOptions":{"elk.port.side":"WEST"}}
                    ]}
                ], "edges":[
                    {"id":"e0","sources":["a.p"],"targets":["r.first"]},
                    {"id":"e1","sources":["b.p"],"targets":["r.second"]}
                ]
            });
            // ElkPort is the typed model, whose coordinates and dimensions
            // are required; the source test creates zero-valued arena ports.
            for node in input["children"].as_array_mut().unwrap() {
                node["width"] = json!(0.0);
                node["height"] = json!(0.0);
                for port in node["ports"].as_array_mut().unwrap() {
                    for key in ["x", "y", "width", "height"] {
                        port[key] = json!(0.0);
                    }
                }
            }
            let graph: crate::ElkGraph = serde_json::from_value(input).unwrap();
            let (mut arena, root) = LayeredArena::from_elk_graph(&graph).unwrap();
            arena.graphs[root as usize].layers = vec![0, 1];
            arena.layers = vec![
                Layer {
                    graph: root,
                    nodes: vec![0, 1],
                    width: 0.0,
                    height: 0.0,
                },
                Layer {
                    graph: root,
                    nodes: vec![2],
                    width: 0.0,
                    height: 0.0,
                },
            ];
            for node in [0, 1] {
                arena.nodes[node].layer = Some(0);
            }
            arena.nodes[2].layer = Some(1);
            if fixed {
                arena.nodes[2].port_constraints = PortConstraints::FixedOrder;
            }
            let initial = arena.nodes[2].ports.clone();
            let mut distributor = GreedyPortDistributor::new(&arena);
            assert_eq!(distributor.distribute(&mut arena, root, 1, true), !fixed);
            let expected = if fixed {
                initial.clone()
            } else {
                initial.into_iter().rev().collect()
            };
            assert_eq!(arena.nodes[2].ports, expected);
            assert!(!distributor.distribute(&mut arena, root, 1, true));
        }
    }
}
