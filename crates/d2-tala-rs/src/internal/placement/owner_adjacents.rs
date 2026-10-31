// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! optimization_guard.go176–329: collect neighbors from actual retained
//! incident/abduction headers, then live selected graph group/tree fields.
use super::container_slices::SharedNodeSlice;
use super::limits::{OptimizationError, OptimizationWorkGuard};
use super::owner_groups::OwnerGroupKind;
use super::owner_median::{
    append_owner_tree_nears, optimizer_owner_ordered_nears, optimizer_owner_tree_root,
};
use super::owner_near_eligibility::{OwnerNearEligibilityError, optimizer_owner_is_descendant_of};
use super::owner_placement::OwnerPlacementError;
use super::shared_topology::OwnerAbductionHeader;
use super::{ArenaGraph, NodeId, OwnerGraphId, OwnerTopologyError, OwnerTreeId};

fn invalid(guard: &OptimizationWorkGuard<'_>, message: &str) -> OwnerPlacementError {
    OptimizationError::Invalid(format!("TALA {} {message}", guard.location())).into()
}
fn step(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    return guard.step_with_owner(owner);
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.step()
    }
}
fn live_graph(owner: &ArenaGraph, node: NodeId) -> OwnerGraphId {
    owner
        .topology_node_owner(node)
        .expect("runtime error: invalid memory address or nil pointer dereference")
}
fn positioned(owner: &ArenaGraph, node: Option<NodeId>) -> Result<bool, OwnerTopologyError> {
    match node {
        None => Ok(false),
        Some(node) => {
            owner
                .topology_node(node)
                .ok_or(OwnerTopologyError::UnknownNode(node))?;
            Ok(owner.position(node).is_some())
        }
    }
}
fn append(
    mut nodes: SharedNodeSlice,
    node: NodeId,
) -> Result<SharedNodeSlice, OwnerPlacementError> {
    nodes = nodes
        .append_node(node)
        .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
    Ok(nodes)
}
fn append_bounded(
    nodes: SharedNodeSlice,
    ordered: &SharedNodeSlice,
    guard: &OptimizationWorkGuard<'_>,
) -> Result<SharedNodeSlice, OwnerPlacementError> {
    if nodes.len() > 1_000_000usize.saturating_sub(ordered.len()) {
        return Err(invalid(
            guard,
            "optimizer node references exceed limit 1000000",
        ));
    }
    nodes
        .append(&ordered.visible_slots())
        .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()).into())
}
fn sentinel(owner: &ArenaGraph, tree: OwnerTreeId) -> Result<Option<NodeId>, OwnerTopologyError> {
    let fields = owner
        .owner_tree(tree)
        .ok_or(OwnerTopologyError::UnknownTree(tree))?
        .fields();
    let edge = fields
        .sentinel_edge
        .expect("runtime error: invalid memory address or nil pointer dereference");
    let from = owner.owner_edge_endpoint(edge, false)?;
    if from == fields.node {
        owner.owner_edge_endpoint(edge, true)
    } else {
        Ok(from)
    }
}
fn project_near(
    owner: &mut ArenaGraph,
    node: NodeId,
    near: Option<NodeId>,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<Option<NodeId>, OwnerPlacementError> {
    let near = near.expect("runtime error: invalid memory address or nil pointer dereference");
    let mut projected = Some(near);
    for kind in [OwnerGroupKind::Cluster, OwnerGroupKind::Sequence] {
        if let Some(vessel) = owner.owner_node_group_vessel(near, kind)? {
            let vessel =
                vessel.expect("runtime error: invalid memory address or nil pointer dereference");
            if owner.topology_node_owner(vessel).is_some() {
                projected = Some(vessel);
                break;
            }
        }
    }
    // Node.Graph is evaluated after group projection, and again for every
    // projection; a root traversal can replace it for the next caller iteration.
    if let Some(tree) = owner.owner_graph_node_to_tree_entry(live_graph(owner, node), projected)? {
        let root = optimizer_owner_tree_root(owner, tree, guard)?;
        let Some(root) = root else {
            return Err(invalid(guard, "found a tree without a sentinel"));
        };
        if sentinel(owner, root)?.is_none() {
            return Err(invalid(guard, "found a tree without a sentinel"));
        }
        projected = sentinel(owner, root)?;
    }
    Ok(projected)
}

pub(super) fn optimizer_owner_adjacents(
    owner: &mut ArenaGraph,
    node: Option<NodeId>,
    abductions: &OwnerAbductionHeader,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<SharedNodeSlice, OwnerPlacementError> {
    let Some(node) = node else {
        return Err(invalid(
            guard,
            "adjacency lookup requires a node with a graph",
        ));
    };
    owner
        .topology_node(node)
        .ok_or(OwnerTopologyError::UnknownNode(node))?;
    if owner.topology_node_owner(node).is_none() {
        return Err(invalid(
            guard,
            "adjacency lookup requires a node with a graph",
        ));
    }
    let incidents = owner.owner_node_incident_header(node)?;
    owner.validate_owner_abduction_header(abductions)?;
    if incidents.len() > 50_000 || abductions.len() > 50_000 {
        return Err(invalid(guard, "adjacency inputs exceed edge limit 50000"));
    }
    let mut adjacents = SharedNodeSlice::with_len_cap(0, incidents.len())
        .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
    let mut used = vec![false; abductions.len()];
    for edge in incidents.iter_slots() {
        step(owner, guard)?;
        let endpoints = edge
            .map(|edge| {
                Ok::<_, OwnerTopologyError>((
                    owner.owner_edge_endpoint(edge, false)?,
                    owner.owner_edge_endpoint(edge, true)?,
                ))
            })
            .transpose()?;
        let Some((from, to)) =
            endpoints.filter(|(from, to)| *from == Some(node) || *to == Some(node))
        else {
            if owner.topology_source_input_node(node).is_none() {
                return Err(OwnerPlacementError::UnboundDebugId(node));
            }
            let name = &owner.topology_node(node).unwrap().external_id;
            return Err(invalid(
                guard,
                &format!("found a malformed incident edge on {name}"),
            ));
        };
        let adjacent = if from == Some(node) { to } else { from };
        if !positioned(owner, adjacent)? {
            continue;
        }
        let adjacent = adjacent.unwrap();
        let mut add_node = adjacent;
        for (index, record) in abductions.iter_slots().enumerate() {
            step(owner, guard)?;
            let Some(record) = record else {
                return Err(invalid(guard, "found a nil edge abduction"));
            };
            if used[index] {
                continue;
            }
            let fields = owner
                .owner_edge_abduction(record)
                .ok_or(OwnerTopologyError::UnknownRecord(record))?;
            if fields.current_from == Some(node)
                && fields.current_to == Some(adjacent)
                && fields.originally_to.is_some()
            {
                used[index] = true;
                add_node = fields.originally_to.unwrap();
                break;
            }
            if fields.current_from == Some(adjacent)
                && fields.current_to == Some(node)
                && fields.originally_from.is_some()
            {
                used[index] = true;
                add_node = fields.originally_from.unwrap();
                break;
            }
        }
        adjacents = append(adjacents, add_node)?;
    }
    if !adjacents.is_empty() {
        return Ok(adjacents);
    }

    let ordered = optimizer_owner_ordered_nears(owner, Some(node), guard)?;
    for near in ordered.iter_slots() {
        let near = project_near(owner, node, near, guard)?;
        if !positioned(owner, near)? {
            continue;
        }
        let container = owner.topology_node(node).unwrap().container;
        let descendant = optimizer_owner_is_descendant_of(owner, near, container, guard).map_err(
            |e| match e {
                OwnerNearEligibilityError::Optimization(e) => OwnerPlacementError::Optimization(e),
                OwnerNearEligibilityError::Topology(e) => OwnerPlacementError::Topology(e),
                OwnerNearEligibilityError::UnboundDebugId(node) => {
                    OwnerPlacementError::UnboundDebugId(node)
                }
            },
        )?;
        if descendant {
            adjacents = append(adjacents, near.unwrap())?;
        }
    }
    if !adjacents.is_empty() {
        return Ok(adjacents);
    }

    let mut nears = SharedNodeSlice::nil();
    if owner.owner_is_cluster_vessel(node) {
        let cluster = owner
            .owner_graph_group_entry(live_graph(owner, node), OwnerGroupKind::Cluster, Some(node))?
            .flatten();
        let Some(cluster) = cluster else {
            return Err(invalid(guard, "found a cluster vessel without a cluster"));
        };
        let members = owner.owner_group_fields(cluster)?.nodes;
        for member in members.iter_slots() {
            step(owner, guard)?;
            let ordered = optimizer_owner_ordered_nears(owner, member, guard)?;
            nears = append_bounded(nears, &ordered, guard)?;
        }
    }
    if let Some(sequence) = owner
        .owner_graph_group_entry(
            live_graph(owner, node),
            OwnerGroupKind::Sequence,
            Some(node),
        )?
        .flatten()
    {
        let members = owner.owner_group_fields(sequence)?.nodes;
        for member in members.iter_slots() {
            step(owner, guard)?;
            let ordered = optimizer_owner_ordered_nears(owner, member, guard)?;
            nears = append_bounded(nears, &ordered, guard)?;
        }
    }
    let roots = owner.owner_graph_tree_roots_entry(live_graph(owner, node), Some(node))?;
    if roots.as_ref().is_some_and(|roots| roots.len() > 10_000) {
        return Err(invalid(guard, "tree roots exceed node limit 10000"));
    }
    if let Some(roots) = roots {
        for index in 0..roots.len() {
            let tree = roots.get(index).expect("retained tree-root slot");
            nears = append_owner_tree_nears(owner, nears, tree, guard)?;
        }
    }
    for near in nears.iter_slots() {
        step(owner, guard)?;
        let projected = project_near(owner, node, near, guard)?;
        if positioned(owner, projected)? {
            adjacents = append(adjacents, projected.unwrap())?;
        }
    }
    Ok(adjacents)
}

#[cfg(test)]
#[path = "owner_adjacents_tests.rs"]
mod tests;
