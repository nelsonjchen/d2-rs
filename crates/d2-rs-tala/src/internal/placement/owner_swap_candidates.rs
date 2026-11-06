// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! sizeless_optimizer.go554–595: retain the actual TopLeft allocation, check
//! all four cells even after errors, and return the last recorded error.
use super::container_slices::SharedNodeSlice;
use super::limits::OptimizationWorkGuard;
use super::owner_near_eligibility::{OwnerNearEligibilityError, can_optimize_owner_node_guarded};
use super::owner_placement::OwnerPlacementError;
use super::owner_sizeless_constructor::OwnerSizelessOptimizer;
use super::{ArenaGraph, NodeId, OwnerTopologyError, Point};

#[cfg(test)]
#[path = "owner_point_swaps_tests.rs"]
mod tests;
impl OwnerSizelessOptimizer {
    /// The optimizer's committed swap uses the same canonical owner as its
    /// live occupancy and selection. The enclosing optimizer journal remains
    /// responsible for restoring a later failure of the whole iteration.
    pub(super) fn commit_swap_guarded(
        &self,
        owner: &mut ArenaGraph,
        node: Option<NodeId>,
        candidate: Option<NodeId>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), super::limits::OptimizationError> {
        owner.optimizer_swap_positions(node, candidate, guard)
    }

    /// Score callbacks receive the actual exclusive owner after both moves;
    /// all captured positions restore before this call returns or unwinds.
    pub(super) fn with_swap_trial_guarded<R>(
        &self,
        owner: &mut ArenaGraph,
        node: Option<NodeId>,
        candidate: Option<NodeId>,
        guard: &mut OptimizationWorkGuard<'_>,
        score: impl FnOnce(
            &mut ArenaGraph,
            &mut OptimizationWorkGuard<'_>,
        ) -> Result<R, super::limits::OptimizationError>,
    ) -> Result<R, super::limits::OptimizationError> {
        owner.with_optimizer_positions_swapped(node, candidate, guard, score)
    }

    pub(super) fn swap_candidates_guarded(
        &self,
        owner: &mut ArenaGraph,
        node: Option<NodeId>,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<SharedNodeSlice, OwnerPlacementError> {
        let mut adjacent = SharedNodeSlice::with_len_cap(0, 4)
            .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?;
        let node = node.expect("runtime error: invalid memory address or nil pointer dereference");
        let tl = owner.owner_node_point(node)?;
        let mut adjacent_error = None;
        for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
            #[cfg(feature = "diagnostic-traces")]
            let charge = guard.step_with_owner(owner);
            #[cfg(not(feature = "diagnostic-traces"))]
            let charge = guard.step();
            if let Err(error) = charge {
                adjacent_error = Some(OwnerPlacementError::Optimization(error));
                continue;
            }
            let tl = tl
                .get()
                .expect("runtime error: invalid memory address or nil pointer dereference");
            let candidate = self.occupied().borrow().lookup(Point {
                x: tl.x + dx,
                y: tl.y + dy,
            });
            if let Some(candidate) = candidate {
                match can_optimize_owner_node_guarded(
                    owner,
                    candidate,
                    Some(self.selected_graph()),
                    guard,
                ) {
                    Ok(true) => {
                        adjacent = adjacent
                            .append_node(candidate.unwrap())
                            .map_err(|e| OwnerTopologyError::ContainerSlice(e.to_string()))?
                    }
                    Ok(false) => (),
                    Err(error) => {
                        adjacent_error = Some(match error {
                            OwnerNearEligibilityError::Optimization(e) => {
                                OwnerPlacementError::Optimization(e)
                            }
                            OwnerNearEligibilityError::Topology(e) => {
                                OwnerPlacementError::Topology(e)
                            }
                            OwnerNearEligibilityError::UnboundDebugId(n) => {
                                OwnerPlacementError::UnboundDebugId(n)
                            }
                        })
                    }
                }
            }
        }
        if let Some(error) = adjacent_error {
            Err(error)
        } else {
            Ok(adjacent)
        }
    }
}
