// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! D2 v0.9.0 limits/work.go signed work accounting and cancellation order.
use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

/// Maximum work units permitted for one hierarchy or placement optimization.
pub(super) const MAX_OPTIMIZATION_WORK_UNITS: u64 = 250_000_000;
/// limits/work.go aggregate request transaction ceiling.
pub(super) const MAX_TRANSACTION_WORK_UNITS: i64 = 1_000_000_000;
const OPTIMIZATION_CONTEXT_CHECK_STRIDE: u64 = 64;

/// Default engine work ceiling from D2 v0.9.0's limits/work.go.
pub(super) const MAX_ENGINE_WORK_UNITS: i64 = (10_000 + 50_000) * 1_024;

pub(super) trait Context {
    /// Candidate polls cannot mutate the sized search's geometry or topology.
    fn stable_sized_owner(&self) -> bool {
        false
    }

    /// True when polls only read the calling thread's cancellation scope, so
    /// a worker thread running inside a child of that scope observes the same
    /// cancellation without this context crossing threads.
    fn reads_cancellation_scope(&self) -> bool {
        false
    }

    /// None corresponds to Go's nil Done channel; Some tests a cached channel.
    fn done(&self) -> Option<bool>;
    fn error(&self) -> Option<ContextError>;

    /// The request context may already carry a transaction WorkGuard. Its
    /// own original context is checked, independently of this caller context.
    fn retained_transaction_guard_check(&self) -> Option<Result<(), Error>> {
        None
    }

    /// Diagnostic mutation controls borrow the actual owner during the real
    /// optimization poll, before an error return or panic unwinds its caller.
    #[cfg(feature = "diagnostic-traces")]
    fn error_with_owner(&self, _owner: &mut crate::internal::ArenaGraph) -> Option<ContextError> {
        self.error()
    }

    /// Observe an actual occupied-map capture boundary without requesting a
    /// context poll or changing optimizer work. Absent observers are inert.
    #[cfg(feature = "diagnostic-traces")]
    fn occupied_capture_state(
        &self,
        _owner: &crate::internal::ArenaGraph,
        _phase: crate::internal::occupied::OccupiedCapturePhase,
        _point: Option<crate::Point>,
        _node: Option<crate::NodeId>,
        _original: &crate::internal::occupied::OccupiedPointMap,
        _copied: &crate::internal::occupied::OccupiedPointMap,
    ) {
    }

    /// Local diagnostic contexts need not be thread-safe. Production scoring
    /// opts into sharing only after capturing the caller's cancellation token.
    /// Opt in only when compaction polls cannot mutate workspace topology or
    /// geometry. Custom observer contexts retain live traversal/full rollback.
    fn stable_compaction_workspace(&self) -> bool {
        false
    }

    /// Opt in only when packing polls cannot mutate topology or node headers.
    /// Custom contexts keep resolving each visited node from live state.
    fn stable_packing_topology(&self) -> bool {
        false
    }

    fn shared_scoring_context(&self) -> Option<&(dyn Context + Sync)> {
        None
    }

    /// Diagnostic observers receive the actual current transaction view before
    /// a real poll. Ordinary builds contain neither this hook nor its inputs.
    #[cfg(feature = "diagnostic-traces")]
    fn transaction_state(&self, _graph: &crate::internal::ArenaGraph, _site: TransactionPollSite) {}

    #[cfg(feature = "diagnostic-traces")]
    fn transaction_work_used(&self, _used: i64) {}

    #[cfg(feature = "diagnostic-traces")]
    fn label_placement_work_used(&self, _used: i64) {}

    #[cfg(feature = "diagnostic-traces")]
    fn scoring_state(
        &self,
        graph: &crate::internal::ArenaGraph,
        _site: &'static str,
        _node: Option<crate::NodeId>,
        _iteration: Option<usize>,
    ) {
        self.transaction_state(graph, TransactionPollSite::General);
    }

    /// Observes an actual obstruction slice boundary, even when its index
    /// does not trigger the scorer's 64-item cancellation interval.
    #[cfg(feature = "diagnostic-traces")]
    fn scoring_obstruction_set(&self, _set_index: usize) {}

    #[cfg(feature = "diagnostic-traces")]
    fn scoring_obstruction_inventory(
        &self,
        _graph: &crate::internal::ArenaGraph,
        _scoring_node: crate::NodeId,
        _inventory: &crate::internal::OrderedObstructionInventory,
    ) {
    }

    /// Receives the actual borrowed owner and copied container headers used
    /// by the direct scorer, before the source903/907 polls read their slots.
    #[cfg(feature = "diagnostic-traces")]
    fn scoring_owner_obstruction_inventory(
        &self,
        _frame: &crate::internal::OwnerScoreFrame<'_>,
        _scoring_local: crate::NodeId,
        _node_replacement: crate::NodeId,
        _adjacent_replacement: crate::NodeId,
        _sets: &[crate::internal::OwnerObstructionSet],
    ) {
    }
}

#[cfg(feature = "diagnostic-traces")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransactionPollSite {
    General,
    InnerGeometryClone,
    OuterGeometryClone,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContextError {
    Cancelled,
    DeadlineExceeded,
}
impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "context canceled",
            Self::DeadlineExceeded => "context deadline exceeded",
        })
    }
}
impl std::error::Error for ContextError {}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Error {
    Invalid(String),
    /// A source panic represented fallibly inside a per-seed recovery boundary.
    SeedInvariant(&'static str),
    Limit {
        location: String,
        limit: i64,
    },
    Context {
        location: String,
        cause: ContextError,
    },
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::SeedInvariant(message) => f.write_str(message),
            Self::Limit { location, limit } => {
                write!(f, "TALA {location} work exceeds limit {limit}")
            }
            Self::Context { location, cause } if location.is_empty() => cause.fmt(f),
            Self::Context { location, cause } => write!(f, "{location}: {cause}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Context { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

pub(super) struct WorkGuard<'a> {
    context: &'a dyn Context,
    cancellable: bool,
    location: String,
    used: i64,
    limit: i64,
    transaction: Option<Rc<TransactionWork>>,
}

struct TransactionWork {
    used: Cell<i64>,
    limit: i64,
}

thread_local! {
    // Go carries this pointer in the context passed down one seed's pipeline.
    // Placement transactions stay on that seed's calling thread.
    static TRANSACTION_WORK: RefCell<Option<Rc<TransactionWork>>> = const { RefCell::new(None) };
}

/// engine/pipeline.go runAllStages installs one guard before any stage runs.
/// Recursive placement scopes reuse it; independent seed attempts do not.
pub(super) struct AutolayoutTransactionScope(Option<Rc<TransactionWork>>);

impl AutolayoutTransactionScope {
    pub(super) fn enter() -> Self {
        Self::with_limit(MAX_TRANSACTION_WORK_UNITS)
    }

    fn with_limit(limit: i64) -> Self {
        Self(TRANSACTION_WORK.with(|current| {
            let previous = current.borrow().clone();
            if previous.is_none() {
                *current.borrow_mut() = Some(Rc::new(TransactionWork {
                    used: Cell::new(0),
                    limit,
                }));
            }
            previous
        }))
    }
}

impl Drop for AutolayoutTransactionScope {
    fn drop(&mut self) {
        TRANSACTION_WORK.with(|current| *current.borrow_mut() = self.0.take());
    }
}

/// Error returned by the bounded hierarchy/placement optimization guard.
///
/// This is kept separate from [`Error`] because the upstream optimization
/// package exposes a resource-limit sentinel that callers can classify while
/// retaining the operation-specific diagnostic text.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum OptimizationError {
    Invalid(String),
    ResourceLimit {
        location: String,
        detail: OptimizationResourceLimitDetail,
    },
    Context {
        location: String,
        cause: ContextError,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum OptimizationResourceLimitDetail {
    WorkLimit(u64),
    CheckedPlacementCount(usize),
    ScoringArithmeticOverflow,
    PlacementCandidateCount(usize),
    ArithmeticOverflow,
}

impl fmt::Display for OptimizationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::ResourceLimit { location, detail } => {
                f.write_str("TALA optimization resource limit exceeded: ")?;
                match detail {
                    OptimizationResourceLimitDetail::WorkLimit(limit) => {
                        write!(f, "TALA {location} work exceeds limit {limit}")
                    }
                    OptimizationResourceLimitDetail::CheckedPlacementCount(limit) => {
                        write!(
                            f,
                            "TALA {location} checked placement count exceeds limit {limit}"
                        )
                    }
                    OptimizationResourceLimitDetail::PlacementCandidateCount(limit) => {
                        write!(
                            f,
                            "TALA {location} placement candidates exceed limit {limit}"
                        )
                    }
                    OptimizationResourceLimitDetail::ScoringArithmeticOverflow => {
                        write!(f, "{location} scoring work arithmetic overflow")
                    }
                    OptimizationResourceLimitDetail::ArithmeticOverflow => {
                        write!(f, "{location} work arithmetic overflow")
                    }
                }
            }
            Self::Context { location, cause } => write!(f, "{location}: {cause}"),
        }
    }
}

impl std::error::Error for OptimizationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Context { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

/// Work accounting used by the pinned v0.9.0 hierarchy and placement
/// optimizers.
///
/// Charges are unsigned like the Go implementation. A charge is accepted
/// only when it fits both `u64` and the configured limit; rejected charges do
/// not alter [`Self::used`]. Context cancellation is checked at every 64-unit
/// boundary and at explicit zero charges.
pub(super) struct OptimizationWorkGuard<'a> {
    context: &'a dyn Context,
    location: String,
    used: u64,
    limit: u64,
}

impl<'a> OptimizationWorkGuard<'a> {
    pub(super) fn context(&self) -> &'a dyn Context {
        self.context
    }

    pub(super) fn new(
        context: Option<&'a dyn Context>,
        location: &str,
        limit: u64,
    ) -> Result<Self, OptimizationError> {
        let guard = Self::unpolled(context, location, limit)?;
        guard.finish()?;
        Ok(guard)
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn new_with_owner(
        context: Option<&'a dyn Context>,
        location: &str,
        limit: u64,
        owner: &mut crate::internal::ArenaGraph,
    ) -> Result<Self, OptimizationError> {
        let guard = Self::unpolled(context, location, limit)?;
        guard.finish_with_owner(owner)?;
        Ok(guard)
    }

    fn unpolled(
        context: Option<&'a dyn Context>,
        location: &str,
        limit: u64,
    ) -> Result<Self, OptimizationError> {
        let Some(context) = context else {
            return Err(OptimizationError::Invalid(format!(
                "TALA {location} requires a context"
            )));
        };
        Ok(Self {
            context,
            location: location.into(),
            used: 0,
            limit,
        })
    }

    pub(super) fn step(&mut self) -> Result<(), OptimizationError> {
        self.add(1)
    }

    pub(super) fn add(&mut self, amount: u64) -> Result<(), OptimizationError> {
        if self.accept_charge(amount)? {
            self.check()?;
        }
        Ok(())
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn step_with_owner(
        &mut self,
        owner: &mut crate::internal::ArenaGraph,
    ) -> Result<(), OptimizationError> {
        self.add_with_owner(1, owner)
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn add_with_owner(
        &mut self,
        amount: u64,
        owner: &mut crate::internal::ArenaGraph,
    ) -> Result<(), OptimizationError> {
        if self.accept_charge(amount)? {
            self.check_with_owner(owner)?;
        }
        Ok(())
    }

    /// One source accounting path for ordinary and owner-aware dispatch.
    /// True means the accepted charge requires one actual context poll.
    fn accept_charge(&mut self, amount: u64) -> Result<bool, OptimizationError> {
        if amount == 0 {
            return Ok(true);
        }
        let Some(next) = self.used.checked_add(amount) else {
            return Err(self.resource_limit(OptimizationResourceLimitDetail::ArithmeticOverflow));
        };
        if next > self.limit {
            return Err(self.resource_limit(OptimizationResourceLimitDetail::WorkLimit(self.limit)));
        }
        let previous = self.used;
        self.used = next;
        Ok(
            previous / OPTIMIZATION_CONTEXT_CHECK_STRIDE
                != next / OPTIMIZATION_CONTEXT_CHECK_STRIDE,
        )
    }

    pub(super) fn add_product(&mut self, left: u64, right: u64) -> Result<(), OptimizationError> {
        let Some(product) = left.checked_mul(right) else {
            return Err(self.resource_limit(OptimizationResourceLimitDetail::ArithmeticOverflow));
        };
        self.add(product)
    }

    pub(super) fn add_sort(&mut self, length: usize) -> Result<(), OptimizationError> {
        if length <= 1 {
            return self.check();
        }
        // Go's bits.Len64 is fixed-width even when this crate is compiled for
        // wasm32. Deriving the level count from usize silently charged a
        // different amount on 32-bit targets.
        let length = length as u64;
        let comparisons = length
            .checked_mul(u64::BITS as u64 - (length - 1).leading_zeros() as u64)
            .ok_or_else(|| {
                self.resource_limit(OptimizationResourceLimitDetail::ArithmeticOverflow)
            })?;
        self.add(comparisons)
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn add_product_with_owner(
        &mut self,
        left: u64,
        right: u64,
        owner: &mut crate::internal::ArenaGraph,
    ) -> Result<(), OptimizationError> {
        let Some(product) = left.checked_mul(right) else {
            return Err(self.resource_limit(OptimizationResourceLimitDetail::ArithmeticOverflow));
        };
        self.add_with_owner(product, owner)
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn add_sort_with_owner(
        &mut self,
        length: usize,
        owner: &mut crate::internal::ArenaGraph,
    ) -> Result<(), OptimizationError> {
        if length <= 1 {
            return self.check_with_owner(owner);
        }
        let length = length as u64;
        let comparisons = length
            .checked_mul(u64::BITS as u64 - (length - 1).leading_zeros() as u64)
            .ok_or_else(|| {
                self.resource_limit(OptimizationResourceLimitDetail::ArithmeticOverflow)
            })?;
        self.add_with_owner(comparisons, owner)
    }

    pub(super) fn check(&self) -> Result<(), OptimizationError> {
        if let Some(cause) = self.context.error() {
            return Err(OptimizationError::Context {
                location: self.location.clone(),
                cause,
            });
        }
        Ok(())
    }

    pub(super) fn finish(&self) -> Result<(), OptimizationError> {
        self.check()
    }

    #[cfg(feature = "diagnostic-traces")]
    fn check_with_owner(
        &self,
        owner: &mut crate::internal::ArenaGraph,
    ) -> Result<(), OptimizationError> {
        if let Some(cause) = self.context.error_with_owner(owner) {
            return Err(OptimizationError::Context {
                location: self.location.clone(),
                cause,
            });
        }
        Ok(())
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn finish_with_owner(
        &self,
        owner: &mut crate::internal::ArenaGraph,
    ) -> Result<(), OptimizationError> {
        self.check_with_owner(owner)
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe_occupied_capture(
        &self,
        owner: &crate::internal::ArenaGraph,
        phase: crate::internal::occupied::OccupiedCapturePhase,
        point: Option<crate::Point>,
        node: Option<crate::NodeId>,
        original: &crate::internal::occupied::OccupiedPointMap,
        copied: &crate::internal::occupied::OccupiedPointMap,
    ) {
        self.context
            .occupied_capture_state(owner, phase, point, node, original, copied);
    }

    pub(super) fn set_limit(&mut self, limit: u64) {
        self.limit = limit;
    }

    pub(super) fn used(&self) -> u64 {
        self.used
    }

    pub(super) fn location(&self) -> &str {
        &self.location
    }

    fn resource_limit(&self, detail: OptimizationResourceLimitDetail) -> OptimizationError {
        OptimizationError::ResourceLimit {
            location: self.location.clone(),
            detail,
        }
    }
}
impl<'a> WorkGuard<'a> {
    pub(super) fn new(
        context: Option<&'a dyn Context>,
        location: &str,
        limit: i64,
    ) -> Result<Self, Error> {
        let Some(context) = context else {
            return Err(Error::Invalid(format!(
                "TALA {location} requires a context"
            )));
        };
        if limit < 0 {
            return Err(Error::Invalid(format!(
                "TALA {location} work limit must not be negative"
            )));
        }
        let guard = Self {
            context,
            cancellable: context.done().is_some(),
            location: location.into(),
            used: 0,
            limit,
            transaction: None,
        };
        guard.finish()?;
        Ok(guard)
    }

    /// layoutgraph.ensureTransactionWorkGuard retains the enclosing guard's
    /// identity and consumed work instead of starting a fresh stage budget.
    pub(super) fn transaction(
        context: Option<&'a dyn Context>,
        location: &str,
    ) -> Result<Self, Error> {
        let transaction = TRANSACTION_WORK.with(|current| current.borrow().clone());
        let mut guard = Self::new(
            context,
            if transaction.is_some() {
                "AutolayoutTransactions"
            } else {
                location
            },
            transaction
                .as_ref()
                .map_or(MAX_TRANSACTION_WORK_UNITS, |work| work.limit),
        )?;
        guard.transaction = transaction;
        Ok(guard)
    }

    fn set_used(&mut self, used: i64) {
        if let Some(transaction) = &self.transaction {
            transaction.used.set(used);
        } else {
            self.used = used;
        }
    }
    fn stride(&self) -> i64 {
        if self.cancellable { 1024 } else { 64 }
    }
    fn limit_error(&self) -> Error {
        Error::Limit {
            location: self.location.clone(),
            limit: self.limit,
        }
    }
    pub(super) fn step(&mut self) -> Result<(), Error> {
        let used = self.used().wrapping_add(1);
        self.set_used(used);
        if used > self.limit {
            return Err(self.limit_error());
        }
        if used % self.stride() == 0 {
            self.check()?;
        }
        Ok(())
    }
    /// `count` consecutive `step` calls. A range with no stride poll and no
    /// limit failure is one addition; otherwise it steps one at a time.
    pub(super) fn steps(&mut self, count: usize) -> Result<(), Error> {
        let stride = self.stride();
        let used = self.used();
        if let Ok(units) = i64::try_from(count)
            && used >= 0
            && let Some(end) = used.checked_add(units)
            && end <= self.limit
            && end / stride == used / stride
        {
            self.set_used(end);
            return Ok(());
        }
        for _ in 0..count {
            self.step()?;
        }
        Ok(())
    }
    pub(super) fn add(&mut self, units: i64) -> Result<(), Error> {
        if units < 0 {
            return Err(Error::Invalid(format!(
                "TALA {} work charge must not be negative",
                self.location
            )));
        }
        let previous = self.used();
        if units > self.limit || previous > self.limit.wrapping_sub(units) {
            self.set_used(self.limit.wrapping_add(1));
            let stride = self.stride();
            if previous <= self.limit && previous / stride != self.limit / stride {
                self.check()?;
            }
            return Err(self.limit_error());
        }
        let used = previous.wrapping_add(units);
        self.set_used(used);
        let stride = self.stride();
        if used % stride != 0 && previous / stride == used / stride {
            return Ok(());
        }
        self.check()
    }
    pub(super) fn check(&self) -> Result<(), Error> {
        if self.cancellable && self.context.done() != Some(true) {
            return Ok(());
        }
        #[cfg(feature = "diagnostic-traces")]
        self.context.transaction_work_used(self.used());
        if let Some(cause) = self.context.error() {
            return Err(Error::Context {
                location: self.location.clone(),
                cause,
            });
        }
        Ok(())
    }
    pub(super) fn finish(&self) -> Result<(), Error> {
        self.check()
    }
    /// Direct ctx.Err checks in transaction operations do not use the cached
    /// Done-channel fast path or add the work guard's diagnostic prefix.
    pub(super) fn check_context(&self) -> Result<(), Error> {
        self.check_context_with(self.context)
    }

    /// `ContextWithTransactionWorkGuard` preserves the guard's own context;
    /// Commit and placement operations still poll their distinct caller ctx.
    pub(super) fn check_context_with(&self, context: &dyn Context) -> Result<(), Error> {
        #[cfg(feature = "diagnostic-traces")]
        context.transaction_work_used(self.used());
        if let Some(cause) = context.error() {
            return Err(Error::Context {
                location: String::new(),
                cause,
            });
        }
        Ok(())
    }

    pub(super) fn context(&self) -> &'a dyn Context {
        self.context
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe_transaction_graph(
        &self,
        graph: &crate::internal::ArenaGraph,
        site: TransactionPollSite,
    ) {
        self.context.transaction_state(graph, site);
    }
    pub(super) fn set_limit(&mut self, limit: i64) {
        self.limit = limit;
    }
    pub(super) fn used(&self) -> i64 {
        self.transaction
            .as_ref()
            .map_or(self.used, |work| work.used.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::go_rng::GoRng;
    use std::cell::Cell;

    #[test]
    fn autolayout_transactions_share_recursive_work_and_release_the_request() {
        struct Background;
        impl Context for Background {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                None
            }
        }

        // pipeline.go:404 and transaction.go:131-143 retain one request
        // guard, including while a recursive scope opens another transaction.
        {
            let _request = AutolayoutTransactionScope::with_limit(5);
            let mut first = WorkGuard::transaction(Some(&Background), "First").unwrap();
            first.add(2).unwrap();
            {
                let _recursive = AutolayoutTransactionScope::enter();
                let mut second = WorkGuard::transaction(Some(&Background), "Second").unwrap();
                assert_eq!(second.used(), 2);
                second.steps(3).unwrap();
            }
            assert_eq!(first.used(), 5);
            assert_eq!(
                first.step().unwrap_err().to_string(),
                "TALA AutolayoutTransactions work exceeds limit 5"
            );
        }

        // Another seed gets a fresh counter; standalone operations keep their
        // own diagnostic location and budget after the request has returned.
        {
            let _request = AutolayoutTransactionScope::with_limit(5);
            let mut next = WorkGuard::transaction(Some(&Background), "Next").unwrap();
            assert_eq!(next.used(), 0);
            next.add(5).unwrap();
        }
        let standalone = WorkGuard::transaction(Some(&Background), "Standalone").unwrap();
        assert_eq!(standalone.used(), 0);
        assert_eq!(standalone.location, "Standalone");
    }

    #[test]
    fn nested_autolayout_reports_the_enclosing_transaction_limit() {
        use crate::{Graph, LayoutStage, Node, Size, layout_snapshot};

        // The scale-sweep nested/200 witness has 96 containers and 104 leaves.
        // Exercise the same public graph contract with a smaller inventory and
        // an injected budget, as upstream transaction limit tests do.
        let mut graph = Graph::default();
        let mut parent = None;
        for i in 0..4 {
            let mut node = Node::new(
                format!("c{i}"),
                Size {
                    width: 100.0,
                    height: 60.0,
                },
            );
            node.parent = parent;
            parent = Some(graph.add_node(node));
        }
        for i in 0..8 {
            let mut node = Node::new(
                format!("n{i}"),
                Size {
                    width: 100.0,
                    height: 60.0,
                },
            );
            node.parent = parent;
            graph.add_node(node);
        }
        let _request = AutolayoutTransactionScope::with_limit(100);
        let result = layout_snapshot(&graph, 1, LayoutStage::Normalize);
        assert_eq!(
            result.layout_error.unwrap().to_string(),
            "TALA AutolayoutTransactions work exceeds limit 100"
        );
    }

    #[test]
    fn operation_cancellation_does_not_replace_the_guard_context() {
        struct FixedContext(bool);
        impl Context for FixedContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                self.0.then_some(ContextError::Cancelled)
            }
        }
        let background = FixedContext(false);
        let cancelled = FixedContext(true);
        let mut guard = WorkGuard::new(Some(&background), "RetainedGuard", 100).unwrap();
        guard.add(14).unwrap();
        let error = guard.check_context_with(&cancelled).unwrap_err();
        assert_eq!(error.to_string(), "context canceled");
        assert_eq!(guard.used(), 14);
        assert!(guard.finish().is_ok());
    }

    #[test]
    fn operation_background_does_not_suppress_guard_cancellation() {
        struct MutableContext(Cell<bool>);
        impl Context for MutableContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                self.0.get().then_some(ContextError::Cancelled)
            }
        }
        let guarded = MutableContext(Cell::new(false));
        let operation = MutableContext(Cell::new(false));
        let guard = WorkGuard::new(Some(&guarded), "RetainedGuard", 100).unwrap();
        guarded.0.set(true);
        assert!(guard.check_context_with(&operation).is_ok());
        assert_eq!(
            guard.finish().unwrap_err().to_string(),
            "RetainedGuard: context canceled"
        );
    }

    #[test]
    fn direct_context_error_preserves_the_unwrapped_source_cause() {
        struct InconsistentContext;
        impl Context for InconsistentContext {
            fn done(&self) -> Option<bool> {
                Some(false)
            }
            fn error(&self) -> Option<ContextError> {
                Some(ContextError::Cancelled)
            }
        }
        let guard = WorkGuard::new(Some(&InconsistentContext), "Transaction", 10).unwrap();
        assert!(
            guard.finish().is_ok(),
            "the cached Done channel is still open"
        );
        let error = guard.check_context().unwrap_err();
        assert_eq!(error.to_string(), "context canceled");
        assert!(std::error::Error::source(&error).is_some());
        assert_eq!(guard.used(), 0);
    }
    struct TestContext {
        cancellable: bool,
        cancelled: Cell<bool>,
    }
    impl Context for TestContext {
        fn done(&self) -> Option<bool> {
            self.cancellable.then(|| self.cancelled.get())
        }
        fn error(&self) -> Option<ContextError> {
            self.cancelled.get().then_some(ContextError::Cancelled)
        }
    }
    fn context(cancellable: bool) -> TestContext {
        TestContext {
            cancellable,
            cancelled: Cell::new(false),
        }
    }
    #[test]
    fn upstream_limit_and_cancellation_errors() {
        let ctx = context(true);
        let mut g = WorkGuard::new(Some(&ctx), "test", 2).unwrap();
        g.step().unwrap();
        g.step().unwrap();
        assert_eq!(
            g.step().unwrap_err().to_string(),
            "TALA test work exceeds limit 2"
        );
        ctx.cancelled.set(true);
        assert!(matches!(
            WorkGuard::new(Some(&ctx), "canceled test", 2),
            Err(Error::Context {
                cause: ContextError::Cancelled,
                ..
            })
        ));
    }
    #[test]
    fn upstream_zero_negative_and_missing_context() {
        let ctx = context(false);
        assert!(WorkGuard::new(None, "test", 0).is_err());
        assert!(WorkGuard::new(Some(&ctx), "negative test", -1).is_err());
        let mut g = WorkGuard::new(Some(&ctx), "zero test", 0).unwrap();
        assert_eq!(
            g.step().unwrap_err().to_string(),
            "TALA zero test work exceeds limit 0"
        );
    }
    #[test]
    fn upstream_crossed_stride_observes_cancellation() {
        for cancellable in [true, false] {
            let stride = if cancellable { 1024 } else { 64 };
            for limit in [stride, stride + 1] {
                let ctx = context(cancellable);
                let mut g = WorkGuard::new(Some(&ctx), "crossed stride", limit).unwrap();
                g.step().unwrap();
                ctx.cancelled.set(true);
                assert!(matches!(g.add(stride), Err(Error::Context { .. })));
                assert_eq!(g.used(), stride + 1);
            }
        }
    }
    #[test]
    fn upstream_limit_precedes_cancellation_without_accepted_boundary() {
        for cancellable in [true, false] {
            let stride = if cancellable { 1024 } else { 64 };
            for (initial, add) in [(1, stride - 1), (stride - 1, 1), (stride, 0), (1, i64::MAX)] {
                let ctx = context(cancellable);
                let mut g = WorkGuard::new(Some(&ctx), "overflow ordering", stride - 1).unwrap();
                assert_eq!(g.add(initial).is_err(), initial == stride);
                ctx.cancelled.set(true);
                assert!(matches!(g.add(add), Err(Error::Limit { .. })));
                assert_eq!(g.used(), stride);
            }
        }
    }
    #[test]
    fn upstream_zero_charge_boundary_polling_and_finish() {
        for steps in [0, 1] {
            let ctx = context(true);
            let mut g = WorkGuard::new(Some(&ctx), "zero charge", 10000).unwrap();
            for _ in 0..steps {
                g.step().unwrap();
            }
            ctx.cancelled.set(true);
            assert_eq!(g.add(0).is_err(), steps == 0);
            assert_eq!(g.used(), steps);
            assert!(matches!(g.finish(), Err(Error::Context { .. })));
        }
    }
    #[test]
    fn changing_limit_keeps_consumed_work() {
        let ctx = context(false);
        let mut g = WorkGuard::new(Some(&ctx), "test", 5).unwrap();
        g.add(3).unwrap();
        g.set_limit(2);
        assert_eq!(g.used(), 3);
        assert!(matches!(g.add(0), Err(Error::Limit { .. })));
    }

    #[test]
    fn optimization_guard_matches_upstream_overflow_and_limit_classification() {
        let ctx = context(false);
        let mut guard = OptimizationWorkGuard::new(Some(&ctx), "test", u64::MAX).unwrap();
        guard.add(u64::MAX).unwrap();
        assert!(matches!(
            guard.step(),
            Err(OptimizationError::ResourceLimit { .. })
        ));

        let ctx = context(false);
        let mut guard = OptimizationWorkGuard::new(Some(&ctx), "test", u64::MAX).unwrap();
        let error = guard.add_product(u64::MAX, 2).unwrap_err();
        assert_eq!(
            error.to_string(),
            "TALA optimization resource limit exceeded: test work arithmetic overflow"
        );
    }

    #[test]
    fn optimization_guard_charges_sort_bound_and_preserves_limit_updates() {
        let ctx = context(false);
        let mut guard = OptimizationWorkGuard::new(Some(&ctx), "sort", 100).unwrap();
        guard.add_sort(10).unwrap();
        assert_eq!(guard.used(), 40);
        guard.set_limit(20);
        assert_eq!(guard.used(), 40);
        guard.add(0).unwrap();
        let error = guard.step().unwrap_err();
        assert_eq!(
            error.to_string(),
            "TALA optimization resource limit exceeded: TALA sort work exceeds limit 20"
        );
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn optimization_sort_uses_go_bits_len64_for_wide_lengths() {
        let ctx = context(false);
        let length = 1usize << 32;
        let mut guard =
            OptimizationWorkGuard::new(Some(&ctx), "wide-sort", (length as u64) * 32).unwrap();
        guard.add_sort(length).unwrap();
        assert_eq!(guard.used(), (length as u64) * 32);
    }

    #[test]
    fn guarded_shuffle_matches_go_stream_and_charges_one_draw_each() {
        for length in [0, 1, 2, 3, 10, 127, 1024] {
            let mut expected = (0..length).collect::<Vec<_>>();
            let mut actual = expected.clone();
            let mut expected_rng = GoRng::new(991);
            let mut actual_rng = GoRng::new(991);
            expected_rng.shuffle(&mut expected);
            let ctx = context(false);
            let mut guard =
                OptimizationWorkGuard::new(Some(&ctx), "shuffle", MAX_OPTIMIZATION_WORK_UNITS)
                    .unwrap();
            actual_rng.shuffle_guarded(&mut actual, &mut guard).unwrap();
            assert_eq!(actual, expected, "length {length}");
            assert_eq!(actual_rng.draw_count(), expected_rng.draw_count());
            assert_eq!(guard.used(), actual_rng.draw_count());
        }
    }

    #[test]
    fn optimization_guard_polls_cancellation_at_stride_and_zero_charge() {
        let ctx = context(true);
        let mut guard = OptimizationWorkGuard::new(Some(&ctx), "cancel", 1000).unwrap();
        for _ in 0..63 {
            guard.step().unwrap();
        }
        ctx.cancelled.set(true);
        assert!(guard.step().is_err());

        let ctx = context(true);
        let guard = OptimizationWorkGuard::new(Some(&ctx), "cancel-zero", 1000).unwrap();
        ctx.cancelled.set(true);
        assert!(guard.finish().is_err());
    }
}

#[cfg(all(test, feature = "diagnostic-traces"))]
mod owner_aware_guard_contract_tests {
    use super::*;
    use std::cell::Cell;

    fn owner() -> (crate::internal::ArenaGraph, crate::NodeId) {
        let mut input = crate::Graph::default();
        let node = input.add_node(crate::Node::new(
            "ordinary",
            crate::Size {
                width: 10.0,
                height: 20.0,
            },
        ));
        let mut owner = crate::internal::ArenaGraph::from_input(&input);
        owner.initialize_shared_topology();
        owner.set_position(node, crate::Point { x: 1.0, y: 2.0 });
        (owner, node)
    }

    #[test]
    fn owner_aware_constructor_mutates_actual_owner_before_returning_cancellation() {
        struct MutateAndCancel {
            calls: Cell<usize>,
            node: crate::NodeId,
        }
        impl Context for MutateAndCancel {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                panic!("owner poll must not replay ordinary error")
            }
            fn error_with_owner(
                &self,
                owner: &mut crate::internal::ArenaGraph,
            ) -> Option<ContextError> {
                self.calls.set(self.calls.get() + 1);
                owner.set_position(self.node, crate::Point { x: 71.25, y: -13.5 });
                owner.nodes[self.node.0 as usize].rect.size.width = 99.5;
                Some(ContextError::Cancelled)
            }
        }
        let (mut owner, node) = owner();
        let context = MutateAndCancel {
            calls: Cell::new(0),
            node,
        };
        let error = OptimizationWorkGuard::new_with_owner(
            Some(&context),
            "sizelessOptimizer.optimize",
            0,
            &mut owner,
        )
        .err()
        .unwrap();
        assert_eq!(
            error.to_string(),
            "sizelessOptimizer.optimize: context canceled"
        );
        assert_eq!(context.calls.get(), 1);
        assert_eq!(
            owner.position(node),
            Some(crate::Point { x: 71.25, y: -13.5 })
        );
        assert_eq!(owner.nodes[node.0 as usize].rect.size.width, 99.5);
    }

    #[test]
    fn occupied_capture_observation_forwards_actual_aliases_without_polling() {
        use crate::internal::occupied::{OccupiedCapturePhase, OccupiedPointMap};
        use std::cell::RefCell;
        struct Observe {
            polls: Cell<usize>,
            node: crate::NodeId,
            original: OccupiedPointMap,
            phases: RefCell<Vec<(OccupiedCapturePhase, usize, Option<usize>)>>,
        }
        impl Context for Observe {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                self.polls.set(self.polls.get() + 1);
                None
            }
            fn occupied_capture_state(
                &self,
                owner: &crate::internal::ArenaGraph,
                phase: OccupiedCapturePhase,
                point: Option<crate::Point>,
                node: Option<crate::NodeId>,
                original: &OccupiedPointMap,
                copied: &OccupiedPointMap,
            ) {
                assert_eq!(self.polls.get(), 1);
                assert_eq!(
                    owner.position(self.node),
                    Some(crate::Point { x: 1.0, y: 2.0 })
                );
                assert!(original.same_map_allocation(&self.original));
                assert!(!original.same_map_allocation(copied));
                assert!(!copied.is_nil());
                match phase {
                    OccupiedCapturePhase::BeforeStep | OccupiedCapturePhase::AfterCopy => {
                        let point = point.unwrap();
                        assert_eq!(point.x.to_bits(), 0.2f64.to_bits());
                        assert_eq!(point.y.to_bits(), (-0.0f64).to_bits());
                        assert_eq!(node, None); // Actual present map value nil, not a missing boundary row.
                    }
                    OccupiedCapturePhase::Complete => {
                        assert!(point.is_none());
                        assert!(node.is_none());
                    }
                }
                self.phases
                    .borrow_mut()
                    .push((phase, copied.len(), copied.map_address()));
            }
        }
        let (mut owner, node) = owner();
        let original = OccupiedPointMap::new();
        let point = crate::Point { x: 0.2, y: -0.0 };
        original.insert(point, None);
        let context = Observe {
            polls: Cell::new(0),
            node,
            original: original.clone(),
            phases: RefCell::new(Vec::new()),
        };
        let mut guard = OptimizationWorkGuard::new_with_owner(
            Some(&context),
            "occupied-observation",
            64,
            &mut owner,
        )
        .unwrap();
        let snapshot = original.capture_with_owner(&mut guard, &mut owner).unwrap();
        let phases = context.phases.borrow();
        assert_eq!(
            phases.iter().map(|row| (row.0, row.1)).collect::<Vec<_>>(),
            [
                (OccupiedCapturePhase::BeforeStep, 0),
                (OccupiedCapturePhase::AfterCopy, 1),
                (OccupiedCapturePhase::Complete, 1)
            ]
        );
        assert!(phases[0].2.is_some());
        assert!(phases.iter().all(|row| row.2 == phases[0].2)); // One actual copied allocation throughout.
        assert_eq!(guard.used(), 1);
        assert_eq!(context.polls.get(), 1); // Boundary observation adds no real Err poll.
        assert_eq!(original.lookup(point), Some(None));
        assert_eq!(snapshot.observed_entries().len(), 1);
    }

    #[test]
    fn owner_aware_guard_shares_zero_stride_limit_and_overflow_accounting() {
        struct Count(Cell<usize>);
        impl Context for Count {
            fn done(&self) -> Option<bool> {
                None
            }
            fn error(&self) -> Option<ContextError> {
                self.0.set(self.0.get() + 1);
                None
            }
        }
        let (mut owner, _) = owner();
        let context = Count(Cell::new(0));
        let mut guard =
            OptimizationWorkGuard::new_with_owner(Some(&context), "owner-work", 64, &mut owner)
                .unwrap();
        assert_eq!(context.0.get(), 1); // Default dispatch calls ordinary error once.
        guard.add_with_owner(63, &mut owner).unwrap();
        assert_eq!(guard.used(), 63);
        assert_eq!(context.0.get(), 1);
        guard.step_with_owner(&mut owner).unwrap();
        assert_eq!(guard.used(), 64);
        assert_eq!(context.0.get(), 2);
        let error = guard.step_with_owner(&mut owner).unwrap_err();
        assert_eq!(
            error.to_string(),
            "TALA optimization resource limit exceeded: TALA owner-work work exceeds limit 64"
        );
        assert_eq!(guard.used(), 64);
        assert_eq!(context.0.get(), 2); // Rejection has no poll and no accepted charge.
        guard.add_with_owner(0, &mut owner).unwrap();
        assert_eq!(guard.used(), 64);
        assert_eq!(context.0.get(), 3);
        guard.finish_with_owner(&mut owner).unwrap();
        assert_eq!(context.0.get(), 4);

        let context = Count(Cell::new(0));
        let mut guard = OptimizationWorkGuard::new_with_owner(
            Some(&context),
            "owner-overflow",
            u64::MAX,
            &mut owner,
        )
        .unwrap();
        guard.add_with_owner(u64::MAX, &mut owner).unwrap();
        assert_eq!(context.0.get(), 2); // One check even across many stride boundaries.
        let error = guard.step_with_owner(&mut owner).unwrap_err();
        assert_eq!(
            error.to_string(),
            "TALA optimization resource limit exceeded: owner-overflow work arithmetic overflow"
        );
        assert_eq!(guard.used(), u64::MAX);
        assert_eq!(context.0.get(), 2);
    }
}
