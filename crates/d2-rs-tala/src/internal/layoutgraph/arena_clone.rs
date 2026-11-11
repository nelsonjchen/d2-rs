// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::*;

// Numerical candidates detach graph storage once per allocation while retaining
// every internal route/node alias. ArenaEdge::clone itself copies a slice header.
impl Clone for ArenaGraph {
    fn clone(&self) -> Self {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(GraphClone);
        let headers = self
            .edge_records
            .iter()
            .map(|e| e.route.header().clone())
            .collect::<Vec<_>>();
        let (shared_topology, routes, maps) = self.shared_topology.clone_with_routes(
            &headers,
            &[
                self.containers.share_map(),
                self.hubs.header(),
                self.common_uncle_siblings.header(),
            ],
        );
        let mut edges = self.edge_records.clone();
        for (edge, route) in edges.iter_mut().zip(routes) {
            edge.route.replace(RoutePoints::from_header(route));
        }
        let mut result = Self {
            nodes: self.nodes.clone(),
            shared_topology,
            node_order: self.node_order.clone(),
            node_order_membership: self.node_order_membership.clone(),
            node_order_membership_valid: self.node_order_membership_valid,
            active_sequence_flags: self.active_sequence_flags.clone(),
            active_cluster_flags: self.active_cluster_flags.clone(),
            active_sequence_indices: self.active_sequence_indices.clone(),
            active_cluster_indices: self.active_cluster_indices.clone(),
            active_sequence_owners: self.active_sequence_owners.clone(),
            active_cluster_owners: self.active_cluster_owners.clone(),
            active_node_containers: self.active_node_containers.clone(),
            active_aggregate_vessels_by_member: self.active_aggregate_vessels_by_member.clone(),
            active_aggregate_vessel_nodes: self.active_aggregate_vessel_nodes.clone(),
            node_order_tala_carriers: self.node_order_tala_carriers.clone(),
            active_graph_node_order: self.active_graph_node_order.clone(),
            #[cfg(feature = "diagnostic-traces")]
            trace_owner_node_orders: self.trace_owner_node_orders.clone(),
            #[cfg(feature = "diagnostic-traces")]
            trace_root_owner_node_orders: self.trace_root_owner_node_orders.clone(),
            #[cfg(feature = "diagnostic-traces")]
            trace_root_owner_container_children: self.trace_root_owner_container_children.clone(),
            #[cfg(feature = "diagnostic-traces")]
            trace_container_placement_ranks: self.trace_container_placement_ranks.clone(),
            reachable_container_tala_ids: self.reachable_container_tala_ids.clone(),
            source_hierarchy_ordered: self.source_hierarchy_ordered,
            source_container_child_order: self.source_container_child_order.clone(),
            connected_container_child_order: self.connected_container_child_order.clone(),
            source_positions_by_tala: self.source_positions_by_tala.clone(),
            source_parent_tala_by_tala: self.source_parent_tala_by_tala.clone(),
            edge_records: edges,
            source_edge_endpoints: self.source_edge_endpoints.clone(),
            source_edge_endpoints_by_id: self.source_edge_endpoints_by_id.clone(),
            edge_order: self.edge_order.clone(),
            incident_edge_order: self.incident_edge_order.clone(),
            external_edge_counts: self.external_edge_counts.clone(),
            placement_scope_owned: self.placement_scope_owned,
            root_hierarchy: self.root_hierarchy,
            containers: ContainerMap::from_shared(maps[0].clone()),
            directions: self.directions.clone(),
            scoring_directions: self.scoring_directions.clone(),
            directions_by_tala: self.directions_by_tala.clone(),
            scoring_directions_by_tala: self.scoring_directions_by_tala.clone(),
            cell_size: self.cell_size,
            #[cfg(feature = "diagnostic-traces")]
            source_cell_size: self.source_cell_size,
            cost_cache: self.cost_cache.clone(),
            routing_costs: self.routing_costs,
            max_spacing_delta: self.max_spacing_delta,
            placement_component: self.placement_component.clone(),
            common_uncle_siblings: GraphNodeMap::from_header(maps[2].clone()),
            hubs: GraphNodeMap::from_header(maps[1].clone()),
            sized_adjacent_overrides: self.sized_adjacent_overrides.clone(),
            sized_cluster_distance_boxes: self.sized_cluster_distance_boxes.clone(),
            sized_edge_abductions: self.sized_edge_abductions.clone(),
            sized_edge_abductions_present: self.sized_edge_abductions_present,
            long_distance_neighbor_data: self.long_distance_neighbor_data.clone(),
            sized_projected_obstructions: self.sized_projected_obstructions.clone(),
            sized_projected_obstruction_sets: self.sized_projected_obstruction_sets.clone(),
            sized_collapsed_symmetry_neighbors: self.sized_collapsed_symmetry_neighbors.clone(),
            // A clone starts with no recorded fixpoints.
            projection_memo: ProjectionMemo::default(),
            suppress_aggregate_projection: self.suppress_aggregate_projection,
            swap_scoring_vessel_boxes: self.swap_scoring_vessel_boxes,
            transaction_external_containers: self.transaction_external_containers.clone(),
            transaction_validation_external_containers: self
                .transaction_validation_external_containers
                .clone(),
            transaction_external_container_children: self
                .transaction_external_container_children
                .clone(),
            trace_container_children: self.trace_container_children.clone(),
            transaction_external_aggregate_children: self
                .transaction_external_aggregate_children
                .clone(),
            transaction_external_cluster_layouts: self.transaction_external_cluster_layouts.clone(),
            projected_transaction_nodes: self.projected_transaction_nodes.clone(),
            root_grid_bin_pack_completed: self.root_grid_bin_pack_completed,
            sequences: self.sequences.clone(),
            sequence_backlinks_complete: self.sequence_backlinks_complete,
            clusters: self.clusters.clone(),
            cluster_vessel_sizes: self.cluster_vessel_sizes.clone(),
            serialized_vessel_sizes_by_tala: self.serialized_vessel_sizes_by_tala.clone(),
            serialized_vessel_positions_by_tala: self.serialized_vessel_positions_by_tala.clone(),
            serialized_cluster_arrangements_by_tala: self
                .serialized_cluster_arrangements_by_tala
                .clone(),
            serialized_cluster_edge_abductions: self.serialized_cluster_edge_abductions.clone(),
            serialized_sequence_edge_abductions: self.serialized_sequence_edge_abductions.clone(),
            serialized_cluster_containers_by_tala: self
                .serialized_cluster_containers_by_tala
                .clone(),
            pending_cluster_vessel_positions: self.pending_cluster_vessel_positions.clone(),
            cluster_backlinks_complete: self.cluster_backlinks_complete,
            preprocessed_tree_children: self.preprocessed_tree_children.clone(),
            restored_tree_edges: self.restored_tree_edges.clone(),
            published_tree_node_edge_order: self.published_tree_node_edge_order.clone(),
            restored_hierarchy_endpoint_order: self.restored_hierarchy_endpoint_order.clone(),
            tree_routing_nodes: self.tree_routing_nodes.clone(),
            tree_sentinels: self.tree_sentinels.clone(),
        };
        result.detach_incident_storage_from(self);
        result.detach_hierarchy_storage();
        result.bind_root_proximity_fields();
        result
    }
}
