// SPDX-License-Identifier: MPL-2.0
//! transaction.go::NewRequestTransaction, CloneGeometryContext and UpdateState.
use super::*;
use std::cell::{Ref, RefCell};
use std::rc::Rc;
type Pairs = BTreeSet<(NodeId, NodeId)>;

#[derive(Clone, Copy, Default)]
pub(in crate::internal) struct GeometryTransactionOptions {
    pub(in crate::internal) affect_containers: bool,
    pub(in crate::internal) ignore_container_escape: bool,
    pub(in crate::internal) affect_edge_routes: bool,
}
impl GeometryTransactionOptions {
    pub(in crate::internal) fn packing(affect_edge_routes: bool) -> Self {
        Self {
            ignore_container_escape: true,
            affect_edge_routes,
            ..Self::default()
        }
    }
}

pub(in crate::internal) struct PackingGeometryTransaction<'op> {
    prior: Rc<RefCell<PackingTrialSnapshot>>,
    spare: Option<Rc<RefCell<PackingTrialSnapshot>>>,
    prior_shared: bool,
    options: GeometryTransactionOptions,
    overlaps: Rc<(Pairs, Pairs)>,
    operations: PackingOperations<'op>,
    placement_costs: Option<PlacementCostSnapshot>,
}
impl<'op> PackingGeometryTransaction<'op> {
    pub(in crate::internal) fn capture(
        graph: &ArenaGraph,
        capture_edge_routes: bool,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Self, limits::Error> {
        Self::capture_with_options(
            graph,
            GeometryTransactionOptions::packing(capture_edge_routes),
            guard,
        )
    }
    pub(in crate::internal) fn capture_with_options(
        graph: &ArenaGraph,
        options: GeometryTransactionOptions,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Self, limits::Error> {
        let prior = PackingTrialSnapshot::capture(graph, options.affect_edge_routes, guard)?;
        let overlaps = graph.transaction_overlap_snapshot_checked(guard)?;
        guard.finish()?;
        Ok(Self {
            prior: Rc::new(RefCell::new(prior)),
            spare: None,
            prior_shared: false,
            options,
            overlaps: Rc::new(overlaps),
            operations: PackingOperations::new(),
            placement_costs: None,
        })
    }
    pub(in crate::internal) fn snapshot(&self) -> Ref<'_, PackingTrialSnapshot> {
        self.prior.borrow()
    }
    pub(in crate::internal) fn overlaps(&self) -> (&Pairs, &Pairs) {
        (&self.overlaps.0, &self.overlaps.1)
    }
    pub(in crate::internal) fn operations(&self) -> PackingOperations<'op> {
        self.operations.clone()
    }
    pub(in crate::internal) fn clone_geometry(
        &mut self,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<Self, limits::Error> {
        guard.finish()?;
        self.prior_shared = true;
        // The stage-owned placement-cost snapshot is deliberately not cloned.
        Ok(Self {
            prior: Rc::clone(&self.prior),
            spare: None,
            prior_shared: true,
            options: self.options,
            overlaps: Rc::clone(&self.overlaps),
            operations: self.operations.copy_visible(),
            placement_costs: None,
        })
    }
    pub(in crate::internal) fn update(
        &mut self,
        graph: &mut ArenaGraph,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        let updated = match PackingTrialSnapshot::capture_refresh(
            graph,
            self.options.affect_edge_routes,
            guard,
        ) {
            Ok(updated) => updated,
            Err(error) => {
                self.rollback(graph);
                return Err(error);
            }
        };
        // Published snapshots are never modified by refresh. A private spare
        // retains its identity across reuse; shared prior state is not recycled.
        let destination = if let Some(spare) = self.spare.take() {
            *spare.borrow_mut() = updated;
            spare
        } else {
            Rc::new(RefCell::new(updated))
        };
        let previous = std::mem::replace(&mut self.prior, destination);
        self.spare = if self.prior_shared {
            None
        } else {
            Some(previous)
        };
        self.prior_shared = false;
        // Original overlap exceptions remain anchored to request construction.
        Ok(())
    }
    pub(in crate::internal) fn rollback(&self, graph: &mut ArenaGraph) {
        self.prior.borrow().state.restore(graph);
    }
    pub(in crate::internal) fn capture_placement_costs(
        &mut self,
        graph: &ArenaGraph,
        location: &str,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<(), limits::Error> {
        gap_guard::capture_placement_costs_once(
            &mut self.placement_costs,
            location,
            guard,
            || {
                graph
                    .owner_graph_edge_length_cache_entries(OwnerGraphId(0))
                    .expect("initialized request cost owner")
            },
            || {
                graph
                    .owner_graph_snapshot_placement_costs(OwnerGraphId(0))
                    .expect("initialized request cost owner")
            },
        )
    }
    pub(in crate::internal) fn restore_placement_costs(&mut self) -> bool {
        let Some(costs) = self.placement_costs.take() else {
            return false;
        };
        costs.restore();
        true
    }
    #[cfg(test)]
    pub(in crate::internal) fn cost_snapshot(&self) -> Option<&PlacementCostSnapshot> {
        self.placement_costs.as_ref()
    }
    pub(in crate::internal) fn commit(
        &self,
        graph: &mut ArenaGraph,
        guard: &mut limits::WorkGuard<'_>,
    ) -> Result<bool, limits::Error> {
        let prior = self.prior.borrow();
        let mut trial = prior.trial(graph);
        let accepted = trial.apply_queued_transaction_commit_checked(
            &self.overlaps.0,
            &self.overlaps.1,
            guard,
            &self.operations,
            self.options,
        )?;
        if accepted {
            trial.accept();
        }
        Ok(accepted)
    }
    #[cfg(test)]
    pub(in crate::internal) fn checkpoint_identity(&self) -> usize {
        Rc::as_ptr(&self.prior) as usize
    }
}
