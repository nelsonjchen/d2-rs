// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! sized_optimizer.go54–103: retain the original Node.Edges header, remove
//! one original edge per spoke, and restore the header on every exit.
use super::container_slices::SharedNodeSlice;
use super::limits::{OptimizationError, OptimizationWorkGuard};
use super::owner_score_session::OwnerScoreSession;
use super::{ArenaGraph, NodeId};

pub(super) trait HubOwner {
    fn hub_owner(&self) -> &ArenaGraph;
    fn hub_owner_mut(&mut self) -> &mut ArenaGraph;
}
impl HubOwner for ArenaGraph {
    fn hub_owner(&self) -> &ArenaGraph {
        self
    }
    fn hub_owner_mut(&mut self) -> &mut ArenaGraph {
        self
    }
}
impl HubOwner for OwnerScoreSession<'_> {
    fn hub_owner(&self) -> &ArenaGraph {
        self.owner()
    }
    fn hub_owner_mut(&mut self) -> &mut ArenaGraph {
        self.owner_mut()
    }
}
fn invalid(message: &str) -> OptimizationError {
    OptimizationError::Invalid(format!("TALA LocalOptimize {message}"))
}
fn topology(error: impl std::fmt::Display) -> OptimizationError {
    OptimizationError::Invalid(error.to_string())
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

pub(super) fn with_hub_spokes_suppressed<T: HubOwner, R>(
    context: &mut T,
    node: Option<NodeId>,
    spokes: &SharedNodeSlice,
    guard: &mut OptimizationWorkGuard<'_>,
    body: impl FnOnce(&mut T, &mut OptimizationWorkGuard<'_>) -> Result<R, OptimizationError>,
) -> Result<R, OptimizationError> {
    let node = node.ok_or_else(|| invalid("cannot suppress spokes on a nil hub"))?;
    let original = context
        .hub_owner()
        .owner_node_incident_header(node)
        .map_err(topology)?;
    if spokes.len() > 10_000 || original.len() > 50_000 {
        return Err(invalid("hub suppression inputs exceed engine limits"));
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut removed = vec![false; original.len()];
        for spoke in spokes.iter_slots() {
            step(context.hub_owner_mut(), guard)?;
            let spoke = spoke.ok_or_else(|| invalid("found a nil hub spoke"))?;
            for index in 0..original.len() {
                // Source range captures the slot before its Step callback.
                let edge = original.get(index).expect("retained hub incident slot");
                step(context.hub_owner_mut(), guard)?;
                if removed[index] {
                    continue;
                }
                let edge = edge.ok_or_else(|| invalid("found a malformed hub edge"))?;
                let owner = context.hub_owner();
                let from = owner.owner_edge_endpoint(edge, false).map_err(topology)?;
                let to = owner.owner_edge_endpoint(edge, true).map_err(topology)?;
                if from != Some(node) && to != Some(node) {
                    return Err(invalid("found a malformed hub edge"));
                }
                if (if from == Some(node) { to } else { from }) == Some(spoke) {
                    removed[index] = true;
                    break;
                }
            }
        }
        let mut working = Vec::new();
        for index in 0..original.len() {
            let edge = original.get(index).expect("retained hub incident slot");
            step(context.hub_owner_mut(), guard)?;
            if !removed[index] {
                working.push(edge);
            }
        }
        let working = context
            .hub_owner_mut()
            .allocate_owner_incident_header(&working, original.len(), true)
            .map_err(topology)?;
        context
            .hub_owner_mut()
            .set_owner_node_incident_header(node, working)
            .map_err(topology)?;
        body(context, guard)
    }));
    context
        .hub_owner_mut()
        .set_owner_node_incident_header(node, original)
        .expect("retained hub incident header restores in its owning arena");
    match result {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
