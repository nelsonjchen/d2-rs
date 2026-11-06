// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! optimization_guard.go530–546: charge legacy optimizer scoring scans.
use crate::internal::limits::{
    OptimizationError, OptimizationResourceLimitDetail, OptimizationWorkGuard,
};
use crate::internal::{ArenaGraph, NodeId};
pub(super) fn charge(
    owner: &mut ArenaGraph,
    node: Option<NodeId>,
    abductions: usize,
    symmetry: bool,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    let Some(node) = node else {
        return Err(OptimizationError::Invalid(format!(
            "TALA {} scoring requires a node with a graph",
            guard.location()
        )));
    };
    let Some(graph) = owner.topology_node_owner(node) else {
        return Err(OptimizationError::Invalid(format!(
            "TALA {} scoring requires a node with a graph",
            guard.location()
        )));
    };
    let edges = (owner
        .owner_node_incident_header(node)
        .map_err(|e| OptimizationError::Invalid(e.to_string()))?
        .len() as u64)
        .wrapping_add(1);
    let members = owner
        .owner_graph_member_header(graph)
        .map_err(|e| OptimizationError::Invalid(e.to_string()))?
        .len();
    let mut search = (members as u64)
        .wrapping_add(abductions as u64)
        .wrapping_add(1);
    if symmetry {
        search = search
            .checked_add(edges)
            .ok_or_else(|| OptimizationError::ResourceLimit {
                location: guard.location().into(),
                detail: OptimizationResourceLimitDetail::ScoringArithmeticOverflow,
            })?;
    }
    #[cfg(feature = "diagnostic-traces")]
    {
        guard.add_product_with_owner(edges, search, owner)
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        guard.add_product(edges, search)
    }
}
