// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Direct source-shaped Graph.Nodes controls. Inputs are bound to
//! member-header-controls-v2.go.txt; capture, release and restoration use
//! the actual owner snapshot and genuine mutable-owner context polls.
#![cfg(feature = "diagnostic-traces")]

use super::*;
use crate::internal::shared_topology::OwnerMemberHeader;

pub(super) fn member_header_observation(owner: &ArenaGraph, header: &OwnerMemberHeader) -> Value {
    let capacity = header
        .capacity_slots()
        .into_iter()
        .enumerate()
        .map(|(index, node)| {
            let mut row = owner_frame_probe_node(owner, node);
            row["index"] = json!(index);
            row["slot_pointer"] = json!(pointer(header.slot_address(index)));
            row
        })
        .collect::<Vec<_>>();
    json!({"nil":header.is_nil(),"length":header.len(),"capacity":header.capacity(),
        "data_pointer":pointer(header.data_address()),
        "native_backing_pointer":pointer(header.backing_address()),"native_offset":header.offset(),
        "logical_items":capacity.iter().take(header.len()).cloned().collect::<Vec<_>>(),
        "capacity_items":capacity})
}

struct MemberControls<'a> {
    request: &'a Value,
    nodes: [NodeId; 5],
    receiver: OwnerGraphId,
    temporary: OwnerGraphId,
    initial: OwnerMemberHeader,
    full_backing: OwnerMemberHeader,
    replacement: OwnerMemberHeader,
    empty_replacement: OwnerMemberHeader,
    observer: bool,
    polls: Cell<u64>,
    mutation_applied: Cell<bool>,
    additional_reads: Cell<u64>,
    unexpected_immutable_polls: Cell<u64>,
    observations: RefCell<Vec<Value>>,
    panic_payload: Arc<&'static str>,
}

impl MemberControls<'_> {
    fn text(&self, key: &str) -> &str {
        self.request.get(key).and_then(Value::as_str).unwrap_or("")
    }

    fn state(&self, owner: &ArenaGraph) -> Value {
        let current = owner
            .owner_graph_member_header(self.receiver)
            .expect("actual selected member header");
        json!({"nodes":self.nodes.iter().map(|&node|owner_frame_probe_node(owner,Some(node))).collect::<Vec<_>>(),
            "selected_graph_pointer":owner_frame_probe_address(owner.owner_graph_view(self.receiver)),
            "selected_member_header":member_header_observation(owner,&current),
            "retained_initial_member_header":member_header_observation(owner,&self.initial),
            "retained_initial_full_backing":member_header_observation(owner,&self.full_backing),
            "preallocated_replacement_member_header":member_header_observation(owner,&self.replacement),
            "preallocated_empty_replacement_member_header":member_header_observation(owner,&self.empty_replacement),
            "selected_C_header":owner_frame_probe_map_header(owner,self.receiver,Some(self.nodes[0])),
            "selected_D_header":owner_frame_probe_map_header(owner,self.receiver,Some(self.nodes[3])),
            "selected_raw_costs":cache_observation(owner,self.receiver),
            "original_raw_costs":cache_observation(owner,OwnerGraphId(0)),
            "temporary_raw_costs":cache_observation(owner,self.temporary),
            "direct_snapshot":Value::Null,"native_snapshot_at_actual_poll_available":false,
            "native_point_addresses_are_inline_fields":true})
    }

    fn direct_state(
        &self,
        owner: &ArenaGraph,
        snapshot: &OwnerOptimizerMutationSnapshot,
        guard: Option<&OptimizationWorkGuard<'_>>,
        returned: bool,
    ) -> Value {
        let mut state = self.state(owner);
        let mut observed = snapshot_observation(
            owner,
            snapshot,
            guard.map_or(0, OptimizationWorkGuard::used),
        );
        observed["native_successful_capture_return_available"] = json!(returned);
        observed["native_returned_snapshot_pointer"] = json!(if returned {
            owner_frame_probe_address(Some(snapshot))
        } else {
            pointer(None)
        });
        state["direct_snapshot"] = observed;
        state
    }

    fn mutate_geometry(&self, owner: &mut ArenaGraph) {
        for &id in &self.nodes {
            let node = &mut owner.nodes[id.0 as usize];
            let mut position = node.position.expect("literal member fixture TopLeft");
            position.x += 17.25;
            position.y -= 13.5;
            node.position = Some(position);
            node.rect.origin = position;
            node.rect.size.width += 0.25;
            node.rect.size.height -= 0.5;
        }
    }

    fn mutate(&self, owner: &mut ArenaGraph) {
        match self.text("action") {
            "none" => {}
            "current-to-D" => owner
                .write_owner_member_capacity_slot(&self.initial, 63, Some(self.nodes[3]))
                .expect("authored current member slot"),
            "current-to-nil" => owner
                .write_owner_member_capacity_slot(&self.initial, 63, None)
                .expect("authored current nil member slot"),
            "future-to-D" => owner
                .write_owner_member_capacity_slot(&self.initial, 64, Some(self.nodes[3]))
                .expect("authored future member slot"),
            "future-to-nil" => owner
                .write_owner_member_capacity_slot(&self.initial, 64, None)
                .expect("authored future nil member slot"),
            "replace" => owner
                .set_owner_graph_member_header(self.receiver, self.replacement.clone())
                .expect("authored replacement member header"),
            "replace-nil" => {
                let header = owner.nil_owner_member_header();
                owner
                    .set_owner_graph_member_header(self.receiver, header)
                    .unwrap();
            }
            "replace-empty" => owner
                .set_owner_graph_member_header(self.receiver, self.empty_replacement.clone())
                .expect("authored empty replacement member header"),
            "append-within-cap" => {
                let current = owner.owner_graph_member_header(self.receiver).unwrap();
                let appended = owner
                    .append_owner_member_header(&current, &[Some(self.nodes[2])])
                    .unwrap();
                owner
                    .set_owner_graph_member_header(self.receiver, appended)
                    .unwrap();
            }
            "spare-to-D" => owner
                .write_owner_member_capacity_slot(&self.initial, 69, Some(self.nodes[3]))
                .expect("authored spare capacity member slot"),
            _ => unreachable!("validated member action"),
        }
        if self
            .request
            .get("mutate_geometry_at_poll")
            .and_then(Value::as_bool)
            == Some(true)
        {
            self.mutate_geometry(owner);
        }
    }
}

impl Context for MemberControls<'_> {
    fn done(&self) -> Option<bool> {
        None
    }

    fn error(&self) -> Option<ContextError> {
        self.unexpected_immutable_polls
            .set(self.unexpected_immutable_polls.get() + 1);
        None
    }

    fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        let before = self.state(owner);
        let mutation_poll = self
            .request
            .get("mutation_poll")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mutate_here = mutation_poll != 0 && poll == mutation_poll;
        if mutate_here && !self.mutation_applied.get() {
            self.mutate(owner);
            self.mutation_applied.set(true);
        }
        let cancel = self
            .request
            .get("cancel_at_poll")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let panic = self
            .request
            .get("panic_at_poll")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cancel_here = cancel != 0 && poll == cancel;
        let panic_here = panic != 0 && poll == panic;
        let additional = if self.observer {
            self.additional_reads.set(self.additional_reads.get() + 1);
            self.state(owner)
        } else {
            Value::Null
        };
        self.observations.borrow_mut().push(json!({"actual_poll_ordinal":poll,
            "before":before,"after":self.state(owner),"mutation_applied_here":mutate_here,
            "additional_read_only_state":additional,"returns_canceled":cancel_here,"panics":panic_here,
            "actual_native_owner_context_callback":true,"native_source_caller_stack_available":false}));
        if panic_here {
            panic_any(Arc::clone(&self.panic_payload));
        }
        if cancel_here {
            Some(ContextError::Cancelled)
        } else {
            None
        }
    }
}

fn capture_member_direct<'a>(
    owner: &mut ArenaGraph,
    receiver: OwnerGraphId,
    context: &'a dyn Context,
    limit: u64,
    snapshot: &mut OwnerOptimizerMutationSnapshot,
    guard: &mut Option<OptimizationWorkGuard<'a>>,
    direct_calls: &mut u64,
    returned: &mut bool,
) -> Result<(), OptimizationError> {
    // Source wrapper clears the returned pointer and old guard before NewGuard.
    // NewGuard error/panic leaves the previous scratch alone, with no returned
    // snapshot eligible for explicit restoration on this invocation.
    *returned = false;
    *guard = None;
    *guard = Some(OptimizationWorkGuard::new_with_owner(
        Some(context),
        LOCATION,
        limit,
        owner,
    )?);
    *direct_calls += 1;
    snapshot.capture_into(
        owner,
        receiver,
        guard.as_mut().expect("actual member capture guard"),
    )?;
    *returned = true;
    Ok(())
}

fn restore_member_returned(
    owner: &mut ArenaGraph,
    snapshot: &OwnerOptimizerMutationSnapshot,
    returned: bool,
) -> Result<(), OptimizationError> {
    if returned {
        snapshot.restore(owner)
    } else {
        Ok(())
    }
}

pub(super) fn diagnostic_owner_member_probe(request: &Value) -> Value {
    let initial_kind = request
        .get("initial_header")
        .and_then(Value::as_str)
        .unwrap_or("");
    let action = request.get("action").and_then(Value::as_str).unwrap_or("");
    let outcome = request.get("outcome").and_then(Value::as_str).unwrap_or("");
    if !matches!(
        initial_kind,
        "nil" | "empty0" | "empty-cap5" | "nil-single" | "long65" | "long65-loaded-D" | "offset65"
    ) || !matches!(
        action,
        "none"
            | "current-to-D"
            | "current-to-nil"
            | "future-to-D"
            | "future-to-nil"
            | "replace"
            | "replace-nil"
            | "replace-empty"
            | "append-within-cap"
            | "spare-to-D"
    ) || !matches!(outcome, "restore" | "commit" | "reuse")
    {
        return json!({"supported":false,"request":request,"request_error":"unknown literal member control input"});
    }
    let mut prepared = match prepared_owner_frame_probe_fixture() {
        Ok(prepared) => prepared,
        Err(error) => return json!({"supported":false,"request":request,"request_error":error}),
    };
    let receiver = prepared.receiver;
    let temporary = prepared
        .fixture
        .prepared_owner
        .as_ref()
        .expect("actual temporary owner")
        .view;
    let owner = &mut prepared.fixture.original.graph;
    // Actual codexObstructionPrepare authors these independent temporary costs
    // before its tree copy. The common input applies to every member request.
    owner
        .owner_graph_cost_cache(temporary)
        .unwrap()
        .store_edge_length(SYMMETRY_SENTINEL_KEY, 19.0);
    owner
        .restore_owner_graph_routing_costs(temporary, (11.0, 13.0, 17.0))
        .unwrap();
    let nodes = [1, 2, 3, 1000, 1001].map(|tala| {
        owner
            .nodes
            .iter()
            .position(|node| node.tala_id == tala)
            .map(|index| NodeId(index as u32))
            .expect("actual member fixture node")
    });
    owner.set_position(nodes[2], Point { x: 500.0, y: 100.0 });
    owner.clusters.clear();
    owner.sequences.clear();
    for node in &mut owner.nodes {
        node.herd_assignment = None;
    }
    owner
        .owner_graph_cost_cache(receiver)
        .unwrap()
        .publish_deserialized_routing((-0.0, 1.25, 2.5));

    let (full_backing, initial) = match initial_kind {
        "nil" => (
            owner.nil_owner_member_header(),
            owner.nil_owner_member_header(),
        ),
        "empty0" => (
            owner.nil_owner_member_header(),
            owner.allocate_owner_member_header(&[], 0).unwrap(),
        ),
        "empty-cap5" => {
            let backing = owner.allocate_owner_member_header(&[None; 5], 5).unwrap();
            let header = backing.reslice_with_capacity(0, 0, 5).unwrap();
            (backing, header)
        }
        "nil-single" => {
            let backing = owner.allocate_owner_member_header(&[None], 1).unwrap();
            (backing.clone(), backing)
        }
        _ => {
            let start = usize::from(initial_kind == "offset65") * 2;
            let backing = owner
                .allocate_owner_member_header(&vec![None; start + 70], start + 70)
                .unwrap();
            let header = backing
                .reslice_with_capacity(start, start + 65, start + 70)
                .unwrap();
            for index in 0..65 {
                owner
                    .write_owner_member_capacity_slot(&header, index, Some(nodes[0]))
                    .unwrap();
            }
            if initial_kind == "long65-loaded-D" {
                owner
                    .write_owner_member_capacity_slot(&header, 63, Some(nodes[3]))
                    .unwrap();
            }
            for (index, node) in [nodes[2], nodes[3], nodes[1], nodes[4]]
                .into_iter()
                .enumerate()
            {
                owner
                    .write_owner_member_capacity_slot(&header, index + 65, Some(node))
                    .unwrap();
            }
            (backing, header)
        }
    };
    owner
        .set_owner_graph_member_header(receiver, initial.clone())
        .unwrap();
    let replacement_backing = owner
        .allocate_owner_member_header(
            &[
                Some(nodes[3]),
                Some(nodes[2]),
                Some(nodes[1]),
                Some(nodes[0]),
                None,
            ],
            5,
        )
        .unwrap();
    let replacement = replacement_backing.reslice_with_capacity(0, 2, 5).unwrap();
    let empty_replacement = owner.allocate_owner_member_header(&[], 5).unwrap();
    let controls = MemberControls {
        request,
        nodes,
        receiver,
        temporary,
        initial,
        full_backing,
        replacement,
        empty_replacement,
        observer: request
            .get("observer")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        polls: Cell::new(0),
        mutation_applied: Cell::new(false),
        additional_reads: Cell::new(0),
        unexpected_immutable_polls: Cell::new(0),
        observations: RefCell::new(Vec::new()),
        panic_payload: Arc::new("actual source member header boundary panic"),
    };
    let background = BackgroundContext;
    let mut snapshot = OwnerOptimizerMutationSnapshot::default();
    let mut guard = None;
    let mut direct_calls = 0;
    let mut returned = false;
    let mut events = Vec::new();
    if request.get("prewarm").and_then(Value::as_bool) == Some(true) {
        if let Err(error) = capture_member_direct(
            owner,
            receiver,
            &background,
            1000,
            &mut snapshot,
            &mut guard,
            &mut direct_calls,
            &mut returned,
        ) {
            return json!({"supported":false,"request":request,"request_error":error.to_string()});
        }
        events.push(json!({"phase":"after_actual_prewarm_capture",
            "state":controls.direct_state(owner,&snapshot,guard.as_ref(),returned)}));
    }
    let entry = controls.direct_state(owner, &snapshot, guard.as_ref(), returned);
    let limit = request
        .get("work_limit")
        .and_then(Value::as_u64)
        .unwrap_or(1000);
    let operation = catch_unwind(AssertUnwindSafe(|| -> Result<(), OptimizationError> {
        capture_member_direct(
            owner,
            receiver,
            &controls,
            limit,
            &mut snapshot,
            &mut guard,
            &mut direct_calls,
            &mut returned,
        )?;
        events.push(json!({"phase":"after_actual_capture",
            "state":controls.direct_state(owner,&snapshot,guard.as_ref(),returned)}));
        controls.mutate_geometry(owner);
        events.push(json!({"phase":"after_authored_geometry_mutation",
            "state":controls.direct_state(owner,&snapshot,guard.as_ref(),returned)}));
        if outcome == "reuse" {
            snapshot.release();
            events.push(json!({"phase":"after_actual_release_before_reuse",
                "state":controls.direct_state(owner,&snapshot,guard.as_ref(),returned)}));
            capture_member_direct(
                owner,
                receiver,
                &controls,
                limit,
                &mut snapshot,
                &mut guard,
                &mut direct_calls,
                &mut returned,
            )?;
            events.push(json!({"phase":"after_second_actual_capture",
                "state":controls.direct_state(owner,&snapshot,guard.as_ref(),returned)}));
            controls.mutate_geometry(owner);
        }
        if outcome == "restore" || outcome == "reuse" {
            restore_member_returned(owner, &snapshot, returned)?;
            events.push(json!({"phase":"after_actual_restore",
                "state":controls.direct_state(owner,&snapshot,guard.as_ref(),returned)}));
        }
        Ok(())
    }));
    let mut error_value = None;
    let mut panic_caught = false;
    let mut panic_exact = false;
    let mut panic_pointer = None;
    let mut panic_type = None;
    let mut panic_value = None;
    match operation {
        Ok(Ok(())) => {}
        Ok(Err(error)) => error_value = Some(error.to_string()),
        Err(payload) => {
            panic_caught = true;
            if let Some(actual) = payload.downcast_ref::<Arc<&'static str>>() {
                panic_exact = Arc::ptr_eq(actual, &controls.panic_payload);
                panic_pointer = Some(pointer(Some(Arc::as_ptr(actual) as usize)));
                panic_type = Some("std::sync::Arc<&str>");
                panic_value = Some(**actual);
            } else {
                panic_type = Some("unqualified native panic payload");
            }
        }
    }
    if error_value.is_some() || panic_caught {
        events.push(json!({"phase":"after_interrupted_capture",
            "state":controls.direct_state(owner,&snapshot,guard.as_ref(),returned)}));
        if let Err(error) = restore_member_returned(owner, &snapshot, returned) {
            return json!({"supported":false,"request":request,"request_error":error.to_string()});
        }
        events.push(json!({"phase":"after_actual_nil_returned_restore",
            "state":controls.direct_state(owner,&snapshot,guard.as_ref(),returned)}));
    }
    let after = controls.direct_state(owner, &snapshot, guard.as_ref(), returned);
    snapshot.release();
    let after_release = controls.direct_state(owner, &snapshot, guard.as_ref(), returned);
    events.push(json!({"phase":"after_actual_release","state":after_release.clone()}));
    json!({"case":request.get("case"),"observer":controls.observer,"request":request,"supported":true,
        "probe":"owner-member-mutation-snapshot",
        "result":{"entry":entry,"events":events,"after_operation":after,"after_actual_release":after_release,
            "error":error_value,"panic_caught":panic_caught,"panic_exact_payload":panic_exact,
            "panic_actual_type":panic_type,"panic_actual_value":panic_value,"panic_actual_pointer":panic_pointer,
            "polls":controls.polls.get(),"mutation_applied":controls.mutation_applied.get(),
            "additional_read_only_observer_reads":controls.additional_reads.get(),
            "operation_context_observations":std::mem::take(&mut *controls.observations.borrow_mut()),
            "actual_direct_guard_used":guard.as_ref().map_or(0,OptimizationWorkGuard::used),
            "actual_direct_capture_into_call_count":direct_calls,
            "unexpected_immutable_context_polls":controls.unexpected_immutable_polls.get()},
        "actual_range_header_local_observer_available":false,
        "actual_native_direct_capture_into_invoked":direct_calls>0,
        "native_successful_capture_return_available":returned,
        "ordinary_nil_clusters_sequences_herds":false,"native_empty_clusters_sequences_nil_herds":true,
        "source_nil_group_map_header_identity_qualified":false,
        "native_snapshot_at_actual_poll_available":false,"native_scratch_capacity_rows_observed":false,
        "source_heap_point_alias_identity_qualified":false,
        "full_operation_trace_parity":false,"whole_optimizer_or_production_qualified":false,"full_goal_complete":false})
}
