// SPDX-License-Identifier: MPL-2.0
//! Geometry-only GraphState sharing from transaction.go:647, 1420.
//! Operations, overlap exceptions and the request guard belong to the caller.
use super::*;

pub(super) struct GeometryCheckpoint {
    prior: Arc<trial_geometry::TrialGeometrySnapshot>,
}
impl GeometryCheckpoint {
    pub(super) fn validate_commit(
        &self,
        graph: &ArenaGraph,
        overlaps: &BTreeSet<(NodeId, NodeId)>,
        exact_overlaps: &BTreeSet<(NodeId, NodeId)>,
        context: &dyn limits::Context,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        self.prior
            .validate_commit(graph, overlaps, exact_overlaps, context, guard)
    }
    /// NewRequestTransaction with an existing aggregate guard first checks
    /// ensureTransactionWorkGuard, then newTransactionWithOptionsContext.
    /// capture() alone corresponds only to GraphState.updateContext.
    pub(super) fn capture_request(
        graph: &ArenaGraph,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Self, limits::Error> {
        guard.finish()?;
        guard.finish()?;
        Self::capture(graph, guard)
    }
    pub(super) fn capture(
        graph: &ArenaGraph,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Self, limits::Error> {
        Ok(Self {
            prior: Arc::new(graph.trial_geometry_snapshot_with_guard(guard)?),
        })
    }
    pub(super) fn from_snapshot(prior: trial_geometry::TrialGeometrySnapshot) -> Self {
        Self {
            prior: Arc::new(prior),
        }
    }
    pub(super) fn preserve(&self) -> Self {
        Self {
            prior: Arc::clone(&self.prior),
        }
    }
    pub(super) fn clone_geometry(
        &self,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Self, limits::Error> {
        guard.finish()?;
        Ok(self.preserve())
    }
    pub(super) fn update(
        &mut self,
        graph: &mut ArenaGraph,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        match graph.trial_geometry_snapshot_with_guard(guard) {
            Ok(prior) => {
                self.prior = Arc::new(prior);
                Ok(())
            }
            Err(error) => {
                self.rollback(graph);
                Err(error)
            }
        }
    }
    pub(super) fn rollback(&self, graph: &mut ArenaGraph) {
        graph.restore_trial_geometry_snapshot((*self.prior).clone());
    }
    pub(super) fn original_size(&self, node: NodeId) -> Option<Size> {
        self.prior.original_size(node)
    }
    pub(super) fn original_position(&self, node: NodeId) -> Option<Point> {
        self.prior.original_position(node)
    }
    #[cfg(test)]
    pub(super) fn shares_prior(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.prior, &other.prior)
    }
}
