// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! sizeless_optimizer.go106–158: actual owner rollback arms only after both
//! inert captures succeed. The caller supplies the optimizer's guarded body.
//! Constructors, nonempty worklists and shared Point/group objects still need
//! their own source bindings; this wrapper does not qualify those domains.

use super::limits::{Context, OptimizationError, OptimizationWorkGuard};
use super::occupied::{OccupiedPointMap, OccupiedPointSnapshot};
use super::owner_score_session::OwnerOptimizerMutationSnapshot;
use super::{ArenaGraph, OwnerGraphId, OwnerScoreSession};
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

trait OwnerOptimizerReceiver {
    fn owner_mut(&mut self) -> &mut ArenaGraph;
    fn receiver(&self) -> OwnerGraphId;
}

impl OwnerOptimizerReceiver for OwnerScoreSession<'_> {
    fn owner_mut(&mut self) -> &mut ArenaGraph {
        OwnerScoreSession::owner_mut(self)
    }
    fn receiver(&self) -> OwnerGraphId {
        self.current_view()
    }
}

struct OwnerGraphOptimizerReceiver<'a> {
    owner: &'a mut ArenaGraph,
    receiver: OwnerGraphId,
}

impl OwnerOptimizerReceiver for OwnerGraphOptimizerReceiver<'_> {
    fn owner_mut(&mut self) -> &mut ArenaGraph {
        self.owner
    }
    fn receiver(&self) -> OwnerGraphId {
        self.receiver
    }
}

fn restore_pass(
    session: &mut impl OwnerOptimizerReceiver,
    occupied: &RefCell<OccupiedPointMap>,
    scratch: &OwnerOptimizerMutationSnapshot,
    occupied_snapshot: &OccupiedPointSnapshot,
) -> Result<(), OptimizationError> {
    let graph_result = scratch.restore(session.owner_mut());
    occupied_snapshot.restore(&mut occupied.borrow_mut());
    graph_result
}

/// General native transaction over the actual selected graph and actual
/// occupied field. No occupied-field borrow or backing lock crosses a poll.
/// Geometry/cost/map restoration never restores the caller's RNG stream.
pub(super) fn with_owner_optimizer_mutation<T>(
    session: &mut OwnerScoreSession<'_>,
    occupied: &RefCell<OccupiedPointMap>,
    scratch: &mut OwnerOptimizerMutationSnapshot,
    context: &dyn Context,
    limit: u64,
    body: impl FnOnce(
        &mut OwnerScoreSession<'_>,
        &RefCell<OccupiedPointMap>,
        &mut OptimizationWorkGuard<'_>,
    ) -> Result<T, OptimizationError>,
) -> Result<T, OptimizationError> {
    with_optimizer_mutation_receiver(session, occupied, scratch, context, limit, body)
}

/// Source capture operates on Graph itself, independently of the dense-local
/// score mapping. Duplicate Graph.Nodes members retain their charged walk.
pub(super) fn with_owner_graph_optimizer_mutation<T>(
    owner: &mut ArenaGraph,
    receiver: OwnerGraphId,
    occupied: &RefCell<OccupiedPointMap>,
    scratch: &mut OwnerOptimizerMutationSnapshot,
    context: &dyn Context,
    limit: u64,
    body: impl FnOnce(
        &mut ArenaGraph,
        &RefCell<OccupiedPointMap>,
        &mut OptimizationWorkGuard<'_>,
    ) -> Result<T, OptimizationError>,
) -> Result<T, OptimizationError> {
    with_optimizer_mutation_receiver(
        &mut OwnerGraphOptimizerReceiver { owner, receiver },
        occupied,
        scratch,
        context,
        limit,
        |selected, occupied, guard| body(selected.owner_mut(), occupied, guard),
    )
}

fn with_optimizer_mutation_receiver<T, Receiver: OwnerOptimizerReceiver>(
    session: &mut Receiver,
    occupied: &RefCell<OccupiedPointMap>,
    scratch: &mut OwnerOptimizerMutationSnapshot,
    context: &dyn Context,
    limit: u64,
    body: impl FnOnce(
        &mut Receiver,
        &RefCell<OccupiedPointMap>,
        &mut OptimizationWorkGuard<'_>,
    ) -> Result<T, OptimizationError>,
) -> Result<T, OptimizationError> {
    // Source registers scratch.release only after NewGuard and typed optimizer
    // validation. An initial Context error/panic leaves prior scratch intact.
    #[cfg(feature = "diagnostic-traces")]
    let mut guard = OptimizationWorkGuard::new_with_owner(
        Some(context),
        "sizelessOptimizer.optimize",
        limit,
        session.owner_mut(),
    )?;
    #[cfg(not(feature = "diagnostic-traces"))]
    let mut guard = OptimizationWorkGuard::new(Some(context), "sizelessOptimizer.optimize", limit)?;

    let operation = catch_unwind(AssertUnwindSafe(|| {
        let receiver = session.receiver();
        scratch.capture_into(session.owner_mut(), receiver, &mut guard)?;
        let occupied_ref = occupied.borrow().clone();
        #[cfg(feature = "diagnostic-traces")]
        let occupied_snapshot = occupied_ref.capture_with_owner(&mut guard, session.owner_mut())?;
        #[cfg(not(feature = "diagnostic-traces"))]
        let occupied_snapshot = occupied_ref.capture(&mut guard)?;

        // Only this successful pair of captures arms the error/panic handler.
        let armed = catch_unwind(AssertUnwindSafe(|| {
            let value = body(session, occupied, &mut guard)?;
            #[cfg(feature = "diagnostic-traces")]
            guard.finish_with_owner(session.owner_mut())?;
            #[cfg(not(feature = "diagnostic-traces"))]
            guard.finish()?;
            Ok(value)
        }));
        match armed {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => {
                restore_pass(session, occupied, scratch, &occupied_snapshot)?;
                Err(error)
            }
            Err(payload) => {
                // Replacing the arena or deleting its stable nodes is outside
                // this binding. Report that unsupported restoration explicitly.
                restore_pass(session, occupied, scratch, &occupied_snapshot)
                    .expect("owner optimizer panic restore requires the original arena nodes");
                resume_unwind(payload)
            }
        }
    }));
    scratch.release();
    match operation {
        Ok(result) => result,
        Err(payload) => resume_unwind(payload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::go_rng::GoRng;
    use crate::internal::limits::ContextError;
    use crate::internal::{ArenaGraph, CanonicalScoreMapping, OwnerGraphId};
    use crate::{Graph, Node, NodeId, Point, Size};
    use std::cell::Cell;
    use std::sync::Arc;

    struct QuietContext;
    impl Context for QuietContext {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            None
        }
    }

    fn fixture(empty: bool) -> (ArenaGraph, CanonicalScoreMapping) {
        let mut graph = Graph::default();
        if !empty {
            for name in ["first", "second"] {
                graph.add_node(Node::new(
                    name,
                    Size {
                        width: 10.0,
                        height: 20.0,
                    },
                ));
            }
        }
        let mut owner = ArenaGraph::from_input(&graph);
        owner.containers.clear();
        for (index, node) in owner.nodes.iter_mut().enumerate() {
            node.position = Some(Point {
                x: index as f64 + 0.2,
                y: index as f64 + 0.4,
            });
            node.rect.origin = node.position.unwrap();
        }
        owner.initialize_shared_topology();
        let mapping = CanonicalScoreMapping::new(
            owner.topology_arena_id().unwrap(),
            owner.node_order.clone(),
            owner.edge_order.read(),
        );
        (owner, mapping)
    }

    #[test]
    fn armed_owner_pass_error_restores_geometry_cost_and_original_occupied_alias() {
        let (mut owner, mapping) = fixture(false);
        let node = NodeId(0);
        let before = owner.position(node).unwrap();
        let costs = owner.cost_cache.clone();
        costs.store_edge_length(7, 7.25);
        costs.restore_routing((3.0, 5.0, 7.0));
        let old_cache = costs.edge_length_cache_alias().unwrap();
        let original = OccupiedPointMap::new();
        original.insert(before, Some(node));
        let occupied = RefCell::new(original.clone());
        let replacement = OccupiedPointMap::new();
        replacement.insert(Point { x: 99.0, y: 100.0 }, None);
        let mut scratch = OwnerOptimizerMutationSnapshot::default();
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let result = with_owner_optimizer_mutation(
            &mut session,
            &occupied,
            &mut scratch,
            &QuietContext,
            100,
            |session, occupied, _guard| {
                session
                    .owner_mut()
                    .set_position(node, Point { x: 77.0, y: 88.0 });
                session.owner_mut().nodes[0].rect.size.width = 123.0;
                costs.publish_deserialized_routing((30.0, 50.0, 70.0));
                costs.store_edge_length(9, 9.25);
                old_cache.store(11, 11.25);
                original.insert(Point { x: 1.2, y: 1.4 }, None);
                *occupied.borrow_mut() = replacement.clone();
                Err::<(), _>(OptimizationError::Invalid("authored body error".into()))
            },
        );
        assert_eq!(result.unwrap_err().to_string(), "authored body error");
        assert_eq!(session.owner().position(node), Some(before));
        assert_eq!(session.owner().nodes[0].rect.size.width, 10.0);
        assert_eq!(costs.raw(), (3.0, 5.0, 7.0));
        assert_eq!(costs.lookup_edge_length(7), Some(7.25));
        assert_eq!(costs.lookup_edge_length(9), None);
        assert_eq!(old_cache.lookup(11), None);
        assert!(old_cache.is_current());
        assert!(occupied.borrow().same_map_allocation(&original));
        assert_eq!(original.len(), 1);
        assert_eq!(original.lookup(before), Some(Some(node)));
        assert_eq!(replacement.lookup(Point { x: 99.0, y: 100.0 }), Some(None));
        assert!(!scratch.is_complete());
        assert_eq!(scratch.captured_nodes().len(), 0);
    }

    #[test]
    fn armed_owner_pass_rethrows_exact_panic_and_keeps_consumed_rng() {
        let (mut owner, mapping) = fixture(false);
        let before = owner.position(NodeId(0));
        let original = OccupiedPointMap::default();
        let occupied = RefCell::new(original.clone());
        let mut scratch = OwnerOptimizerMutationSnapshot::default();
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let mut rng = GoRng::new_untraced(1729);
        let entry_draws = rng.draw_count();
        let payload = Arc::new(String::from("actual body panic"));
        let run = catch_unwind(AssertUnwindSafe(|| {
            with_owner_optimizer_mutation(
                &mut session,
                &occupied,
                &mut scratch,
                &QuietContext,
                100,
                |session, occupied, _guard| {
                    session
                        .owner_mut()
                        .set_position(NodeId(0), Point { x: 77.0, y: 88.0 });
                    *occupied.borrow_mut() = OccupiedPointMap::new();
                    let _ = rng.float64();
                    std::panic::panic_any(Arc::clone(&payload));
                    #[allow(unreachable_code)]
                    Ok::<(), OptimizationError>(())
                },
            )
        }));
        let actual = run.unwrap_err();
        assert!(Arc::ptr_eq(
            actual.downcast_ref::<Arc<String>>().unwrap(),
            &payload
        ));
        assert_eq!(session.owner().position(NodeId(0)), before);
        assert!(occupied.borrow().is_nil());
        assert_eq!(rng.draw_count(), entry_draws + 1);
        assert!(!scratch.is_complete());
        assert_eq!(scratch.captured_nodes().len(), 0);
    }

    #[test]
    fn initial_guard_error_and_panic_leave_prior_scratch_and_mutations_intact() {
        struct InitialContext {
            panic_payload: Option<Arc<String>>,
        }
        impl Context for InitialContext {
            fn done(&self) -> Option<bool> {
                Some(true)
            }
            fn error(&self) -> Option<ContextError> {
                if let Some(payload) = &self.panic_payload {
                    std::panic::panic_any(Arc::clone(payload));
                }
                Some(ContextError::Cancelled)
            }
        }
        let (mut owner, mapping) = fixture(false);
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let mut scratch = OwnerOptimizerMutationSnapshot::default();
        let mut guard =
            OptimizationWorkGuard::new(Some(&QuietContext), "sizelessOptimizer.optimize", 100)
                .unwrap();
        session
            .capture_optimizer_mutation_state_into(&mut guard, &mut scratch)
            .unwrap();
        let prior_nodes = scratch.captured_nodes().collect::<Vec<_>>();
        let changed = Point { x: 70.0, y: 80.0 };
        session.owner_mut().set_position(NodeId(0), changed);
        let original = OccupiedPointMap::new();
        original.insert(changed, None);
        let occupied = RefCell::new(original.clone());
        let called = Cell::new(false);
        let error = with_owner_optimizer_mutation(
            &mut session,
            &occupied,
            &mut scratch,
            &InitialContext {
                panic_payload: None,
            },
            100,
            |_session, _occupied, _guard| {
                called.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "sizelessOptimizer.optimize: context canceled"
        );
        assert!(!called.get());
        assert!(scratch.is_complete());
        assert_eq!(scratch.captured_nodes().collect::<Vec<_>>(), prior_nodes);
        assert_eq!(session.owner().position(NodeId(0)), Some(changed));
        let payload = Arc::new(String::from("initial actual Context panic"));
        let run = catch_unwind(AssertUnwindSafe(|| {
            with_owner_optimizer_mutation(
                &mut session,
                &occupied,
                &mut scratch,
                &InitialContext {
                    panic_payload: Some(Arc::clone(&payload)),
                },
                100,
                |_session, _occupied, _guard| Ok(()),
            )
        }));
        let actual = run.unwrap_err();
        assert!(Arc::ptr_eq(
            actual.downcast_ref::<Arc<String>>().unwrap(),
            &payload
        ));
        assert!(scratch.is_complete());
        assert_eq!(scratch.captured_nodes().collect::<Vec<_>>(), prior_nodes);
        assert_eq!(session.owner().position(NodeId(0)), Some(changed));
        assert!(occupied.borrow().same_map_allocation(&original));
        assert_eq!(original.lookup(changed), Some(None));
    }

    #[test]
    fn occupied_copy_error_is_unarmed_and_preserves_actual_header_replacement() {
        struct MutatingContext<'a> {
            polls: Cell<usize>,
            occupied: &'a RefCell<OccupiedPointMap>,
            original: OccupiedPointMap,
            replacement: OccupiedPointMap,
        }
        impl Context for MutatingContext<'_> {
            fn done(&self) -> Option<bool> {
                Some(false)
            }
            fn error(&self) -> Option<ContextError> {
                let poll = self.polls.get() + 1;
                self.polls.set(poll);
                if poll == 3 {
                    self.original.insert(Point { x: 900.2, y: 900.4 }, None);
                    *self.occupied.borrow_mut() = self.replacement.clone();
                    Some(ContextError::Cancelled)
                } else {
                    None
                }
            }
        }
        let (mut owner, mapping) = fixture(true);
        let original = OccupiedPointMap::new();
        for index in 0..65 {
            original.insert(
                Point {
                    x: index as f64 + 0.2,
                    y: index as f64 + 0.4,
                },
                None,
            );
        }
        let occupied = RefCell::new(original.clone());
        let replacement = OccupiedPointMap::new();
        replacement.insert(Point { x: 999.2, y: 999.4 }, None);
        let context = MutatingContext {
            polls: Cell::new(0),
            occupied: &occupied,
            original: original.clone(),
            replacement: replacement.clone(),
        };
        let mut scratch = OwnerOptimizerMutationSnapshot::default();
        let mut session =
            OwnerScoreSession::new(&mut owner, OwnerGraphId(0), mapping, None).unwrap();
        let body_called = Cell::new(false);
        let error = with_owner_optimizer_mutation(
            &mut session,
            &occupied,
            &mut scratch,
            &context,
            100,
            |_session, _occupied, _guard| {
                body_called.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "sizelessOptimizer.optimize: context canceled"
        );
        assert_eq!(context.polls.get(), 3);
        assert!(!body_called.get());
        assert!(occupied.borrow().same_map_allocation(&replacement));
        assert_eq!(occupied.borrow().len(), 1);
        assert_eq!(original.len(), 66);
        assert_eq!(original.lookup(Point { x: 900.2, y: 900.4 }), Some(None));
        assert!(!scratch.is_complete());
        assert_eq!(scratch.captured_nodes().len(), 0);
    }

    #[test]
    fn graph_receiver_pass_accepts_duplicate_members_and_cycle_before_armed_restore() {
        let (mut owner, _mapping) = fixture(false);
        let first = NodeId(0);
        let second = NodeId(1);
        let receiver = OwnerGraphId(0);
        let members = vec![first, second, first];
        owner
            .set_owner_graph_members(receiver, members.clone(), vec![])
            .unwrap();
        owner.containers.insert(Some(first), vec![second]);
        owner.containers.insert(Some(second), vec![first]);
        let before = owner.position(first).unwrap();
        let costs = owner.owner_graph_cost_cache(receiver).unwrap().clone();
        costs.store_edge_length(7, 7.25);
        let old_cache = costs.edge_length_cache_alias().unwrap();
        let original = OccupiedPointMap::new();
        original.insert(before, Some(first));
        let occupied = RefCell::new(original.clone());
        let mut scratch = OwnerOptimizerMutationSnapshot::default();
        let called = Cell::new(false);
        let error = with_owner_graph_optimizer_mutation(
            &mut owner,
            receiver,
            &occupied,
            &mut scratch,
            &QuietContext,
            100,
            |owner, occupied, guard| {
                called.set(true);
                // Cache1 + member slots3 + cyclic children2 + herd slots2
                // + occupied entry1. Duplicate visits still consume work.
                assert_eq!(guard.used(), 9);
                assert_eq!(
                    owner
                        .owner_graph_view(receiver)
                        .unwrap()
                        .nodes
                        .iter_slots()
                        .collect::<Vec<_>>(),
                    members.iter().copied().map(Some).collect::<Vec<_>>()
                );
                owner.set_position(first, Point { x: 77.0, y: 88.0 });
                costs.store_edge_length(9, 9.25);
                *occupied.borrow_mut() = OccupiedPointMap::new();
                Err::<(), _>(OptimizationError::Invalid(
                    "duplicate graph body error".into(),
                ))
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "duplicate graph body error");
        assert!(called.get());
        assert_eq!(
            owner
                .owner_graph_view(receiver)
                .unwrap()
                .nodes
                .iter_slots()
                .collect::<Vec<_>>(),
            members.iter().copied().map(Some).collect::<Vec<_>>()
        );
        assert_eq!(owner.position(first), Some(before));
        assert_eq!(costs.lookup_edge_length(7), Some(7.25));
        assert_eq!(costs.lookup_edge_length(9), None);
        assert!(old_cache.is_current());
        assert!(occupied.borrow().same_map_allocation(&original));
        assert_eq!(original.lookup(before), Some(Some(first)));
        assert!(!scratch.is_complete());
        assert_eq!(scratch.captured_nodes().len(), 0);
    }
}
