// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! sized_optimizer.go109–231: canonical sized constructor validation and setup.
use crate::internal::limits::{
    Context, MAX_OPTIMIZATION_WORK_UNITS, OptimizationError, OptimizationWorkGuard,
};
use crate::internal::node_requirements_map::NodeRequirementsMap;
use crate::internal::shared_topology::OwnerAbductionHeader;
use crate::internal::{ArenaGraph, NodeId, OwnerGraphId, Point};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug)]
pub(super) struct Setup {
    pub(super) cell: f64,
    pub(super) origin: Option<Point>,
}
fn error(message: impl Into<String>) -> OptimizationError {
    OptimizationError::Invalid(message.into())
}
fn step(
    owner: &mut ArenaGraph,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<(), OptimizationError> {
    #[cfg(feature = "diagnostic-traces")]
    {
        guard.step_with_owner(owner)
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        let _ = owner;
        guard.step()
    }
}
fn name(owner: &ArenaGraph, node: Option<NodeId>) -> Result<String, OptimizationError> {
    crate::internal::optimizer_spatial_index::owner_search_debug_id(owner, node)
}
pub(super) fn validate(
    owner: &ArenaGraph,
    receiver: Option<OwnerGraphId>,
    abductions: &OwnerAbductionHeader,
    obstacles: usize,
) -> Result<OwnerGraphId, OptimizationError> {
    let receiver = receiver.ok_or_else(|| error("TALA LocalOptimize requires a graph"))?;
    let graph = owner
        .owner_graph_view(receiver)
        .ok_or_else(|| error("TALA LocalOptimize requires a graph"))?;
    if graph.nodes.len() > 10000 {
        return Err(error("TALA LocalOptimize node count exceeds limit 10000"));
    }
    if graph.edges.len() > 50000 {
        return Err(error("TALA LocalOptimize edge count exceeds limit 50000"));
    }
    if abductions.len() > 50000 {
        return Err(error(
            "TALA LocalOptimize edge abduction count exceeds limit 50000",
        ));
    }
    if obstacles > 1_000_000 {
        return Err(error(
            "TALA LocalOptimize obstacle count exceeds limit 1000000",
        ));
    }
    let cell = owner
        .owner_graph_cell_size(receiver)
        .map_err(|e| error(e.to_string()))?;
    if cell < 1. || !cell.is_finite() || cell.trunc() != cell {
        return Err(error(
            "TALA LocalOptimize requires a finite positive integer cell size",
        ));
    }
    Ok(receiver)
}
pub(super) fn guarded(
    owner: &mut ArenaGraph,
    receiver: OwnerGraphId,
    root: Option<NodeId>,
    abductions: &OwnerAbductionHeader,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<Setup, OptimizationError> {
    let members = owner
        .owner_graph_member_header(receiver)
        .map_err(|e| error(e.to_string()))?;
    for node in members.iter_slots() {
        step(owner, guard)?;
        let Some(node) = node else {
            return Err(error("TALA LocalOptimize found a node without a graph"));
        };
        let Some(graph) = owner.topology_node_owner(node) else {
            return Err(error("TALA LocalOptimize found a node without a graph"));
        };
        if owner
            .owner_node_incident_header(node)
            .map_err(|e| error(e.to_string()))?
            .len()
            > 50000
            || owner
                .owner_node_near_map(node)
                .map_err(|e| error(e.to_string()))?
                .len()
                > 10000
        {
            return Err(error(format!(
                "TALA LocalOptimize node {} adjacency references exceed engine limits",
                name(owner, Some(node))?
            )));
        }
        if owner
            .owner_graph_cell_size(graph)
            .map_err(|e| error(e.to_string()))?
            != owner
                .owner_graph_cell_size(receiver)
                .map_err(|e| error(e.to_string()))?
        {
            return Err(error(format!(
                "cell size mismatch for node {}",
                name(owner, Some(node))?
            )));
        }
    }
    let cell = owner
        .owner_graph_cell_size(receiver)
        .map_err(|e| error(e.to_string()))?;
    let mut origin = None;
    let members = owner
        .owner_graph_member_header(receiver)
        .map_err(|e| error(e.to_string()))?;
    for node in members.iter_slots() {
        let node = node.expect("runtime error: invalid memory address or nil pointer dereference");
        if crate::internal::owner_placement::owner_owning_container(owner, node)
            .map_err(|e| error(e.to_string()))?
            != root
        {
            continue;
        }
        if let (Some(position), Some(fixed)) = (
            owner.position(node),
            owner.nodes[node.0 as usize].fixed_top_left,
        ) {
            origin = Some(Point {
                x: position.x - fixed.x,
                y: position.y - fixed.y,
            });
            break;
        }
    }
    let members = owner
        .owner_graph_member_header(receiver)
        .map_err(|e| error(e.to_string()))?;
    for node in members.iter_slots() {
        step(owner, guard)?;
        let node = node.expect("runtime error: invalid memory address or nil pointer dereference");
        let mut skipped = BTreeSet::new();
        let mut replacements = BTreeMap::new();
        for record in abductions.iter_slots() {
            step(owner, guard)?;
            let fields = record.and_then(|r| owner.owner_edge_abduction(r)).copied();
            let Some(fields) = fields.filter(|r| r.source_edge.is_some()) else {
                return Err(error("TALA LocalOptimize found a nil edge abduction"));
            };
            if fields.current_from == Some(node) || fields.current_to == Some(node) {
                if fields.originally_from.is_some() || fields.originally_to.is_some() {
                    skipped.insert(fields.source_edge.unwrap());
                    continue;
                }
                let edge = fields.source_edge.unwrap();
                let from = owner
                    .owner_edge_endpoint(edge, false)
                    .map_err(|e| error(e.to_string()))?;
                let to = owner
                    .owner_edge_endpoint(edge, true)
                    .map_err(|e| error(e.to_string()))?;
                let adjacent = if from == Some(node) { to } else { from };
                if fields.current_from == Some(node) && fields.originally_to.is_some() {
                    replacements.insert(adjacent, fields.originally_to);
                } else if fields.current_to == Some(node) && fields.originally_from.is_some() {
                    replacements.insert(adjacent, fields.originally_from);
                }
            }
        }
        let requirements = NodeRequirementsMap::allocated();
        let incidents = owner
            .owner_node_incident_header(node)
            .map_err(|e| error(e.to_string()))?;
        for edge in incidents.iter_slots() {
            step(owner, guard)?;
            let endpoints = edge
                .map(|edge| {
                    Ok::<_, OptimizationError>((
                        owner
                            .owner_edge_endpoint(edge, false)
                            .map_err(|e| error(e.to_string()))?,
                        owner
                            .owner_edge_endpoint(edge, true)
                            .map_err(|e| error(e.to_string()))?,
                    ))
                })
                .transpose()?;
            let Some((from, to)) =
                endpoints.filter(|(from, to)| *from == Some(node) || *to == Some(node))
            else {
                return Err(error(format!(
                    "TALA LocalOptimize found a malformed incident edge on {}",
                    name(owner, Some(node))?
                )));
            };
            let edge = edge.unwrap();
            if skipped.contains(&edge) {
                continue;
            }
            let mut adjacent = if from == Some(node) { to } else { from };
            if let Some(replacement) = replacements.get(&adjacent) {
                adjacent = *replacement;
            }
            let mut requirement = requirements.get(adjacent);
            requirement.edge_count = requirement.edge_count.wrapping_add(1);
            let edge = &owner.edge_records[edge.0 as usize];
            let width = crate::internal::loop_offsets::loop_spacing_units(edge.min_width);
            let height = crate::internal::loop_offsets::loop_spacing_units(edge.min_height);
            if width > requirement.max_width {
                requirement.max_width = width;
            }
            if height > requirement.max_height {
                requirement.max_height = height;
            }
            requirements.set(adjacent, requirement).unwrap();
        }
        let cursor = requirements.live_cursor();
        for key in cursor {
            let requirement = requirements.get(key);
            step(owner, guard)?;
            if requirement.edge_count < 3 {
                continue;
            }
            let cell_int = crate::internal::loop_offsets::loop_spacing_units(cell);
            if requirement.max_width <= cell_int && requirement.max_height <= cell_int {
                continue;
            }
            owner
                .set_owner_node_requirement_map(node, requirements.retained())
                .map_err(|e| error(e.to_string()))?;
            break;
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    guard.finish_with_owner(owner)?;
    #[cfg(not(feature = "diagnostic-traces"))]
    guard.finish()?;
    Ok(Setup { cell, origin })
}
pub(super) fn initialize(
    owner: &mut ArenaGraph,
    receiver: Option<OwnerGraphId>,
    root: Option<NodeId>,
    abductions: &OwnerAbductionHeader,
    obstacles: usize,
    context: &dyn Context,
) -> Result<Setup, OptimizationError> {
    let receiver = validate(owner, receiver, abductions, obstacles)?;
    #[cfg(feature = "diagnostic-traces")]
    let mut guard = OptimizationWorkGuard::new_with_owner(
        Some(context),
        "LocalOptimizeSetup",
        MAX_OPTIMIZATION_WORK_UNITS,
        owner,
    )?;
    #[cfg(not(feature = "diagnostic-traces"))]
    let mut guard = OptimizationWorkGuard::new(
        Some(context),
        "LocalOptimizeSetup",
        MAX_OPTIMIZATION_WORK_UNITS,
    )?;
    guarded(owner, receiver, root, abductions, &mut guard)
}
