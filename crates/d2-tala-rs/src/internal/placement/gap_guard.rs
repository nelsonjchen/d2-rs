// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Shared accounting at NormalizeGaps' first placement-score boundary.
//!
//! D2 v0.9.0 `layoutgraph.Transaction.CapturePlacementCosts` attaches one
//! stage-owned snapshot before the first actual score. Geometry-only candidate
//! rollback deliberately keeps memoized costs. The enclosing failed stage is
//! responsible for consuming and restoring this optional snapshot.

use crate::internal::limits::{Error, WorkGuard};
use crate::internal::{ArenaGraph, GeometryCheckpoint, NodeId, PlacementCostSnapshot, limits};
use std::collections::BTreeSet;
use std::sync::Arc;

pub(super) const MAX_TRANSACTION_WORK_UNITS: i64 = 1_000_000_000;

pub(super) struct ActiveContext {
    cancellation: Option<crate::CancellationToken>,
}

impl ActiveContext {
    pub(super) fn new() -> Self {
        Self {
            cancellation: crate::internal::current_cancellation(),
        }
    }
}

impl limits::Context for ActiveContext {
    fn done(&self) -> Option<bool> {
        self.cancellation
            .as_ref()
            .map(crate::CancellationToken::is_cancelled)
    }

    fn error(&self) -> Option<limits::ContextError> {
        self.done()
            .filter(|cancelled| *cancelled)
            .map(|_| limits::ContextError::Cancelled)
    }

    fn shared_scoring_context(&self) -> Option<&(dyn limits::Context + Sync)> {
        Some(self)
    }
}

struct BackgroundContext;

impl limits::Context for BackgroundContext {
    fn done(&self) -> Option<bool> {
        None
    }

    fn error(&self) -> Option<limits::ContextError> {
        None
    }

    fn shared_scoring_context(&self) -> Option<&(dyn limits::Context + Sync)> {
        Some(self)
    }
}

/// The stage's request transaction owns one aggregate guard and one lazy cost
/// journal. Nested geometry transactions share both; ordinary rejection never
/// restores the journal. The stage restores it only on an error or unwind.
#[derive(Clone)]
pub(super) struct AnchoredGapOverlaps {
    pairs: Arc<BTreeSet<(NodeId, NodeId)>>,
    exact: Arc<BTreeSet<(NodeId, NodeId)>>,
}
impl AnchoredGapOverlaps {
    pub(super) fn pairs(&self) -> &BTreeSet<(NodeId, NodeId)> {
        &self.pairs
    }
    pub(super) fn exact_pairs(&self) -> &BTreeSet<(NodeId, NodeId)> {
        &self.exact
    }
    #[cfg(test)]
    pub(super) fn shares_sets(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.pairs, &other.pairs) && Arc::ptr_eq(&self.exact, &other.exact)
    }
}

pub(super) struct GapWork<'guard, 'context> {
    pub(super) guard: &'guard mut WorkGuard<'context>,
    pub(super) operation_context: &'guard dyn limits::Context,
    placement_costs: Option<PlacementCostSnapshot>,
    capture_costs: bool,
    anchored_overlaps: Option<AnchoredGapOverlaps>,
    new_request_pending: bool,
}

impl<'guard, 'context> GapWork<'guard, 'context> {
    pub(super) fn new(
        guard: &'guard mut WorkGuard<'context>,
        operation_context: &'guard dyn limits::Context,
        capture_costs: bool,
    ) -> Self {
        Self {
            guard,
            operation_context,
            placement_costs: None,
            capture_costs,
            anchored_overlaps: None,
            new_request_pending: false,
        }
    }

    pub(super) fn begin_unbound_request(&mut self) {
        self.new_request_pending = true;
    }
    /// A nil gap-reducer transaction is constructed only at its first actual
    /// candidate. Existing callers retain their original overlap exceptions.
    pub(super) fn refresh_geometry_request(
        &mut self,
        graph: &mut ArenaGraph,
        checkpoint: &mut GeometryCheckpoint,
    ) -> Result<(), Error> {
        if !self.new_request_pending {
            return checkpoint.update(graph, self.guard);
        }
        let captured = GeometryCheckpoint::capture_request(graph, self.guard)?;
        let (pairs, exact) = graph.transaction_overlap_snapshot_checked(self.guard)?;
        self.guard.finish()?;
        self.anchored_overlaps = None;
        self.anchor_canonical_overlaps(pairs, exact);
        *checkpoint = captured;
        self.new_request_pending = false;
        Ok(())
    }

    /// Source construction owns both exception maps. Geometry UpdateState
    /// and nested clones must never reinterpret their canonical endpoints.
    pub(super) fn anchor_canonical_overlaps(
        &mut self,
        pairs: BTreeSet<(NodeId, NodeId)>,
        exact: BTreeSet<(NodeId, NodeId)>,
    ) {
        if self.anchored_overlaps.is_none() {
            self.anchored_overlaps = Some(AnchoredGapOverlaps {
                pairs: Arc::new(pairs),
                exact: Arc::new(exact),
            });
        }
    }
    pub(super) fn anchor_numeric_overlaps(
        &mut self,
        graph: &ArenaGraph,
        pairs: &BTreeSet<(NodeId, NodeId)>,
        exact: &BTreeSet<(NodeId, NodeId)>,
    ) {
        if self.anchored_overlaps.is_none() {
            self.anchor_canonical_overlaps(
                graph.canonical_transaction_overlap_pairs(pairs),
                graph.canonical_transaction_overlap_pairs(exact),
            );
        }
    }
    pub(super) fn overlap_snapshot(&self) -> Option<AnchoredGapOverlaps> {
        self.anchored_overlaps.clone()
    }
    pub(super) fn refit_containers(
        &mut self,
        graph: &mut ArenaGraph,
        checkpoint: &GeometryCheckpoint,
    ) -> Result<bool, Error> {
        let overlaps = self
            .anchored_overlaps
            .as_ref()
            .expect("geometry transaction overlap construction")
            .clone();
        graph.transaction_refit_containers_checked(
            checkpoint,
            overlaps.pairs(),
            self.operation_context,
            self.guard,
        )
    }

    pub(super) fn validate_commit(
        &mut self,
        graph: &ArenaGraph,
        checkpoint: &GeometryCheckpoint,
    ) -> Result<bool, Error> {
        let overlaps = self
            .anchored_overlaps
            .as_ref()
            .expect("geometry transaction overlap construction")
            .clone();
        checkpoint.validate_commit(
            graph,
            overlaps.pairs(),
            overlaps.exact_pairs(),
            self.operation_context,
            self.guard,
        )
    }

    pub(super) fn before_score(&mut self, graph: &ArenaGraph) -> Result<(), Error> {
        #[cfg(feature = "diagnostic-traces")]
        self.observe(graph, limits::TransactionPollSite::General);
        if self.capture_costs {
            capture_placement_costs_once(
                &mut self.placement_costs,
                "GapNormalizationTransactions",
                self.guard,
                || graph.cost_cache.edge_length_entries(),
                || graph.snapshot_placement_costs(),
            )?;
        }
        Ok(())
    }

    pub(super) fn score(&mut self, graph: &ArenaGraph) -> Result<f64, Error> {
        self.before_score(graph)?;
        #[cfg(feature = "diagnostic-traces")]
        self.operation_context
            .transaction_work_used(self.guard.used());
        graph.global_sized_edge_length_after_tree_restoration_checked(false, self.operation_context)
    }

    pub(super) fn before_operation(&mut self, _graph: &ArenaGraph) -> Result<(), Error> {
        #[cfg(feature = "diagnostic-traces")]
        self.observe(_graph, limits::TransactionPollSite::General);
        self.guard.finish()?;
        self.guard.check_context_with(self.operation_context)?;
        self.guard.step()?;
        self.guard.check_context_with(self.operation_context)
    }

    pub(super) fn check_context(&self, _graph: &ArenaGraph) -> Result<(), Error> {
        #[cfg(feature = "diagnostic-traces")]
        self.observe(_graph, limits::TransactionPollSite::General);
        self.guard.check_context_with(self.operation_context)
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe(&self, graph: &ArenaGraph, site: limits::TransactionPollSite) {
        self.guard.observe_transaction_graph(graph, site);
        self.operation_context.transaction_state(graph, site);
    }

    pub(super) fn restore_costs(&self, graph: &ArenaGraph, routing: (f64, f64, f64)) {
        if let Some(snapshot) = &self.placement_costs {
            snapshot.restore();
        } else {
            graph.restore_routing_costs(routing);
        }
    }
}

/// Legacy graph-level helpers are unbounded background operations, without
/// an enclosing stage cost journal. Production NormalizeGaps uses GapWork
/// directly, so errors are propagated and never converted into trial rejection.
pub(super) fn with_background_work<T>(
    operation: impl FnOnce(&mut GapWork<'_, '_>) -> Result<T, Error>,
) -> T {
    let context = BackgroundContext;
    let mut guard = WorkGuard::new(
        Some(&context),
        "GapNormalizationTransactions",
        MAX_TRANSACTION_WORK_UNITS,
    )
    .expect("a background transaction guard is valid");
    operation(&mut GapWork::new(&mut guard, &context, false))
        .expect("background gap transaction could not complete")
}

/// `layoutgraph.maxEngineTopologyReferences` bounds the graph-owned cache copy.
const MAX_PLACEMENT_CACHE_ENTRIES: usize = 1_000_000;

/// Capture the enclosing stage's cost rollback point exactly once.
///
/// Entry counting and snapshot copying are closures so an already attached
/// snapshot performs neither operation. Charge each existing map entry using
/// the caller's aggregate transaction guard, then copy, poll cancellation, and
/// only then publish the attachment. This preserves the original source's
/// failure boundary and does not reset the caller's consumed work.
///
/// `capture` must copy raw cache state; it must not invoke semantic cost
/// getters. This function does not restore anything on ordinary trial
/// rejection and does not charge speculative scores or geometry work.
pub(super) fn capture_placement_costs_once<S>(
    slot: &mut Option<S>,
    location: &str,
    guard: &mut WorkGuard<'_>,
    entries: impl FnOnce() -> usize,
    capture: impl FnOnce() -> S,
) -> Result<(), Error> {
    if slot.is_some() {
        return Ok(());
    }
    let entries = entries();
    if entries > MAX_PLACEMENT_CACHE_ENTRIES {
        return Err(Error::Invalid(format!(
            "TALA {location} edge-length cache entries exceed limit {MAX_PLACEMENT_CACHE_ENTRIES}"
        )));
    }
    for _ in 0..entries {
        guard.step()?;
    }
    let snapshot = capture();
    guard.finish()?;
    *slot = Some(snapshot);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::limits::{Context, ContextError};
    use std::cell::Cell;

    #[test]
    fn active_context_keeps_the_callers_token_on_another_thread() {
        let token = crate::CancellationToken::new();
        let context = crate::internal::with_cancellation(Some(&token), ActiveContext::new);
        assert_eq!(context.done(), Some(false));
        token.cancel();
        std::thread::scope(|scope| {
            let shared = context.shared_scoring_context().unwrap();
            scope
                .spawn(move || {
                    assert_eq!(crate::internal::cancellation_done(), None);
                    assert_eq!(shared.done(), Some(true));
                    assert_eq!(shared.error(), Some(ContextError::Cancelled));
                })
                .join()
                .unwrap();
        });
    }

    struct TestContext {
        cancelled: Cell<bool>,
    }

    impl Context for TestContext {
        fn done(&self) -> Option<bool> {
            None
        }

        fn error(&self) -> Option<ContextError> {
            self.cancelled.get().then_some(ContextError::Cancelled)
        }
    }

    #[test]
    fn first_capture_charges_entries_once_on_the_existing_guard() {
        let context = TestContext {
            cancelled: Cell::new(false),
        };
        let mut guard =
            WorkGuard::new(Some(&context), "GapNormalizationTransactions", 100).unwrap();
        guard.step().unwrap();
        let mut slot = None;
        capture_placement_costs_once(
            &mut slot,
            "GapNormalizationTransactions",
            &mut guard,
            || 3,
            || 17,
        )
        .unwrap();
        assert_eq!(guard.used(), 4);
        assert_eq!(slot, Some(17));
        context.cancelled.set(true);
        capture_placement_costs_once(
            &mut slot,
            "GapNormalizationTransactions",
            &mut guard,
            || panic!("an attached stage snapshot must not recount the map"),
            || panic!("an attached stage snapshot must not recopy the map"),
        )
        .unwrap();
        assert_eq!(guard.used(), 4);
        assert_eq!(slot, Some(17));
    }

    #[test]
    fn work_failure_preserves_first_rejected_unit_and_leaves_no_attachment() {
        let context = TestContext {
            cancelled: Cell::new(false),
        };
        let mut guard = WorkGuard::new(Some(&context), "GapNormalizationTransactions", 2).unwrap();
        let mut slot: Option<()> = None;
        let error = capture_placement_costs_once(
            &mut slot,
            "GapNormalizationTransactions",
            &mut guard,
            || 3,
            || panic!("work failure must precede snapshot copying"),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "TALA GapNormalizationTransactions work exceeds limit 2"
        );
        assert_eq!(guard.used(), 3);
        assert!(slot.is_none());
    }

    #[test]
    fn cancellation_after_copy_prevents_publishing_the_attachment() {
        let context = TestContext {
            cancelled: Cell::new(false),
        };
        let mut guard =
            WorkGuard::new(Some(&context), "GapNormalizationTransactions", 100).unwrap();
        let mut slot = None;
        let copied = Cell::new(false);
        let error = capture_placement_costs_once(
            &mut slot,
            "GapNormalizationTransactions",
            &mut guard,
            || 1,
            || {
                copied.set(true);
                context.cancelled.set(true);
                17
            },
        )
        .unwrap_err();
        assert!(copied.get());
        assert!(slot.is_none());
        assert_eq!(guard.used(), 1);
        assert_eq!(
            error.to_string(),
            "GapNormalizationTransactions: context canceled"
        );
    }

    #[test]
    fn topology_limit_precedes_charging_or_copying() {
        let context = TestContext {
            cancelled: Cell::new(false),
        };
        let mut guard =
            WorkGuard::new(Some(&context), "GapNormalizationTransactions", 100).unwrap();
        let mut slot: Option<()> = None;
        let error = capture_placement_costs_once(
            &mut slot,
            "GapNormalizationTransactions",
            &mut guard,
            || MAX_PLACEMENT_CACHE_ENTRIES + 1,
            || panic!("topology limit must precede snapshot copying"),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "TALA GapNormalizationTransactions edge-length cache entries exceed limit 1000000"
        );
        assert_eq!(guard.used(), 0);
        assert!(slot.is_none());
    }
}
