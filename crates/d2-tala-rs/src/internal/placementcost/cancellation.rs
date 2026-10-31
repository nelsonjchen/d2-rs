// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! D2 v0.9.0 placementcost/geometry.go cancellation and its 64-item interval.

use super::limits::{Context, Error};

#[derive(Clone, Copy)]
pub(super) struct ScoreContext<'a> {
    context: Option<&'a dyn Context>,
    preparation_polls: Option<&'a std::cell::Cell<usize>>,
}

impl<'a> ScoreContext<'a> {
    pub(super) fn new(context: &'a dyn Context) -> Self {
        Self {
            context: Some(context),
            preparation_polls: None,
        }
    }

    /// Legacy numeric callers have not yet acquired an operation context.
    /// Their wrappers reuse the same kernel without constructing diagnostics.
    pub(super) fn none() -> Self {
        Self {
            context: None,
            preparation_polls: None,
        }
    }

    pub(super) fn counted<'b>(self, polls: &'b std::cell::Cell<usize>) -> ScoreContext<'b>
    where
        'a: 'b,
    {
        ScoreContext {
            context: self.context,
            preparation_polls: Some(polls),
        }
    }

    /// Polls through this context cannot mutate the owner being scored:
    /// no context at all, or one with `stable_sized_owner()`.
    pub(super) fn stable_owner(self) -> bool {
        self.context.is_none_or(Context::stable_sized_owner)
    }

    fn count_poll(self) {
        if let Some(polls) = self.preparation_polls {
            polls.set(polls.get() + 1);
        }
    }

    #[inline]
    pub(super) fn check(self) -> Result<(), Error> {
        self.count_poll();
        if let Some(cause) = self.context.and_then(Context::error) {
            return Err(Error::Context {
                location: "EdgeLength".into(),
                cause,
            });
        }
        Ok(())
    }

    /// Exclusive source callback dispatch, with no frame or field lock held.
    /// Diagnostic owner access is compiled out of ordinary builds.
    pub(super) fn check_with_owner(self, owner: &mut super::ArenaGraph) -> Result<(), Error> {
        self.count_poll();
        #[cfg(feature = "diagnostic-traces")]
        let cause = self
            .context
            .and_then(|context| context.error_with_owner(owner));
        #[cfg(not(feature = "diagnostic-traces"))]
        let cause = {
            let _ = owner;
            self.context.and_then(Context::error)
        };
        if let Some(cause) = cause {
            return Err(Error::Context {
                location: "EdgeLength".into(),
                cause,
            });
        }
        Ok(())
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe(
        self,
        graph: &super::ArenaGraph,
        site: &'static str,
        node: Option<crate::NodeId>,
        iteration: Option<usize>,
    ) {
        if let Some(context) = self.context {
            context.scoring_state(graph, site, node, iteration);
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe_obstruction_set(self, set_index: usize) {
        if let Some(context) = self.context {
            context.scoring_obstruction_set(set_index);
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe_obstruction_inventory(
        self,
        graph: &super::ArenaGraph,
        scoring_node: crate::NodeId,
        inventory: &super::OrderedObstructionInventory,
    ) {
        if let Some(context) = self.context {
            context.scoring_obstruction_inventory(graph, scoring_node, inventory);
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn observe_owner_obstruction_inventory(
        self,
        frame: &super::OwnerScoreFrame<'_>,
        scoring_local: crate::NodeId,
        node_replacement: crate::NodeId,
        adjacent_replacement: crate::NodeId,
        sets: &[super::OwnerObstructionSet],
    ) {
        if let Some(context) = self.context {
            context.scoring_owner_obstruction_inventory(
                frame,
                scoring_local,
                node_replacement,
                adjacent_replacement,
                sets,
            );
        }
    }
}

// All diagnostic arguments disappear at compile time. Every real error poll
// keeps the same EdgeLength prefix, including polling on a nil Done channel.
macro_rules! score_poll {
    ($context:expr, $graph:expr, $site:expr, $node:expr, $iteration:expr) => {{
        #[cfg(feature = "diagnostic-traces")]
        $context.observe($graph, $site, $node, $iteration);
        $context.check()
    }};
}
pub(super) use score_poll;

macro_rules! score_interval {
    ($context:expr, $graph:expr, $site:expr, $node:expr, $iteration:expr) => {{
        if $iteration % 64 == 0 {
            $crate::internal::scoring_context::score_poll!(
                $context,
                $graph,
                $site,
                $node,
                Some($iteration)
            )
        } else {
            Ok(())
        }
    }};
}
pub(super) use score_interval;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct LocalContext {
        polls: Cell<usize>,
        canceled: Cell<bool>,
    }

    impl Context for LocalContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<super::super::limits::ContextError> {
            self.polls.set(self.polls.get() + 1);
            self.canceled
                .get()
                .then_some(super::super::limits::ContextError::Cancelled)
        }
    }

    #[test]
    fn source_interval_checks_zero_and_each_sixty_fourth_item() {
        let context = LocalContext {
            polls: Cell::new(0),
            canceled: Cell::new(false),
        };
        let checked = ScoreContext::new(&context);
        let graph = super::super::ArenaGraph::from_input(&crate::Graph::default());
        for index in 0..130 {
            score_interval!(checked, &graph, "interval-test", None, index).unwrap();
        }
        assert_eq!(context.polls.get(), 3);
        context.canceled.set(true);
        assert_eq!(
            score_interval!(checked, &graph, "interval-test", None, 131),
            Ok(())
        );
        assert_eq!(context.polls.get(), 3);
        assert_eq!(
            score_interval!(checked, &graph, "interval-test", None, 192)
                .unwrap_err()
                .to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(context.polls.get(), 4);
    }

    #[test]
    fn nil_done_is_polled_and_local_context_is_not_shared() {
        let context = LocalContext {
            polls: Cell::new(0),
            canceled: Cell::new(true),
        };
        assert!(context.shared_scoring_context().is_none());
        assert_eq!(
            ScoreContext::new(&context).check().unwrap_err().to_string(),
            "EdgeLength: context canceled"
        );
        assert_eq!(context.polls.get(), 1);
        assert_eq!(ScoreContext::none().check(), Ok(()));
    }
}
