// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Pinned routing/tunnel.go: live discovery and detached endpoint allocation.
use super::ovg_intersections::{Budget, check, descendant_optional, rect, step};
use super::{ArenaGraph, NodeId, Point, PortSide, ShapeKind};
use crate::internal::{OwnerGraphId, owner_groups::OwnerGroupKind};
use std::collections::BTreeSet;
const NIL: &str = "runtime error: invalid memory address or nil pointer dereference";

#[derive(Clone, Copy)]
pub(super) struct Entry {
    pub(super) owner: NodeId,
    pub(super) point: Point,
    // A view for existing Port consumers. Canonical owner metadata is EMPTY.
    pub(super) side: PortSide,
}
pub(super) type Tunnel = [Entry; 2];
fn adjacent(graph: &ArenaGraph, node: NodeId, edge: Option<crate::EdgeId>) -> Option<NodeId> {
    let edge = edge.expect(NIL);
    let from = graph
        .owner_edge_endpoint(edge, false)
        .expect("tunnel source endpoint");
    if from == Some(node) {
        graph
            .owner_edge_endpoint(edge, true)
            .expect("tunnel target endpoint")
    } else {
        from
    }
}

pub(super) fn between(
    graph: &ArenaGraph,
    nodes: &[NodeId],
    a: Option<NodeId>,
    b: Option<NodeId>,
    budget: Budget<'_, '_>,
) -> Result<Option<Vec<Tunnel>>, ()> {
    check(budget)?;
    let below_b = descendant_optional(graph, a, b, budget)?;
    let below_a = descendant_optional(graph, b, a, budget)?;
    if below_b || below_a {
        return Ok(None);
    }
    let a = a.expect(NIL);
    let b = b.expect(NIL);
    let Some((ranges, horizontal)) =
        super::tunnels::tunnel_ranges_between(graph, nodes, a, b, true, budget)?
    else {
        return Ok(None);
    };
    let mut remaining = 0.0;
    for edge in graph
        .owner_node_incident_header(a)
        .expect("tunnel incident header")
        .iter_slots()
    {
        step(budget)?;
        if adjacent(graph, a, edge) == Some(b) {
            remaining += 1.0;
        }
    }
    let mut result = Vec::new();
    for (start, end) in ranges {
        step(budget)?;
        let fit = super::channel_solver::minimum(((end - start) / 40.0).floor(), remaining);
        let mut i = 1.0;
        while i <= fit {
            step(budget)?;
            let value = (start + i * (end - start) / (fit + 1.0)).round();
            // Source newDerivedNode: reserve before allocating each temporary.
            step(budget)?;
            if let Some(b) = budget {
                b.reserve_nodes(1)?;
            }
            let mut pa = if horizontal {
                Point { x: 0.0, y: value }
            } else {
                Point { x: value, y: 0.0 }
            };
            step(budget)?;
            if let Some(b) = budget {
                b.reserve_nodes(1)?;
            }
            let mut pb = pa;
            step(budget)?;
            let ra = rect(graph, a);
            let rb = rect(graph, b);
            let (sa, sb) = if horizontal {
                if ra.origin.x > rb.origin.x {
                    pa.x = ra.origin.x;
                    pb.x = rb.origin.x + rb.size.width;
                    (PortSide::Left, PortSide::Right)
                } else {
                    pa.x = ra.origin.x + ra.size.width;
                    pb.x = rb.origin.x;
                    (PortSide::Right, PortSide::Left)
                }
            } else if ra.origin.y > rb.origin.y {
                pa.y = ra.origin.y;
                pb.y = rb.origin.y + rb.size.height;
                (PortSide::Top, PortSide::Bottom)
            } else {
                pa.y = ra.origin.y + ra.size.height;
                pb.y = rb.origin.y;
                (PortSide::Bottom, PortSide::Top)
            };
            result.push([
                Entry {
                    owner: a,
                    point: pa,
                    side: sa,
                },
                Entry {
                    owner: b,
                    point: pb,
                    side: sb,
                },
            ]);
            remaining -= 1.0;
            i += 1.0;
        }
    }
    Ok(Some(result))
}

pub(super) fn build(
    graph: &ArenaGraph,
    nodes: &[NodeId],
    budget: Budget<'_, '_>,
) -> Result<Vec<Tunnel>, ()> {
    check(budget)?;
    let mut special = BTreeSet::<Option<NodeId>>::new();
    if graph.shared_topology_initialized() {
        let clusters = graph
            .owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)
            .expect("tunnel cluster map");
        for cluster in graph
            .owner_group_map_values(&clusters)
            .expect("tunnel cluster values")
        {
            step(budget)?;
            let members = graph
                .owner_group_fields(cluster.expect(NIL))
                .expect("tunnel cluster fields")
                .nodes;
            for node in members.iter_slots() {
                step(budget)?;
                special.insert(node);
            }
        }
        let trees = graph
            .owner_graph_node_to_tree_map(OwnerGraphId(0))
            .expect("tunnel tree map");
        for node in trees.live_keys() {
            step(budget)?;
            special.insert(node);
        }
    } else {
        for cluster in &graph.clusters {
            step(budget)?;
            for &node in &cluster.members {
                step(budget)?;
                special.insert(Some(node));
            }
        }
        for &node in graph.tree_routing_nodes.keys() {
            step(budget)?;
            special.insert(Some(node));
        }
    }
    let mut result = Vec::new();
    let mut searched = BTreeSet::new();
    for &node in nodes {
        step(budget)?;
        if special.contains(&Some(node))
            || graph.nodes[node.0 as usize].shape == ShapeKind::SqlTable
        {
            continue;
        }
        for edge in graph
            .owner_node_incident_header(node)
            .expect("tunnel incident header")
            .iter_slots()
        {
            step(budget)?;
            let other = adjacent(graph, node, edge);
            if other == Some(node) || special.contains(&other) {
                continue;
            }
            if !searched.insert((Some(node), other)) {
                continue;
            }
            searched.insert((other, Some(node)));
            if let Some(tunnels) = between(graph, nodes, Some(node), other, budget)? {
                result.extend(tunnels);
            }
        }
    }
    Ok(result)
}

#[cfg(feature = "diagnostic-traces")]
pub(super) fn observe(graph: &ArenaGraph, tunnels: &[Tunnel]) -> serde_json::Value {
    serde_json::json!(
        tunnels
            .iter()
            .map(|entries| {
                entries.iter().map(|e| serde_json::json!({
        "owner":graph.nodes[e.owner.0 as usize].tala_id,
        "point":[format!("{:016x}",e.point.x.to_bits()),format!("{:016x}",e.point.y.to_bits())],
        "directions":0,"center":false,
    })).collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    )
}

#[cfg(test)]
mod tests {
    use super::super::{ovg_ports::PortBuild, route_search_guard::OvgBuildBudget};
    use super::*;
    fn graph(x: f64, size: f64, edges: usize) -> ArenaGraph {
        let data = serde_json::json!({"nodes":[
            {"id":1,"width":size,"height":size,"topLeft":{"x":0,"y":0}},
            {"id":2,"width":size,"height":size,"topLeft":{"x":x,"y":0}}
        ],"edges":(0..edges).map(|i|serde_json::json!({"id":i+11,"from":1,"to":2,"points":[]})).collect::<Vec<_>>(),"containers":{"0":[1,2]}});
        ArenaGraph::from_input(
            &crate::graphjson::graphjson_to_graph(&serde_json::to_vec(&data).unwrap()).unwrap(),
        )
    }
    #[test]
    fn touching_owners_keep_their_roles_and_both_self_edge_incidences() {
        let g = graph(100.0, 100.0, 1);
        let nodes = g.graph_node_order();
        let mut ovg = PortBuild::default();
        ovg.add_ports(&g, &nodes, None).unwrap();
        let before = ovg.nodes.len();
        let b = OvgBuildBudget::new();
        ovg.add_tunnels(&g, &nodes, Some(&b)).unwrap();
        assert_eq!(ovg.nodes.len(), before);
        assert_eq!(
            b.observed_resources()[1],
            2,
            "temporary endpoint allocations are distinct from canonical memberships"
        );
        assert_eq!(ovg.edges.len(), 1);
        let e = ovg.edges[0].read();
        assert_eq!(e.from, e.to);
        assert_eq!(e.distance, 0.0);
        assert_eq!(ovg.nodes[e.from_slot()].read().edges, [0, 0]);
        assert!(ovg.nodes[e.from_slot()].read().is_tunnel);
        assert_eq!(ovg.port_directions(e.from_slot(), NodeId(0)), 1 << 5);
        assert_eq!(ovg.port_directions(e.from_slot(), NodeId(1)), 1 << 7);
        assert_eq!(
            ovg.ports[&NodeId(0)].last().unwrap().0,
            ovg.ports[&NodeId(1)].last().unwrap().0
        );
    }
    #[test]
    fn detached_parallel_endpoints_have_even_source_spacing() {
        let g = graph(5000.0, 1000.0, 3);
        let tunnels = build(&g, &g.graph_node_order(), None).unwrap();
        assert_eq!(tunnels.len(), 3);
        assert_eq!(
            tunnels.iter().map(|t| t[0].point).collect::<Vec<_>>(),
            [
                Point {
                    x: 1000.0,
                    y: 250.0
                },
                Point {
                    x: 1000.0,
                    y: 500.0
                },
                Point {
                    x: 1000.0,
                    y: 750.0
                }
            ]
        );
        assert!(tunnels.iter().all(|t| t[1].point.x == 5000.0));
    }
    #[test]
    fn nil_adjacent_endpoint_runs_both_ancestry_checks_before_skipping() {
        let mut g = graph(500.0, 100.0, 1);
        g.set_owner_edge_endpoint(crate::EdgeId(0), true, None)
            .unwrap();
        let b = OvgBuildBudget::new();
        assert!(
            between(&g, &g.graph_node_order(), Some(NodeId(0)), None, Some(&b))
                .unwrap()
                .is_none()
        );
        assert_eq!(b.observed_resources()[0], 2);
    }
}
