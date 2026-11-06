// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! sizeless_optimizer.go478–544: owner score/selection/shared-RNG
//! ordering and speculative swaps, with exclusive callback dispatch between borrowed scores.
use super::limits::{self, Context, OptimizationError, OptimizationWorkGuard};
use super::owner_placement::OwnerPlacementError;
use super::owner_sizeless_constructor::{OwnerOptimizerRng, OwnerSizelessOptimizer};
use super::scoring::OwnerSizelessEdgeLengthScorer;
use super::scoring_context::ScoreContext;
use super::sizeless::precision_compare;
use super::{ArenaGraph, CanonicalScoreMapping, NodeId, OwnerGraphId};

#[cfg(test)]
#[path = "owner_best_swap_tests.rs"]
mod tests;

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
fn score_error(error: limits::Error) -> OwnerPlacementError {
    match error {
        limits::Error::Context { location, cause } => {
            OptimizationError::Context { location, cause }.into()
        }
        error => OptimizationError::Invalid(error.to_string()).into(),
    }
}
fn motion_error(error: OwnerPlacementError) -> OptimizationError {
    match error {
        OwnerPlacementError::Optimization(error) => error,
        error => OptimizationError::Invalid(error.to_string()),
    }
}

impl OwnerOptimizerRng {
    /// Borrow the caller stream only for each draw. Source ctx.Err may itself
    /// draw from that same rand.Rand before the charged shuffle draw begins.
    pub(super) fn shuffle_owner<T>(
        &self,
        values: &mut [T],
        owner: &mut ArenaGraph,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<(), OptimizationError> {
        #[cfg(feature = "diagnostic-traces")]
        self.stream().borrow().trace_operation(
            "shuffle-start",
            || serde_json::json!({"length":values.len()}),
        );
        let result = (|| {
            let mut index = values.len().saturating_sub(1);
            while index > i32::MAX as usize - 1 {
                step(owner, guard)?;
                let target = self.stream().borrow_mut().int63n(index as i64 + 1) as usize;
                #[cfg(feature = "diagnostic-traces")]
                self.stream().borrow().trace_operation(
                    "shuffle-swap",
                    || serde_json::json!({"index":index,"target":target}),
                );
                values.swap(index, target);
                index -= 1;
            }
            while index > 0 {
                let n = (index + 1) as u32;
                step(owner, guard)?;
                let mut product = u64::from(self.stream().borrow_mut().uint32()) * u64::from(n);
                let mut low = product as u32;
                if low < n {
                    let threshold = 0u32.wrapping_sub(n) % n;
                    while low < threshold {
                        step(owner, guard)?;
                        product = u64::from(self.stream().borrow_mut().uint32()) * u64::from(n);
                        low = product as u32;
                    }
                }
                let target = (product >> 32) as usize;
                #[cfg(feature = "diagnostic-traces")]
                self.stream().borrow().trace_operation(
                    "shuffle-swap",
                    || serde_json::json!({"index":index,"target":target}),
                );
                values.swap(index, target);
                index -= 1;
            }
            Ok(())
        })();
        #[cfg(feature = "diagnostic-traces")]
        self.stream().borrow().trace_operation(
            "shuffle-end",
            || serde_json::json!({"length":values.len(),"completed":result.is_ok()}),
        );
        result
    }
}

fn charge(
    owner: &mut ArenaGraph,
    node: Option<NodeId>,
    guard: &mut OptimizationWorkGuard<'_>,
) -> Result<NodeId, OptimizationError> {
    let missing = || {
        OptimizationError::Invalid(format!(
            "TALA {} scoring requires a node with a graph",
            guard.location()
        ))
    };
    let node = node.ok_or_else(missing)?;
    let graph = owner.topology_node_owner(node).ok_or_else(missing)?;
    let nodes = owner
        .owner_graph_view(graph)
        .ok_or_else(missing)?
        .nodes
        .len() as u64
        + 1;
    let edges = owner.owner_node_incident_count(node) as u64 + 1;
    #[cfg(feature = "diagnostic-traces")]
    guard.add_product_with_owner(edges, nodes, owner)?;
    #[cfg(not(feature = "diagnostic-traces"))]
    guard.add_product(edges, nodes)?;
    Ok(node)
}

fn score(
    workspace: &ArenaGraph,
    owner: &mut ArenaGraph,
    mapping: &CanonicalScoreMapping,
    view: OwnerGraphId,
    node: NodeId,
    context: &dyn Context,
) -> Result<f64, OwnerPlacementError> {
    let local = mapping
        .nodes()
        .iter()
        .position(|&canonical| canonical == node)
        .map(|index| NodeId(index as u32))
        .ok_or_else(|| {
            OptimizationError::Invalid(format!(
                "owner swap scorer has no canonical mapping for {node:?}"
            ))
        })?;
    // bestSwapCandidateGuarded invokes fresh NodeEdgeLength, rather than
    // retaining the candidate sweep's prepared scorer across a swap.
    let mut scorer = OwnerSizelessEdgeLengthScorer::new(local, true);
    // This scorer is closed after this one measurement. Cancellation-only
    // polls need initial admission, but no snapshot for a subsequent score
    // (sizeless_optimizer.go:484-524, edge_length.go:145-335).
    if context.stable_sized_owner() {
        scorer.assume_stable_topology();
    }
    let result =
        scorer.score_with_owner(workspace, owner, mapping, view, ScoreContext::new(context));
    scorer.close();
    result.map_err(score_error)
}

impl OwnerSizelessOptimizer {
    pub(super) fn best_swap_candidate_guarded(
        &self,
        workspace: &ArenaGraph,
        owner: &mut ArenaGraph,
        mapping: &CanonicalScoreMapping,
        node: Option<NodeId>,
        context: &dyn Context,
        guard: &mut OptimizationWorkGuard<'_>,
    ) -> Result<Option<NodeId>, OwnerPlacementError> {
        let node = charge(owner, node, guard)?;
        let mut minimum = score(
            workspace,
            owner,
            mapping,
            self.selected_graph(),
            node,
            context,
        )?;
        let mut best = None;
        let candidates = self.swap_candidates_guarded(owner, Some(node), guard)?;
        let mut candidates = candidates.require_nodes().collect::<Vec<_>>();
        let rng = self.rng().ok_or_else(|| {
            OptimizationError::Invalid(format!(
                "TALA {} shuffle requires a random generator",
                guard.location()
            ))
        })?;
        rng.shuffle_owner(&mut candidates, owner, guard)?;
        for candidate in candidates {
            step(owner, guard)?;
            charge(owner, Some(candidate), guard)?;
            let current = score(
                workspace,
                owner,
                mapping,
                self.selected_graph(),
                candidate,
                context,
            )?;
            let (first, second) = self.with_swap_trial_guarded(
                owner,
                Some(node),
                Some(candidate),
                guard,
                |owner, guard| {
                    charge(owner, Some(node), guard)?;
                    let first = score(
                        workspace,
                        owner,
                        mapping,
                        self.selected_graph(),
                        node,
                        context,
                    )
                    .map_err(motion_error)?;
                    let mut second = 0.0;
                    if precision_compare(first, minimum).is_lt() {
                        charge(owner, Some(candidate), guard)?;
                        second = score(
                            workspace,
                            owner,
                            mapping,
                            self.selected_graph(),
                            candidate,
                            context,
                        )
                        .map_err(motion_error)?;
                    }
                    Ok((first, second))
                },
            )?;
            if precision_compare(first, minimum).is_lt()
                && !precision_compare(second, current).is_gt()
            {
                minimum = first;
                best = Some(candidate);
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        let cause = context.error_with_owner(owner);
        #[cfg(not(feature = "diagnostic-traces"))]
        let cause = context.error();
        if let Some(cause) = cause {
            return Err(OptimizationError::Context {
                location: "EdgeLength".into(),
                cause,
            }
            .into());
        }
        Ok(best)
    }
}
