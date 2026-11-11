// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Balancing of compatible parallel orthogonal route segments.
//!
//! Movable segments are grouped into channels and shifted toward regular
//! spacing without crossing obstacles or violating endpoint constraints.

use super::*;

pub(in crate::internal) fn balance_edge_segments(graph: &mut ArenaGraph) {
    super::balance_owner::balance_edge_segments(graph);
}

pub(in crate::internal) fn balance_edge_segments_with_budget(
    graph: &mut ArenaGraph,
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), super::route_stage_guard::RouteStageError> {
    super::balance_owner::balance_edge_segments_with_budget(graph, budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Graph, Node, ShapeKind, Size};

    #[test]
    fn balancing_obstacles_follow_current_graph_nodes_after_vessel_retirement() {
        let mut input = Graph::default();
        let source = input.add_node(Node::new(
            "source",
            Size {
                width: 100.0,
                height: 40.0,
            },
        ));
        let target = input.add_node(Node::new(
            "target",
            Size {
                width: 100.0,
                height: 40.0,
            },
        ));
        let retired = input.add_node(Node::new(
            "retired",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let edge = input.add_edge(Edge { source, target });
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[source.0 as usize].position = Some(Point { x: 0.0, y: -40.0 });
        graph.nodes[target.0 as usize].position = Some(Point { x: 0.0, y: 20.0 });
        graph.nodes[retired.0 as usize].position = Some(Point { x: 30.0, y: 5.0 });
        graph.set_owner_cluster_vessel_flag(retired, true).unwrap();
        // A vessel flag without its actual record is rejected by source
        // topology_preflight.go:1031, even if only retained Containers reach it.
        let kind = crate::internal::owner_groups::OwnerGroupKind::Cluster;
        let group = graph.new_owner_group(kind);
        let mut fields = graph.owner_group_fields(group).unwrap();
        fields.vessel = Some(retired);
        fields.graph = Some(crate::internal::OwnerGraphId(0));
        graph.set_owner_group_fields(group, fields).unwrap();
        let map = graph
            .owner_graph_group_map(crate::internal::OwnerGraphId(0), kind)
            .unwrap();
        graph
            .write_owner_group_map_entry(&map, Some(retired), Some(group))
            .unwrap();
        graph.node_order.retain(|node| *node != retired);
        graph.refresh_node_order_membership();
        graph.synchronize_original_owner_graph_view();
        graph.edge_records[edge.0 as usize]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: 40.0, y: 0.0 },
                Point { x: 40.0, y: 20.0 },
            ]));
        balance_edge_segments_with_budget(
            &mut graph,
            &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS),
        )
        .unwrap();
        assert_eq!(
            graph.edge_records[edge.0 as usize].route.read(),
            vec![Point { x: 50.0, y: 0.0 }, Point { x: 50.0, y: 20.0 }]
        );
        assert_eq!(
            graph.nodes[retired.0 as usize].position,
            Some(Point { x: 30.0, y: 5.0 })
        );

        // A reinserted allocated vessel is a real obstacle again. This is
        // slice membership, not a blanket exclusion of synthetic nodes.
        graph.node_order.push(retired);
        graph.refresh_node_order_membership();
        graph.synchronize_original_owner_graph_view();
        graph.edge_records[edge.0 as usize]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: 40.0, y: 0.0 },
                Point { x: 40.0, y: 20.0 },
            ]));
        balance_edge_segments_with_budget(
            &mut graph,
            &RouteStageWork::new(MAX_ROUTE_STAGE_WORK_UNITS),
        )
        .unwrap();
        assert_eq!(
            graph.edge_records[edge.0 as usize].route.read(),
            vec![Point { x: 70.0, y: 0.0 }, Point { x: 70.0, y: 20.0 }]
        );
    }

    #[test]
    fn fixed_port_interior_pass_skips_curve_routes() {
        let mut input = Graph::default();
        let mut source = Node::new(
            "source",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        source.shape = ShapeKind::Diamond;
        source.locked_position = Some(Point { x: 0.0, y: 0.0 });
        let source = input.add_node(source);
        let mut target = Node::new(
            "target",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        target.shape = ShapeKind::Diamond;
        target.locked_position = Some(Point { x: 300.0, y: 0.0 });
        let target = input.add_node(target);
        let edge = input.add_edge(Edge { source, target });
        let route = vec![
            Point { x: 100.0, y: 50.0 },
            Point { x: 130.0, y: 50.0 },
            Point { x: 130.0, y: 200.0 },
            Point { x: 220.0, y: 200.0 },
            Point { x: 220.0, y: 300.0 },
            Point { x: 300.0, y: 300.0 },
            Point { x: 300.0, y: 50.0 },
        ];
        input.set_edge_route(edge, route.clone());
        input.set_edge_is_curve(edge, true);

        let mut curved = ArenaGraph::from_input(&input);
        assert!(curved.edge_records[edge.0 as usize].is_curve);
        balance_edge_segments(&mut curved);
        assert_eq!(curved.edge_records[edge.0 as usize].route.read(), route);

        input.set_edge_is_curve(edge, false);
        let mut orthogonal = ArenaGraph::from_input(&input);
        balance_edge_segments(&mut orthogonal);
        assert_ne!(orthogonal.edge_records[edge.0 as usize].route.read(), route);
    }
}
