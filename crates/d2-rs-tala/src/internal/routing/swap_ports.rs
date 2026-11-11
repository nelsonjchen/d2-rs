// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Exchange of compatible route endpoints around crowded node borders.
//!
//! Local segment order and side geometry identify inversions that can be
//! corrected without rerunning complete path search.

use super::*;

pub(in crate::internal) fn node_rect(graph: &ArenaGraph, node: NodeId) -> Option<Rect> {
    Some(Rect {
        origin: graph.nodes[node.0 as usize].position?,
        size: graph.nodes[node.0 as usize].rect.size,
    })
}

/// Translation of recovered `Node.SwapEdgePorts`, including same-side
/// uncrossing and adjacent-side tunnel refinement with rollback.
pub(in crate::internal) fn swap_edge_ports(graph: &mut ArenaGraph) -> bool {
    super::swap_owner::swap_edge_ports(graph)
}

/// Checked owner boundary with source work charges and shared-storage rollback.
pub(in crate::internal) fn swap_edge_ports_with_budget(
    graph: &mut ArenaGraph,
    budget: &super::route_search_guard::RouteStageWork,
) -> Result<(), super::route_stage_guard::RouteStageError> {
    super::swap_owner::swap_edge_ports_with_budget(graph, budget)
}
