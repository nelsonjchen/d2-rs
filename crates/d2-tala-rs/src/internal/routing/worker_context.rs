// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! The child context owned by coordinator.generateRouteFlavorResponsesWith.
use crate::internal::limits::{Context, ContextError};
use std::cell::RefCell;

pub(super) struct WorkerContext<'a> {
    parent: &'a dyn Context,
    parent_has_done: bool,
    cause: RefCell<Option<ContextError>>,
}
impl<'a> WorkerContext<'a> {
    pub(super) fn new(parent: &'a dyn Context) -> Self {
        let context = Self {
            parent,
            parent_has_done: parent.done().is_some(),
            cause: RefCell::new(None),
        };
        context.propagate();
        context
    }
    fn propagate(&self) {
        if self.cause.borrow().is_none() && self.parent_has_done && self.parent.done() == Some(true)
        {
            // context.WithCancel rejects a closed Done channel with no cause.
            let error = self.parent.error();
            // Go 1.27 evaluates both parent.Err() and Cause(parent) before
            // cancel; Cause itself calls Err even when the first result is nil.
            let _cause = self.parent.error();
            let cause = error.expect("context: internal error: missing cancel error");
            *self.cause.borrow_mut() = Some(cause);
        }
    }
    pub(super) fn cancel(&self) {
        if self.cause.borrow().is_none() {
            *self.cause.borrow_mut() = Some(ContextError::Cancelled);
        }
    }
}
impl Context for WorkerContext<'_> {
    fn done(&self) -> Option<bool> {
        self.propagate();
        Some(self.cause.borrow().is_some())
    }
    fn error(&self) -> Option<ContextError> {
        self.propagate();
        self.cause.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct Parent {
        done: Cell<Option<bool>>,
        cause: Option<ContextError>,
        polls: Cell<usize>,
    }
    impl Context for Parent {
        fn done(&self) -> Option<bool> {
            self.done.get()
        }
        fn error(&self) -> Option<ContextError> {
            self.polls.set(self.polls.get() + 1);
            self.cause.clone()
        }
    }
    #[test]
    fn child_ignores_err_without_done_and_rebinding_restores_caller() {
        let parent = Parent {
            done: Cell::new(None),
            cause: None,
            polls: Cell::new(0),
        };
        let child = WorkerContext::new(&parent);
        let work =
            super::super::route_search_guard::LocalRouteSearchWork::with_context(10, &parent)
                .unwrap();
        work.step().unwrap();
        assert_eq!(parent.polls.get(), 2);
        work.bind(&child).unwrap();
        work.step().unwrap();
        work.finish().unwrap();
        assert_eq!(parent.polls.get(), 2);
        assert_eq!(work.used(), 2);
        work.bind(&parent).unwrap();
        assert_eq!(parent.polls.get(), 3);
        assert_eq!(work.used(), 2);
    }
    #[test]
    fn closed_parent_cause_is_latched_and_sibling_cancel_does_not_override_it() {
        let parent = Parent {
            done: Cell::new(Some(false)),
            cause: Some(ContextError::DeadlineExceeded),
            polls: Cell::new(0),
        };
        let child = WorkerContext::new(&parent);
        assert_eq!(child.error(), None);
        assert_eq!(parent.polls.get(), 0);
        parent.done.set(Some(true));
        assert_eq!(child.done(), Some(true));
        child.cancel();
        assert_eq!(child.error(), Some(ContextError::DeadlineExceeded));
        assert_eq!(parent.polls.get(), 2);
    }
    #[test]
    fn derivation_observes_cause_before_rejecting_missing_error() {
        let parent = Parent {
            done: Cell::new(Some(true)),
            cause: None,
            polls: Cell::new(0),
        };
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| WorkerContext::new(&parent)));
        assert!(result.is_err());
        assert_eq!(parent.polls.get(), 2);
    }
    #[test]
    #[should_panic(expected = "context: internal error: missing cancel error")]
    fn closed_parent_without_error_panics_at_derivation() {
        let parent = Parent {
            done: Cell::new(Some(true)),
            cause: None,
            polls: Cell::new(0),
        };
        WorkerContext::new(&parent);
    }
}
