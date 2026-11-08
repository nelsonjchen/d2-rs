// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Route-search work accounting translated from D2 v0.9.0.
//!
//! The production router evaluates up to three flavor workers concurrently.
//! Each worker keeps its route-search counter locally and reports exhaustion at
//! the flavor boundary, matching upstream's deferred aggregate accounting. An
//! atomic variant remains available for focused reservation tests; the normal
//! route loop never allocates it.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Go routeSearchIsDescendantOf: charge before each identity/ownership step,
/// including nil comparisons. Container, cluster vessel, then sequence vessel
/// are the source's ownership precedence.
pub(super) fn is_descendant_of(
    graph: &super::ArenaGraph,
    mut descendant: Option<super::NodeId>,
    ancestor: Option<super::NodeId>,
    budget: Option<&LocalRouteSearchWork>,
) -> Option<bool> {
    loop {
        if let Some(budget) = budget {
            budget.step().ok()?;
        }
        if descendant == ancestor {
            return Some(true);
        }
        let Some(current) = descendant else {
            return Some(false);
        };
        let node = &graph.nodes[current.0 as usize];
        descendant = if let Some(container) = node.container {
            Some(container)
        } else if let Some(cluster) = node.cluster {
            graph
                .nodes
                .iter()
                .find(|candidate| candidate.tala_id == graph.clusters[cluster].vessel_tala_id)
                .map(|candidate| candidate.input_id)
        } else if let Some(sequence) = node.sequence {
            graph
                .nodes
                .iter()
                .find(|candidate| candidate.tala_id == graph.sequences[sequence].vessel_tala_id)
                .map(|candidate| candidate.input_id)
        } else {
            return Some(ancestor.is_none());
        };
    }
}

/// filterEdgeAncestorsGuarded charges node visits, but its ancestry
/// predicates use the unguarded ownership walk in upstream.
pub(super) fn filter_edge_ancestors(
    graph: &super::ArenaGraph,
    edge_index: usize,
    budget: Option<&LocalRouteSearchWork>,
) -> Option<Vec<super::NodeId>> {
    let edge = &graph.edge_records[edge_index];
    let mut nodes = Vec::with_capacity(graph.node_order.len());
    for &node in &graph.node_order {
        if let Some(guard) = budget {
            guard.step().ok()?;
        }
        if node != edge.from
            && node != edge.to
            && (is_descendant_of(graph, Some(edge.from), Some(node), None)?
                || is_descendant_of(graph, Some(edge.to), Some(node), None)?)
        {
            continue;
        }
        nodes.push(node);
    }
    Some(nodes)
}

/// Calibrated per-flavor envelope from v0.9.0's route-search guard.
pub const MAX_ROUTE_SEARCH_WORK_UNITS: u64 = 120_000_000;

/// Aggregate envelope for one v0.9.0 edge-routing stage.  D2 accounts OVG
/// construction, all three route flavors, and route-stage discovery against a
/// shared bound.  Keeping this counter separate from the per-flavor guards
/// preserves the source scheduling semantics while making the limit explicit
/// in the Rust implementation.
pub const MAX_ROUTE_STAGE_WORK_UNITS: u64 = 50_000_000;
pub const MAX_EDGE_ROUTING_STAGE_WORK_UNITS: u64 =
    MAX_OVG_WORK_UNITS + MAX_ROUTE_SEARCH_WORK_UNITS * 3 + MAX_ROUTE_STAGE_WORK_UNITS;

/// Independent visibility-graph construction limits from D2 v0.9.0's
/// `ovgBuildGuard`. These are deliberately separate from route-search work:
/// a large derived OVG must be rejected before it can consume unbounded
/// memory, even when Dijkstra itself would stay below its work envelope.
pub(super) const MAX_OVG_INTERSECTION_CANDIDATES: u64 = 1_000_000;
pub(super) const MAX_OVG_NODES: u64 = 200_000;
pub(super) const MAX_OVG_EDGES: u64 = 500_000;
pub(super) const MAX_OVG_WORK_UNITS: u64 = 250_000_000;

/// Thread-safe aggregate accounting used by the three parallel route flavor
/// workers and the OVG builder. Limit-rejected reservations mark the stage
/// exhausted without changing its count. A successful reservation that
/// reports cancellation at a source stride boundary retains the charge,
/// matching D2 v0.9.0's `routeWorkGuard.add`.
#[derive(Debug)]
pub(crate) struct RouteStageWork {
    /// routeWorkGuard.parallel: until a route-search guard shares this
    /// aggregate, one thread owns it and charges skip the CAS loop.
    parallel: AtomicBool,
    used: AtomicU64,
    limit: u64,
    exhausted: AtomicBool,
}

impl RouteStageWork {
    pub(crate) fn new(limit: u64) -> Arc<Self> {
        Arc::new(Self {
            parallel: AtomicBool::new(false),
            used: AtomicU64::new(0),
            limit,
            exhausted: AtomicBool::new(false),
        })
    }

    /// routeWorkGuard.enableParallel; never reverts.
    pub(crate) fn enable_parallel(&self) {
        self.parallel.store(true, Ordering::Release);
    }

    /// Accept `units` if they fit, returning the counter before the charge.
    /// The serial path is routeWorkGuard.add's single-owner branch: a plain
    /// read, check and write in place of the CAS loop.
    #[inline]
    fn try_charge(&self, units: u64) -> Option<u64> {
        if !self.parallel.load(Ordering::Acquire) {
            let previous = self.used.load(Ordering::Relaxed);
            if previous > self.limit || units > self.limit - previous {
                return None;
            }
            self.used.store(previous + units, Ordering::Relaxed);
            return Some(previous);
        }
        loop {
            let previous = self.used.load(Ordering::Acquire);
            if previous > self.limit || units > self.limit - previous {
                return None;
            }
            if self
                .used
                .compare_exchange_weak(
                    previous,
                    previous + units,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return Some(previous);
            }
        }
    }

    pub(crate) fn reserve(&self, units: u64) -> Result<(), ()> {
        self.reserve_in_scope(units, None)
    }

    /// `reserve` for a caller that cached its installed cancellation scope's
    /// Done presence (`Some`), as `OvgBuildBudget::for_current_scope` does;
    /// `None` reads that presence from the thread-local scope.
    #[inline]
    fn reserve_in_scope(&self, units: u64, scope_has_done: Option<bool>) -> Result<(), ()> {
        let Some(previous) = self.try_charge(units) else {
            if crate::internal::cancellation_requested() {
                return Err(());
            }
            self.exhausted.store(true, Ordering::Release);
            return Err(());
        };
        let current = previous + units;
        // Match routeWorkGuard.add: zero-unit charges and stride crossings
        // poll after accepting the charge, so it remains visible if
        // cancellation is observed at that boundary. The stride test is
        // pure, so test it before the thread-local Done lookup that only
        // matters off-stride.
        if (units == 0
            || previous / 1024 != current / 1024
            || !scope_has_done.unwrap_or_else(|| crate::internal::cancellation_done().is_some()))
            && crate::internal::cancellation_requested()
        {
            return Err(());
        }
        Ok(())
    }

    /// Source routeWorkGuard.add cadence with a caller-owned context. Unlike
    /// signed engine work, a nil Done channel does not poll every reservation.
    pub(super) fn reserve_with_check<E>(
        &self,
        units: u64,
        check: impl Fn() -> Result<(), E>,
        limit_error: impl FnOnce(u64) -> E,
    ) -> Result<(), E> {
        let Some(previous) = self.try_charge(units) else {
            check()?;
            self.exhausted.store(true, Ordering::Release);
            return Err(limit_error(self.limit));
        };
        let current = previous + units;
        if units == 0 || previous / 1024 != current / 1024 {
            check()?;
        }
        Ok(())
    }

    pub(crate) fn exhausted(&self) -> bool {
        self.exhausted.load(Ordering::Acquire)
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn used(&self) -> u64 {
        self.used.load(Ordering::Acquire)
    }
}

/// Reserve post-processing work with the same cancellation cadence as
/// v0.9.0's `routeWorkGuard.add`: poll on zero-unit charges, 1024-unit bucket
/// crossings, and rejected reservations, while retaining accepted charges if
/// cancellation is observed after a boundary crossing.
pub(crate) fn reserve_checked(budget: &RouteStageWork, units: u64) -> Result<(), ()> {
    budget.reserve(units)
}

pub(super) struct OvgBuildBudget<'a> {
    context: Option<&'a dyn crate::internal::limits::Context>,
    // The ordinary stage owns one TLS cancellation scope for this guard.
    // Cache only channel presence, as Go's ovgBuildGuard.done does.
    current_scope_has_done: Option<bool>,
    context_failure: std::cell::RefCell<Option<crate::internal::limits::ContextError>>,
    candidates: Cell<u64>,
    nodes: Cell<u64>,
    edges: Cell<u64>,
    work: Cell<u64>,
    exhausted: Cell<bool>,
    aggregate: Option<Arc<RouteStageWork>>,
}

impl<'a> OvgBuildBudget<'a> {
    pub(super) fn new() -> Self {
        Self {
            context: None,
            current_scope_has_done: None,
            context_failure: std::cell::RefCell::new(None),
            candidates: Cell::new(0),
            nodes: Cell::new(0),
            edges: Cell::new(0),
            work: Cell::new(0),
            exhausted: Cell::new(false),
            aggregate: None,
        }
    }

    /// Construct inside the ordinary routing stage's cancellation scope.
    /// Its token stays installed until the stage returns; cancellation state
    /// is still read by check at every original poll boundary. Unscoped and
    /// caller-context constructors retain their dynamic presence reads.
    pub(super) fn for_current_scope(aggregate: Option<Arc<RouteStageWork>>) -> Self {
        let mut budget = Self::new();
        budget.aggregate = aggregate;
        budget.current_scope_has_done = Some(crate::internal::cancellation_done().is_some());
        budget
    }

    fn has_done_signal(&self) -> bool {
        self.current_scope_has_done
            .unwrap_or_else(|| self.done().is_some())
    }

    /// The standalone owner API must retain its caller context through OVG
    /// construction. The normal pipeline continues to use its runtime token.
    pub(super) fn with_context(context: &'a dyn crate::internal::limits::Context) -> Self {
        let mut budget = Self::new();
        budget.context = Some(context);
        budget
    }
    fn done(&self) -> Option<bool> {
        self.context
            .map_or_else(crate::internal::cancellation_done, |context| context.done())
    }
    fn cancelled(&self) -> bool {
        let Some(context) = self.context else {
            return crate::internal::cancellation_requested();
        };
        if context.done() != Some(false)
            && let Some(cause) = context.error()
        {
            *self.context_failure.borrow_mut() = Some(cause);
            return true;
        }
        false
    }
    pub(super) fn check(&self) -> Result<(), ()> {
        if self.cancelled() { Err(()) } else { Ok(()) }
    }
    pub(super) fn take_context_error(&self) -> Option<crate::internal::limits::Error> {
        self.context_failure.borrow_mut().take().map(|cause| {
            crate::internal::limits::Error::Context {
                location: "EdgeRouting".into(),
                cause,
            }
        })
    }

    pub(super) fn context(&self) -> Option<&'a dyn crate::internal::limits::Context> {
        self.context
    }

    pub(super) fn with_aggregate(aggregate: Arc<RouteStageWork>) -> Self {
        let mut budget = Self::new();
        budget.aggregate = Some(aggregate);
        budget
    }

    fn reserve(
        &self,
        used: &Cell<u64>,
        amount: u64,
        limit: u64,
        _resource: &'static str,
    ) -> Result<(), ()> {
        if self.cancelled() {
            return Err(());
        }
        let Some(next) = used.get().checked_add(amount) else {
            self.exhausted.set(true);
            return Err(());
        };
        if next > limit {
            self.exhausted.set(true);
            return Err(());
        }
        used.set(next);
        Ok(())
    }

    pub(super) fn reserve_candidates(&self, amount: u64) -> Result<(), ()> {
        self.reserve(
            &self.candidates,
            amount,
            MAX_OVG_INTERSECTION_CANDIDATES,
            "intersection candidate count",
        )
    }

    pub(super) fn reserve_nodes(&self, amount: u64) -> Result<(), ()> {
        self.reserve(&self.nodes, amount, MAX_OVG_NODES, "node count")
    }

    pub(super) fn reserve_edges(&self, amount: u64) -> Result<(), ()> {
        self.reserve(&self.edges, amount, MAX_OVG_EDGES, "edge count")
    }
    pub(super) fn reserved_edge_count(&self) -> u64 {
        self.edges.get()
    }

    #[inline(always)]
    pub(super) fn reserve_work(&self, amount: u64) -> Result<(), ()> {
        // ovgBuildGuard.reserveWork only inspects the context on a stride
        // crossing when Done is present. Keep that ordinary numerical path
        // inside OVG scans; caller contexts, explicit checks and rejected
        // charges retain the complete source path below.
        // This must inline even when LLVM's size estimate includes aggregate
        // failure handling: an ordinary step only updates the two counters.
        let previous = self.work.get();
        if self.current_scope_has_done == Some(true)
            && amount != 0
            && let Some(next) = previous.checked_add(amount)
            && next <= MAX_OVG_WORK_UNITS
            && previous / 1024 == next / 1024
        {
            self.work.set(next);
            return self.reserve_aggregate_work(amount);
        }
        self.reserve_work_checked(amount)
    }

    #[inline(never)]
    fn reserve_work_checked(&self, amount: u64) -> Result<(), ()> {
        let previous = self.work.get();
        let Some(next) = previous.checked_add(amount) else {
            if self.cancelled() {
                return Err(());
            }
            self.exhausted.set(true);
            return Err(());
        };
        if next > MAX_OVG_WORK_UNITS {
            if self.cancelled() {
                return Err(());
            }
            self.exhausted.set(true);
            return Err(());
        }
        // Go's OVG guard polls a cancellable context only when a bulk charge
        // crosses its 1024-unit boundary; synthetic contexts without Done and
        // zero-unit charges are checked on every call.
        if (!self.has_done_signal() || amount == 0 || previous / 1024 != next / 1024)
            && self.cancelled()
        {
            return Err(());
        }
        self.work.set(next);
        self.reserve_aggregate_work(amount)
    }

    // Keep the aggregate half of reserve_work's ordinary counter update
    // in the scan too; outlining it loses constant step-size propagation.
    #[inline(always)]
    fn reserve_aggregate_work(&self, amount: u64) -> Result<(), ()> {
        // The aggregate charge runs in the same installed scope, so it reuses
        // this guard's cached Done presence instead of a thread-local read.
        if let Some(aggregate) = &self.aggregate
            && aggregate
                .reserve_in_scope(amount, self.current_scope_has_done)
                .is_err()
        {
            if !self.cancelled() {
                self.exhausted.set(true);
            }
            return Err(());
        }
        Ok(())
    }

    /// ovgBuildGuard.reserveSortWork reserves the entire comparison envelope
    /// in one operation. Splitting it into levels changes cancellation and
    /// partial-charge behavior even when the successful total is identical.
    pub(super) fn reserve_sort_work(&self, length: usize) -> Result<(), ()> {
        if length < 2 {
            return self.check();
        }
        let levels = (usize::BITS - (length - 1).leading_zeros()) as u64;
        let Some(work) = u64::try_from(length)
            .ok()
            .and_then(|n| n.checked_mul(levels))
        else {
            // The source sort arithmetic overflow returns without another
            // context check; reserveWork has its own cancellation precedence.
            self.exhausted.set(true);
            return Err(());
        };
        self.reserve_work(work)
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn observed_resources(&self) -> [u64; 4] {
        [
            self.work.get(),
            self.nodes.get(),
            self.edges.get(),
            self.candidates.get(),
        ]
    }

    pub(super) fn exhausted(&self) -> bool {
        self.exhausted.get()
    }
}

/// Atomic reservation model used by focused concurrency/overflow tests.
#[derive(Debug)]
pub struct RouteSearchWork {
    used: AtomicU64,
    limit: u64,
    exhausted: AtomicBool,
}

impl RouteSearchWork {
    pub fn new(limit: u64) -> Self {
        Self {
            used: AtomicU64::new(0),
            limit,
            exhausted: AtomicBool::new(false),
        }
    }

    /// Reserve work before mutating any queue or route state.
    pub fn reserve(&self, units: u64) -> Result<(), ()> {
        let result = self
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(units).filter(|next| *next <= self.limit)
            });
        if result.is_err() {
            self.exhausted.store(true, Ordering::Release);
            return Err(());
        }
        Ok(())
    }

    pub fn exhausted(&self) -> bool {
        self.exhausted.load(Ordering::Acquire)
    }
}

/// Per-flavor guard matching Go's `routeSearchWorkGuard`. A flavor runs on one
/// worker, so a `Cell` avoids an atomic read/modify/write for every queue
/// operation. Production workers publish their final local total at
/// `finish`, just as the upstream default route-stage guard does.
pub struct LocalRouteSearchWork<'a> {
    context: Cell<Option<&'a dyn crate::internal::limits::Context>>,
    context_failure: std::cell::RefCell<Option<crate::internal::limits::ContextError>>,
    used: Cell<u64>,
    limit: u64,
    exhausted: Cell<bool>,
    next_context_check: Cell<u64>,
    /// Set by bind_current_scope when that scope carries a Done signal. Its
    /// presence is fixed until the worker returns, as Go's cached
    /// routeSearchWorkGuard.done is; any other bind clears it.
    scope_has_done: Cell<bool>,
    aggregate: Option<Arc<RouteStageWork>>,
    defer_aggregate: bool,
    aggregate_charged: Cell<u64>,
}

/// Movable state of an ordinary worker guard. Borrowed caller contexts never
/// cross threads; each receiving thread installs and binds its own context.
pub(super) struct RouteSearchWorkState {
    context_failure: Option<crate::internal::limits::ContextError>,
    used: u64,
    limit: u64,
    exhausted: bool,
    next_context_check: u64,
    aggregate: Option<Arc<RouteStageWork>>,
    defer_aggregate: bool,
    aggregate_charged: u64,
}

impl RouteSearchWorkState {
    pub(super) fn resume<'a>(self) -> LocalRouteSearchWork<'a> {
        LocalRouteSearchWork {
            context: Cell::new(None),
            context_failure: std::cell::RefCell::new(self.context_failure),
            used: Cell::new(self.used),
            limit: self.limit,
            exhausted: Cell::new(self.exhausted),
            next_context_check: Cell::new(self.next_context_check),
            scope_has_done: Cell::new(false),
            aggregate: self.aggregate,
            defer_aggregate: self.defer_aggregate,
            aggregate_charged: Cell::new(self.aggregate_charged),
        }
    }
}

impl<'a> LocalRouteSearchWork<'a> {
    pub(super) fn limit(&self) -> u64 {
        self.limit
    }

    pub(super) fn into_state(self) -> RouteSearchWorkState {
        RouteSearchWorkState {
            context_failure: self.context_failure.into_inner(),
            used: self.used.into_inner(),
            limit: self.limit,
            exhausted: self.exhausted.into_inner(),
            next_context_check: self.next_context_check.into_inner(),
            aggregate: self.aggregate,
            defer_aggregate: self.defer_aggregate,
            aggregate_charged: self.aggregate_charged.into_inner(),
        }
    }

    pub(super) fn new(limit: u64) -> Self {
        Self {
            context: Cell::new(None),
            context_failure: std::cell::RefCell::new(None),
            used: Cell::new(0),
            limit,
            exhausted: Cell::new(false),
            next_context_check: Cell::new(1024),
            scope_has_done: Cell::new(false),
            aggregate: None,
            defer_aggregate: false,
            aggregate_charged: Cell::new(0),
        }
    }

    /// Constructors poll the caller; generateRoutes and finalization rebind
    /// the same guard without resetting its accepted work.
    pub(super) fn with_context(
        limit: u64,
        context: &'a dyn crate::internal::limits::Context,
    ) -> Result<Self, crate::LayoutError> {
        let work = Self::new(limit);
        work.context.set(Some(context));
        work.check().map_err(|_| work.failure())?;
        Ok(work)
    }

    pub(super) fn bind(
        &self,
        context: &'a dyn crate::internal::limits::Context,
    ) -> Result<(), crate::LayoutError> {
        self.scope_has_done.set(false);
        self.context.set(Some(context));
        self.check().map_err(|_| self.failure())?;
        self.next_context_check
            .set(self.used.get().wrapping_add(1024));
        Ok(())
    }

    /// Bind the runtime route context for a flavor worker that runs entirely
    /// inside its own cancellation scope, caching that scope's Done presence.
    /// The Done state itself is still read at every poll.
    pub(super) fn bind_current_scope(&self) -> Result<(), crate::LayoutError> {
        self.bind(&super::route_stage_guard::RuntimeRouteContext)?;
        self.scope_has_done
            .set(crate::internal::cancellation_done().is_some());
        Ok(())
    }

    /// A present Done signal below the stride needs no poll: the runtime
    /// context's Done read is pure and its result would be discarded.
    #[inline]
    fn off_stride_with_done(&self) -> bool {
        self.scope_has_done.get() && self.used.get() < self.next_context_check.get()
    }

    fn done(&self) -> Option<bool> {
        self.context
            .get()
            .map_or_else(crate::internal::cancellation_done, |ctx| ctx.done())
    }

    fn cancelled(&self) -> bool {
        let Some(context) = self.context.get() else {
            return crate::internal::cancellation_requested();
        };
        if context.done() != Some(false)
            && let Some(cause) = context.error()
        {
            *self.context_failure.borrow_mut() = Some(cause);
            return true;
        }
        false
    }

    /// `cancelled` for a step that has just read `done`: the context's Done
    /// state is not read a second time.
    fn cancelled_after(&self, done: Option<bool>) -> bool {
        let Some(context) = self.context.get() else {
            return crate::internal::cancellation_requested();
        };
        if done != Some(false)
            && let Some(cause) = context.error()
        {
            *self.context_failure.borrow_mut() = Some(cause);
            return true;
        }
        false
    }

    pub(super) fn failure(&self) -> crate::LayoutError {
        match self.context_failure.borrow().as_ref() {
            Some(crate::internal::limits::ContextError::Cancelled) => crate::LayoutError::Cancelled,
            Some(crate::internal::limits::ContextError::DeadlineExceeded) => {
                crate::LayoutError::DeadlineExceeded
            }
            None => crate::LayoutError::BadRouteState("route-search work limit exceeded".into()),
        }
    }

    pub(super) fn with_aggregate(limit: u64, aggregate: Arc<RouteStageWork>) -> Self {
        // newRouteSearchWorkGuard switches the shared aggregate to its
        // parallel path before any flavor worker can charge it.
        aggregate.enable_parallel();
        let mut budget = Self::new(limit);
        budget.aggregate = Some(aggregate);
        budget
    }

    /// Construct the production flavor guard. Go's route-search workers keep
    /// their counters local and charge the shared edge-stage budget only when
    /// a flavor finishes. This avoids making the result depend on which
    /// parallel worker happens to reserve the next unit.
    pub(super) fn with_deferred_aggregate(limit: u64, aggregate: Arc<RouteStageWork>) -> Self {
        aggregate.enable_parallel();
        let mut budget = Self::new(limit);
        budget.aggregate = Some(aggregate);
        budget.defer_aggregate = true;
        budget
    }

    /// work.go chargeSkippedRouteWork preserves step-wise polling for nil
    /// Done/custom aggregate budgets, and saturates a bounded bulk prefix.
    pub(super) fn charge_skipped(&self, amount: usize) -> Result<(), ()> {
        if amount == 0 {
            return Ok(());
        }
        if self.done().is_none() || (self.aggregate.is_some() && !self.defer_aggregate) {
            for _ in 0..amount {
                self.step()?;
            }
            return Ok(());
        }
        if self.used.get() >= self.limit {
            return self.step();
        }
        let remaining = self.limit - self.used.get();
        if amount as u64 > remaining {
            self.add(remaining)?;
            self.step()
        } else {
            self.add(amount as u64)
        }
    }

    /// Pinned standalone_route_guard.go:179–205, edgeCanOverlapEdgesGuarded.
    /// Empty lists need one check with a Done signal; Err-only contexts retain
    /// all ten zero-charge polls and the final check. Read this guard's bound
    /// context, which may differ from the thread's cancellation context.
    pub(super) fn charge_overlap_checks(&self, count: usize) -> Result<(), ()> {
        if count == 0 && self.done().is_some() {
            return self.check();
        }
        for _ in 0..10 {
            self.add(count as u64)?;
        }
        self.check()
    }

    fn check_before_step(&self) -> Result<(), ()> {
        // routeSearchWorkGuard.step checks a cancellable context only when the
        // prior count has reached the stride. A nil Done channel is checked on
        // every call (the Rust token-less path is observably a no-op check).
        if self.off_stride_with_done() {
            return Ok(());
        }
        let done = self.done();
        if done.is_none() || self.used.get() >= self.next_context_check.get() {
            if self.cancelled_after(done) {
                return Err(());
            }
            self.next_context_check
                .set(self.used.get().saturating_add(1024));
        }
        Ok(())
    }

    /// Match Go's route-search guard: a rejected reservation rechecks context
    /// before deciding that the work limit was the cause. This preserves
    /// cancellation precedence without marking a cancelled worker exhausted.
    fn reject_limit(&self) -> Result<(), ()> {
        if self.cancelled() {
            return Err(());
        }
        self.exhausted.set(true);
        Err(())
    }

    #[inline]
    pub(super) fn reserve(&self, units: u64) -> Result<(), ()> {
        self.add_impl(units)
    }

    /// A nonzero in-limit charge of a deferred current-scope guard that stays
    /// below the stride only counts, exactly as add_impl_slow would.
    #[inline]
    fn add_impl(&self, units: u64) -> Result<(), ()> {
        let used = self.used.get();
        if units != 0
            && used <= self.limit
            && units <= self.limit - used
            && (self.defer_aggregate || self.aggregate.is_none())
            && self.scope_has_done.get()
            && used + units < self.next_context_check.get()
        {
            self.used.set(used + units);
            return Ok(());
        }
        self.add_impl_slow(units)
    }

    fn add_impl_slow(&self, units: u64) -> Result<(), ()> {
        let used = self.used.get();
        if used > self.limit {
            return self.reject_limit();
        }
        if units == 0 {
            // Go's bulk path checks explicitly on a zero charge, regardless
            // of the next stride boundary. A cancellation can arrive between
            // two otherwise small charges and must be observable here.
            if self.cancelled() {
                return Err(());
            }
            self.next_context_check
                .set(self.used.get().saturating_add(1024));
            return Ok(());
        }
        if units > self.limit - used {
            // routeSearchWorkGuard returns before mutating `used` when a
            // bulk charge would exceed its limit. The generic route-stage
            // guard has a different limit+1 convention; this is the
            // route-search guard and must follow its own Go implementation.
            return self.reject_limit();
        }
        let next = used + units;
        self.used.set(next);
        if !self.defer_aggregate
            && let Some(aggregate) = &self.aggregate
            && aggregate.reserve(units).is_err()
        {
            return self.reject_limit();
        }
        // Go checks after a valid bulk charge when it crosses a stride. The
        // charge remains visible when cancellation is reported at that point.
        if self.off_stride_with_done() {
            return Ok(());
        }
        let done = self.done();
        if done.is_none() || self.used.get() >= self.next_context_check.get() {
            if self.cancelled_after(done) {
                return Err(());
            }
            self.next_context_check
                .set(self.used.get().saturating_add(1024));
        }
        Ok(())
    }

    /// Account one inner-loop operation, matching the v0.9.0 guard's
    /// `step` method. Keeping this as a small forwarding method makes the
    /// call sites read like the upstream routing code while retaining the
    /// allocation-free `Cell` counter on a flavor worker.
    #[inline(always)]
    pub(super) fn step(&self) -> Result<(), ()> {
        // Off-stride, in-limit steps of a deferred current-scope guard only
        // count; every other step takes the complete source path.
        let used = self.used.get();
        if self.off_stride_with_done()
            && used < self.limit
            && (self.defer_aggregate || self.aggregate.is_none())
        {
            self.used.set(used + 1);
            return Ok(());
        }
        self.step_slow()
    }

    #[cold]
    #[inline(never)]
    fn step_slow(&self) -> Result<(), ()> {
        self.check_before_step()?;
        if self.used.get() >= self.limit {
            return self.reject_limit();
        }
        self.used.set(self.used.get() + 1);
        if !self.defer_aggregate
            && let Some(aggregate) = &self.aggregate
            && aggregate.reserve(1).is_err()
        {
            return self.reject_limit();
        }
        Ok(())
    }

    /// Account a bulk operation before the caller mutates route state.
    #[inline]
    pub(super) fn add(&self, units: u64) -> Result<(), ()> {
        self.add_impl(units)
    }

    /// Poll cancellation at source call sites that explicitly invoke
    /// `routeSearchWorkGuard.check` after a mutation. This does not charge
    /// work or advance the bounded `step` polling cadence.
    pub(super) fn check(&self) -> Result<(), ()> {
        if self.cancelled() { Err(()) } else { Ok(()) }
    }

    pub(super) fn reserve_product(&self, left: u64, right: u64) -> Result<(), ()> {
        match left.checked_mul(right) {
            Some(units) => self.add(units),
            None => self.reject_limit(),
        }
    }

    pub(super) fn reserve_sum(&self, left: u64, right: u64) -> Result<(), ()> {
        match left.checked_add(right) {
            Some(units) => self.add(units),
            None => self.reject_limit(),
        }
    }

    /// Reserve the comparison work for a stable sort using the same
    /// `len*floor(log2(len-1))` envelope as D2 v0.9.0.
    pub(super) fn reserve_sort(&self, length: usize) -> Result<(), ()> {
        if length < 2 {
            return self.step();
        }
        let mut levels = 0u64;
        let mut remaining = (length - 1) as u64;
        while remaining != 0 {
            levels += 1;
            remaining >>= 1;
        }
        self.reserve_product(length as u64, levels)
    }

    /// Publish a deferred flavor total at the same boundary as Go's
    /// routeSearchWorkGuard.finish. The local count remains authoritative even
    /// when the shared stage rejects the final charge, so diagnostics and
    /// limit classification retain the worker's source-shaped total.
    pub(super) fn finish(&self) -> Result<(), ()> {
        if self.defer_aggregate
            && let Some(aggregate) = &self.aggregate
        {
            let used = self.used.get();
            let charged = self.aggregate_charged.get();
            if used > charged {
                let pending = used - charged;
                if aggregate.reserve(pending).is_err() {
                    return self.reject_limit();
                }
                self.aggregate_charged.set(used);
            }
        }
        if self.cancelled() {
            return Err(());
        }
        Ok(())
    }

    pub(super) fn exhausted(&self) -> bool {
        self.exhausted.get()
    }

    /// Inspect a failure already observed by a fallible operation. Building a
    /// worker response must not poll again or replace it with a route invariant.
    pub(super) fn has_failed(&self) -> bool {
        self.exhausted.get() || self.context_failure.borrow().is_some()
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn used(&self) -> u64 {
        self.used.get()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LocalRouteSearchWork, MAX_OVG_EDGES, MAX_OVG_INTERSECTION_CANDIDATES, MAX_OVG_NODES,
        MAX_OVG_WORK_UNITS, OvgBuildBudget, RouteSearchWork, RouteStageWork,
    };
    use std::sync::atomic::Ordering;

    #[test]
    fn worker_handoff_preserves_accepted_work_and_aggregate_charge() {
        let aggregate = RouteStageWork::new(20);
        let parent = d2_rs_core::CancellationToken::new();
        let child = parent.child();
        let context = super::super::route_stage_guard::RuntimeRouteContext;
        let work = LocalRouteSearchWork::with_deferred_aggregate(14, aggregate.clone());
        crate::internal::with_cancellation(Some(&parent), || {
            work.bind(&context).unwrap();
            work.add(7).unwrap();
        });
        let state = work.into_state();
        let state = std::thread::spawn(move || {
            crate::internal::with_cancellation(Some(&child), || {
                let context = super::super::route_stage_guard::RuntimeRouteContext;
                let work = state.resume();
                work.bind(&context).unwrap();
                assert_eq!(work.used(), 7);
                work.add(5).unwrap();
                work.finish().unwrap();
                child.cancel();
                work.into_state()
            })
        })
        .join()
        .unwrap();
        assert_eq!(aggregate.used(), 12);
        crate::internal::with_cancellation(Some(&parent), || {
            let work = state.resume();
            work.bind(&context).unwrap();
            assert_eq!(work.used(), 12);
            work.add(2).unwrap();
            work.finish().unwrap();
            assert_eq!(aggregate.used(), 14);
            assert!(work.step().is_err());
            let state = work.into_state();
            let work = state.resume();
            assert!(work.exhausted());
            assert_eq!(work.used(), 14);
            work.finish().unwrap();
            assert_eq!(aggregate.used(), 14);
        });
        assert!(parent.error().is_none());
    }

    struct CallerContext {
        polls: std::cell::Cell<u64>,
        done: Option<bool>,
        cause: Option<crate::internal::limits::ContextError>,
    }
    impl crate::internal::limits::Context for CallerContext {
        fn done(&self) -> Option<bool> {
            self.done
        }
        fn error(&self) -> Option<crate::internal::limits::ContextError> {
            self.polls.set(self.polls.get() + 1);
            self.cause.clone()
        }
    }

    #[test]
    fn ovg_sort_is_one_atomic_work_reservation() {
        let context = CallerContext {
            polls: std::cell::Cell::new(0),
            done: None,
            cause: None,
        };
        let budget = OvgBuildBudget::with_context(&context);
        budget.reserve_sort_work(5).unwrap();
        assert_eq!(budget.work.get(), 15);
        assert_eq!(context.polls.get(), 1);
        budget.work.set(MAX_OVG_WORK_UNITS - 10);
        assert!(budget.reserve_sort_work(5).is_err());
        assert_eq!(budget.work.get(), MAX_OVG_WORK_UNITS - 10);
        assert_eq!(context.polls.get(), 2);
        assert!(budget.exhausted());
    }

    #[test]
    fn ovg_explicit_context_preserves_cause_and_precedes_resource_failure() {
        use crate::internal::limits::{ContextError, Error};
        for cause in [ContextError::Cancelled, ContextError::DeadlineExceeded] {
            let context = CallerContext {
                polls: std::cell::Cell::new(0),
                done: None,
                cause: Some(cause.clone()),
            };
            let budget = OvgBuildBudget::with_context(&context);
            assert!(budget.reserve_nodes(MAX_OVG_NODES + 1).is_err());
            assert_eq!(budget.nodes.get(), 0);
            assert!(!budget.exhausted());
            assert_eq!(context.polls.get(), 1);
            assert_eq!(
                budget.take_context_error(),
                Some(Error::Context {
                    location: "EdgeRouting".into(),
                    cause
                })
            );
            assert!(budget.take_context_error().is_none());
        }
    }

    #[test]
    fn ovg_open_done_does_not_query_err_and_short_sorts_do_not_charge() {
        let context = CallerContext {
            polls: std::cell::Cell::new(0),
            done: Some(false),
            cause: Some(crate::internal::limits::ContextError::Cancelled),
        };
        let budget = OvgBuildBudget::with_context(&context);
        budget.check().unwrap();
        budget.reserve_nodes(1).unwrap();
        budget.reserve_sort_work(0).unwrap();
        budget.reserve_sort_work(1).unwrap();
        budget.reserve_sort_work(5).unwrap();
        assert_eq!(budget.work.get(), 15);
        assert_eq!(context.polls.get(), 0);
        assert!(budget.take_context_error().is_none());
    }

    #[test]
    fn reservation_is_pre_mutation_and_limit_is_sticky() {
        let budget = RouteSearchWork::new(3);
        assert!(budget.reserve(2).is_ok());
        assert!(budget.reserve(2).is_err());
        assert!(budget.exhausted());
        // A rejected reservation does not consume the remaining unit.
        assert!(budget.reserve(1).is_ok());
        assert!(budget.reserve(1).is_err());
    }

    #[test]
    fn arithmetic_overflow_is_classified_as_exhaustion() {
        let budget = RouteSearchWork::new(u64::MAX);
        assert!(budget.reserve(u64::MAX).is_ok());
        assert!(budget.reserve(1).is_err());
        assert!(budget.exhausted());
    }

    #[test]
    fn local_flavor_budget_has_the_same_boundary_without_atomics() {
        let budget = LocalRouteSearchWork::new(3);
        assert!(budget.reserve(2).is_ok());
        assert_eq!(budget.used.get(), 2);
        assert!(budget.reserve(2).is_err());
        // routeSearchWorkGuard leaves the local counter unchanged when a
        // rejected bulk charge crosses the per-flavor limit.
        assert_eq!(budget.used.get(), 2);
        assert!(budget.exhausted());
    }

    #[test]
    fn local_step_checks_cancellation_before_the_stride_increment() {
        let budget = LocalRouteSearchWork::new(u64::MAX);
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            for _ in 0..1024 {
                budget.step().unwrap();
            }
            token.cancel();
            assert!(budget.step().is_err());
            assert_eq!(budget.used.get(), 1024);
        });
    }

    #[test]
    fn local_step_rechecks_cancellation_when_the_work_limit_is_hit() {
        let budget = LocalRouteSearchWork::new(1);
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            budget.step().unwrap();
            token.cancel();
            assert!(budget.step().is_err());
        });
        assert_eq!(budget.used.get(), 1);
        assert!(!budget.exhausted());
    }

    #[test]
    fn local_bulk_rejection_rechecks_cancellation_and_preserves_the_counter() {
        let budget = LocalRouteSearchWork::new(1);
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            budget.add(1).unwrap();
            token.cancel();
            assert!(budget.add(1).is_err());
        });
        assert_eq!(budget.used.get(), 1);
        assert!(!budget.exhausted());
    }

    #[test]
    fn local_overflow_rechecks_cancellation_before_classifying_the_limit() {
        let budget = LocalRouteSearchWork::new(u64::MAX);
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            token.cancel();
            assert!(budget.reserve_product(u64::MAX, 2).is_err());
        });
        assert_eq!(budget.used.get(), 0);
        assert!(!budget.exhausted());
    }

    #[test]
    fn aggregate_rejection_rechecks_cancellation_after_local_charge() {
        let aggregate = RouteStageWork::new(1);
        let budget = LocalRouteSearchWork::with_aggregate(2, aggregate.clone());
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            budget.add(1).unwrap();
            token.cancel();
            assert!(budget.add(1).is_err());
        });
        assert_eq!(budget.used.get(), 2);
        assert!(!budget.exhausted());
        assert!(!aggregate.exhausted());
        assert_eq!(aggregate.used(), 1);
    }

    #[test]
    fn deferred_aggregate_finish_prefers_cancellation_over_stage_limit() {
        let aggregate = RouteStageWork::new(0);
        let budget = LocalRouteSearchWork::with_deferred_aggregate(2, aggregate.clone());
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            budget.add(1).unwrap();
            token.cancel();
            assert!(budget.finish().is_err());
        });
        assert!(!budget.exhausted());
        assert!(!aggregate.exhausted());
        assert_eq!(aggregate.used(), 0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cancellation_handle_can_be_installed_on_parallel_flavor_workers() {
        let token = crate::CancellationToken::new();
        let worker_token = crate::internal::with_cancellation(Some(&token), || {
            crate::internal::current_cancellation()
        });
        token.cancel();
        let (left, right) = rayon::join(
            || {
                crate::internal::with_cancellation(worker_token.as_ref(), || {
                    crate::internal::cancellation_requested()
                })
            },
            || {
                crate::internal::with_cancellation(worker_token.as_ref(), || {
                    crate::internal::cancellation_requested()
                })
            },
        );
        assert!(left && right);
    }

    #[test]
    fn skipped_axis_prefix_saturates_limit_in_both_context_modes() {
        for cancellable in [false, true] {
            let token = crate::CancellationToken::new();
            crate::internal::with_cancellation(cancellable.then_some(&token), || {
                let budget = LocalRouteSearchWork::new(7);
                budget.add(2).unwrap();
                assert!(budget.charge_skipped(6).is_err());
                assert_eq!(budget.used(), 7);
                assert!(budget.exhausted());
                assert!(budget.charge_skipped(0).is_ok());
            });
        }
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            let budget = LocalRouteSearchWork::new(7);
            budget.add(2).unwrap();
            token.cancel();
            assert!(budget.charge_skipped(6).is_err());
            assert!(!budget.exhausted());
        });
    }

    #[test]
    fn bulk_and_sort_reservations_use_checked_upstream_shapes() {
        let budget = LocalRouteSearchWork::new(32);
        assert!(budget.step().is_ok());
        assert!(budget.add(2).is_ok());
        assert!(budget.reserve_sum(3, 4).is_ok());
        assert!(budget.reserve_product(2, 3).is_ok());
        // len=5 has ceil(log2(4))=3 comparison levels, so reserve_sort adds
        // fifteen units after the preceding sixteen.
        assert!(budget.reserve_sort(5).is_ok());
        assert_eq!(budget.used.get(), 31);
    }

    #[test]
    fn checked_bulk_reservations_do_not_mutate_on_overflow() {
        let budget = LocalRouteSearchWork::new(u64::MAX);
        assert!(budget.reserve_product(u64::MAX, 2).is_err());
        assert_eq!(budget.used.get(), 0);
        assert!(budget.reserve_sum(u64::MAX, 1).is_err());
        assert_eq!(budget.used.get(), 0);
        assert!(budget.exhausted());
    }

    #[test]
    fn aggregate_stage_budget_is_shared_by_ovg_and_flavor_work() {
        let aggregate = RouteStageWork::new(8);
        let flavor = LocalRouteSearchWork::with_aggregate(32, aggregate.clone());
        assert!(flavor.add(3).is_ok());
        assert_eq!(aggregate.used(), 3);

        let ovg = OvgBuildBudget::with_aggregate(aggregate.clone());
        assert!(ovg.reserve_nodes(10).is_ok());
        assert_eq!(aggregate.used(), 3);
        assert!(ovg.reserve_work(4).is_ok());
        assert_eq!(aggregate.used(), 7);

        // The aggregate rejects before the eighth unit can be charged. The
        // local guards retain their own source-shaped counters but become
        // exhausted so the stage can classify the failure deterministically.
        assert!(flavor.add(2).is_err());
        assert!(aggregate.exhausted());
        assert_eq!(aggregate.used(), 7);
    }

    #[test]
    fn stage_guard_rechecks_cancellation_on_rejected_and_stride_boundary_charges() {
        let limited = RouteStageWork::new(1);
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            limited.reserve(1).unwrap();
            token.cancel();
            assert!(limited.reserve(1).is_err());
        });
        assert_eq!(limited.used(), 1);
        assert!(!limited.exhausted());

        let strided = RouteStageWork::new(2048);
        strided.used.store(1023, Ordering::Release);
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            strided.reserve(1).unwrap();
            assert_eq!(strided.used(), 1024);
            token.cancel();
            // A normal charge within the next bucket is not polled; a zero
            // charge is the explicit cancellation checkpoint.
            strided.reserve(1).unwrap();
            assert!(strided.reserve(0).is_err());
        });
        assert_eq!(strided.used(), 1025);
        assert!(!strided.exhausted());
    }

    #[test]
    fn checked_stage_reservation_keeps_upstream_poll_stride() {
        let budget = RouteStageWork::new(2048);
        budget.used.store(1024, Ordering::Release);
        let token = crate::CancellationToken::new();
        token.cancel();
        crate::internal::with_cancellation(Some(&token), || {
            // D2 does not poll every accepted add inside a bucket.
            assert!(super::reserve_checked(&budget, 1).is_ok());
            assert_eq!(budget.used(), 1025);
            // A zero-unit add is an explicit checkpoint in the source guard.
            assert!(super::reserve_checked(&budget, 0).is_err());
        });
        assert_eq!(budget.used(), 1025);
        assert!(!budget.exhausted());
    }

    #[test]
    fn deferred_flavor_budget_publishes_once_at_finish() {
        let aggregate = RouteStageWork::new(8);
        let flavor = LocalRouteSearchWork::with_deferred_aggregate(32, aggregate.clone());
        assert!(flavor.add(3).is_ok());
        assert_eq!(aggregate.used(), 0);
        assert!(flavor.step().is_ok());
        assert_eq!(aggregate.used(), 0);
        assert!(flavor.finish().is_ok());
        assert_eq!(aggregate.used(), 4);

        let rejected = LocalRouteSearchWork::with_deferred_aggregate(32, aggregate.clone());
        assert!(rejected.add(5).is_ok());
        assert!(rejected.finish().is_err());
        assert!(rejected.exhausted());
        assert_eq!(aggregate.used(), 4);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn aggregate_stage_budget_is_safe_for_parallel_flavors() {
        let aggregate = RouteStageWork::new(2);
        let left = aggregate.clone();
        let right = aggregate.clone();
        let (left_result, right_result) =
            rayon::join(move || left.reserve(2), move || right.reserve(1));
        assert!(left_result.is_ok() ^ right_result.is_ok());
        assert!(aggregate.exhausted());
        let used = aggregate.used();
        assert!(used == 1 || used == 2);
    }

    #[test]
    fn local_guard_polls_cancellation_at_the_upstream_stride() {
        let budget = LocalRouteSearchWork::new(u64::MAX);
        let token = crate::CancellationToken::new();
        token.cancel();
        let result = crate::internal::with_cancellation(Some(&token), || budget.add(1024));
        assert!(result.is_err());
        assert!(!budget.exhausted());
        assert_eq!(budget.used.get(), 1024);
    }

    #[test]
    fn zero_bulk_charge_polls_cancellation_before_the_stride() {
        let budget = LocalRouteSearchWork::new(u64::MAX);
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            budget.add(1).unwrap();
            token.cancel();
            assert!(budget.add(0).is_err());
            assert_eq!(budget.used.get(), 1);
        });
    }

    #[test]
    fn zero_bulk_charge_advances_the_next_context_poll_without_charging_work() {
        let budget = LocalRouteSearchWork::new(2048);
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            budget.add(1).unwrap();
            budget.add(0).unwrap();
            assert_eq!(budget.next_context_check.get(), 1025);
            assert_eq!(budget.used.get(), 1);

            budget.add(1023).unwrap();
            token.cancel();
            // The source polls at 1025 after the zero-cost checkpoint, not
            // at the old 1024 boundary.
            budget.step().unwrap();
            assert_eq!(budget.used.get(), 1025);
            assert!(budget.step().is_err());
        });
        assert_eq!(budget.used.get(), 1025);
        assert!(!budget.exhausted());
    }

    #[test]
    fn zero_bulk_charge_still_rejects_an_already_exceeded_counter() {
        let budget = LocalRouteSearchWork::new(1);
        budget.used.set(2);
        assert!(budget.add(0).is_err());
        assert_eq!(budget.used.get(), 2);
        assert!(budget.exhausted());
    }

    #[test]
    fn ovg_reservations_are_checked_before_counter_mutation() {
        let budget = OvgBuildBudget::new();
        assert!(
            budget
                .reserve_candidates(MAX_OVG_INTERSECTION_CANDIDATES + 1)
                .is_err()
        );
        assert!(budget.reserve_nodes(MAX_OVG_NODES + 1).is_err());
        assert!(budget.reserve_edges(MAX_OVG_EDGES + 1).is_err());
        assert!(budget.reserve_work(MAX_OVG_WORK_UNITS + 1).is_err());
        assert!(budget.exhausted());
    }

    #[test]
    fn ovg_sort_budget_matches_source_width_doubling_envelope() {
        for (length, expected_work) in [(0, 0), (1, 0), (2, 2), (3, 6), (4, 8), (5, 15)] {
            let budget = OvgBuildBudget::new();
            budget.reserve_sort_work(length).unwrap();
            assert_eq!(budget.work.get(), expected_work, "sort length {length}");
        }
    }

    #[test]
    fn ovg_reservations_observe_cancellation() {
        let budget = OvgBuildBudget::new();
        let token = crate::CancellationToken::new();
        token.cancel();
        let result = crate::internal::with_cancellation(Some(&token), || budget.reserve_nodes(1));
        assert!(result.is_err());
        assert!(!budget.exhausted());
    }

    #[test]
    fn ovg_work_guard_polls_at_source_stride_and_prefers_cancellation_at_limit() {
        let budget = OvgBuildBudget::new();
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            budget.reserve_work(1).unwrap();
            token.cancel();
            budget.reserve_work(1).unwrap();
            assert!(budget.reserve_work(1022).is_err());
        });
        assert_eq!(budget.work.get(), 2);
        assert!(!budget.exhausted());

        let over_limit = OvgBuildBudget::new();
        over_limit.work.set(MAX_OVG_WORK_UNITS);
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            token.cancel();
            assert!(over_limit.reserve_work(1).is_err());
        });
        assert_eq!(over_limit.work.get(), MAX_OVG_WORK_UNITS);
        assert!(!over_limit.exhausted());
    }

    #[test]
    fn ovg_aggregate_stride_cancellation_keeps_accepted_stage_charge() {
        // ovg_resource.go reserveWork publishes its local charge before
        // route_stage_guard.go add reaches the independent aggregate stride.
        // The current-scope shortcut must still observe that aggregate poll.
        for scoped in [false, true] {
            let aggregate = RouteStageWork::new(4096);
            aggregate.used.store(1022, Ordering::Release);
            let token = crate::CancellationToken::new();
            crate::internal::with_cancellation(Some(&token), || {
                let budget = if scoped {
                    OvgBuildBudget::for_current_scope(Some(aggregate.clone()))
                } else {
                    OvgBuildBudget::with_aggregate(aggregate.clone())
                };
                budget.reserve_work(1).unwrap();
                token.cancel();
                assert!(budget.reserve_work(1).is_err());
                assert_eq!(budget.work.get(), 2);
                assert_eq!(aggregate.used(), 1024);
                assert!(!aggregate.exhausted());
                assert!(!budget.exhausted());
            });
        }
    }
}

#[cfg(test)]
mod overlap_check_tests {
    use super::*;
    use crate::internal::limits::{Context, ContextError};

    struct Caller {
        signal: Option<bool>,
        polls: Cell<usize>,
        fail_at: Cell<usize>,
    }
    impl Context for Caller {
        fn done(&self) -> Option<bool> {
            self.signal
        }
        fn error(&self) -> Option<ContextError> {
            let count = self.polls.get() + 1;
            self.polls.set(count);
            (count == self.fail_at.get()).then_some(ContextError::Cancelled)
        }
    }

    #[test]
    fn empty_overlap_checks_preserve_context_and_stride_contract() {
        for signal in [None, Some(false)] {
            let caller = Caller {
                signal,
                polls: Cell::new(0),
                fail_at: Cell::new(0),
            };
            let work = LocalRouteSearchWork::with_context(10000, &caller).unwrap();
            work.add(123).unwrap();
            caller.polls.set(0);
            let stride_before = work.next_context_check.get();
            work.charge_overlap_checks(0).unwrap();
            assert_eq!(work.used(), 123);
            if signal.is_some() {
                assert_eq!(caller.polls.get(), 0);
                assert_eq!(work.next_context_check.get(), stride_before);
            } else {
                assert_eq!(caller.polls.get(), 11);
                assert_eq!(work.next_context_check.get(), 1147);
            }
        }
    }

    #[test]
    fn overlap_checks_retain_err_only_cancellation_boundaries() {
        for fail_at in 1..=11 {
            let caller = Caller {
                signal: None,
                polls: Cell::new(0),
                fail_at: Cell::new(0),
            };
            let work = LocalRouteSearchWork::with_context(10000, &caller).unwrap();
            caller.polls.set(0);
            caller.fail_at.set(fail_at);
            assert!(work.charge_overlap_checks(0).is_err());
            assert_eq!(caller.polls.get(), fail_at);
            assert_eq!(work.used(), 0);
            assert!(matches!(work.failure(), crate::LayoutError::Cancelled));
        }
        let caller = Caller {
            signal: Some(true),
            polls: Cell::new(0),
            fail_at: Cell::new(0),
        };
        let work = LocalRouteSearchWork::with_context(10000, &caller).unwrap();
        caller.polls.set(0);
        caller.fail_at.set(1);
        assert!(work.charge_overlap_checks(0).is_err());
        assert_eq!(caller.polls.get(), 1);
        assert!(matches!(work.failure(), crate::LayoutError::Cancelled));
    }

    #[test]
    fn nonempty_overlap_checks_preserve_partial_reservations() {
        for signal in [None, Some(false)] {
            for limit in 0..=30 {
                let caller = Caller {
                    signal,
                    polls: Cell::new(0),
                    fail_at: Cell::new(0),
                };
                let work = LocalRouteSearchWork::with_context(limit, &caller).unwrap();
                let result = work.charge_overlap_checks(3);
                assert_eq!(result.is_ok(), limit == 30);
                assert_eq!(work.used(), (limit / 3).min(10) * 3);
            }
        }
    }
}

#[cfg(test)]
mod scoped_ovg_done_tests {
    use super::*;

    #[test]
    fn scoped_presence_keeps_live_zero_stride_and_limit_cancellation() {
        for reservation in [0, 1023, MAX_OVG_WORK_UNITS] {
            let token = crate::CancellationToken::new();
            crate::internal::with_cancellation(Some(&token), || {
                let budget = OvgBuildBudget::for_current_scope(None);
                assert_eq!(budget.current_scope_has_done, Some(true));
                budget.reserve_work(1).unwrap();
                token.cancel();
                assert!(budget.reserve_work(reservation).is_err());
                assert_eq!(budget.work.get(), 1);
                assert!(!budget.exhausted());
            });
        }
        let token = crate::CancellationToken::new();
        crate::internal::with_cancellation(Some(&token), || {
            let budget = OvgBuildBudget::for_current_scope(None);
            token.cancel();
            // Presence is stable, but the status is read at the original
            // stride/explicit-poll boundaries, just as for the old guard.
            budget.reserve_work(1).unwrap();
            assert!(budget.check().is_err());
            assert_eq!(budget.work.get(), 1);
        });
        crate::internal::with_cancellation(None, || {
            let budget = OvgBuildBudget::for_current_scope(None);
            assert_eq!(budget.current_scope_has_done, Some(false));
            budget.reserve_work(1).unwrap();
        });
        // General guards remain usable across distinct caller scopes.
        let budget = OvgBuildBudget::new();
        let token = crate::CancellationToken::new();
        token.cancel();
        crate::internal::with_cancellation(Some(&token), || {
            assert_eq!(budget.current_scope_has_done, None);
            assert!(budget.reserve_work(0).is_err());
        });
    }
}
