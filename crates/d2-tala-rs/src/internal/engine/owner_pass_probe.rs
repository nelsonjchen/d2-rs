// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Diagnostic controls for the actual owner snapshot and empty-worklist
//! transaction. Source: snapshot-controls-v7.go.txt, pinned optimization_guard.go
//! and sizeless_optimizer.go106–159. No source caller stack or heap Point
//! allocation identity is synthesized by this native probe.
#![cfg(feature = "diagnostic-traces")]

use super::*;
use crate::internal::go_rng::GoRng;
use crate::internal::limits::{Context, ContextError, OptimizationError, OptimizationWorkGuard};
use crate::internal::model::{GraphCostCache, PlacementCostMapObservation};
use crate::internal::occupied::{OccupiedCapturePhase, OccupiedPointMap};
use crate::internal::owner_optimizer::with_owner_graph_optimizer_mutation;
use crate::internal::owner_score_session::OwnerOptimizerMutationSnapshot;
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};
use std::sync::Arc;

#[path = "owner_member_probe.rs"]
mod owner_member_probe;

#[path = "owner_constructor_probe.rs"]
mod owner_constructor_probe;

pub(super) fn diagnostic_owner_constructor_probe(request: &Value) -> Value {
    owner_constructor_probe::diagnostic_owner_constructor_probe(request)
}

pub(super) fn diagnostic_owner_member_probe(request: &Value) -> Value {
    owner_member_probe::diagnostic_owner_member_probe(request)
}

const LOCATION: &str = "sizelessOptimizer.optimize";

fn pointer(value: Option<usize>) -> String {
    format!("0x{:016x}", value.unwrap_or(0))
}

fn map_observation(map: Option<&PlacementCostMapObservation>) -> Value {
    let Some(map) = map else { return Value::Null };
    json!({"cache_pointer": pointer(map.reference_address),
        "native_map_object_pointer": pointer(Some(map.map_object_address)),
        "length": map.length, "native_capacity": map.capacity,
        "cache_entries": map.entries.iter().map(|&(state,cost_bits)|
            json!({"state":state,"cost_bits":cost_bits})).collect::<Vec<_>>()})
}

// Actual current and detached alias maps are observed under the original
// cost-owner lock and then the map lock. No observer creates a saved snapshot.
fn cache_observation(owner: &ArenaGraph, graph: OwnerGraphId) -> Value {
    let Ok(costs) = owner.owner_graph_cost_cache(graph) else {
        return Value::Null;
    };
    let observed = costs.diagnostic_observe();
    let cache = map_observation(observed.cache.as_ref());
    json!({"graph_pointer":owner_frame_probe_address(owner.owner_graph_view(graph)),
        "native_cost_owner_pointer":pointer(Some(observed.cost_owner_address)),
        "cache_nil":observed.cache.is_none(),
        "cache_pointer":pointer(observed.cache.as_ref().and_then(|map|map.reference_address)),
        "cache_entries":cache.get("cache_entries").cloned().unwrap_or(Value::Null),
        "native_actual_cache":cache,
        "crossing_cost_bits":observed.routing_bits[0],"turn_cost_bits":observed.routing_bits[1],
        "non_center_cost_bits":observed.routing_bits[2],
        "native_routing_field_pointers":observed.routing_addresses.map(|p|pointer(Some(p))),
        "cache_entry_observer_uses_clone":false,
        "actual_map_address_and_traversal_observed":true})
}

fn retained_cache_observation(
    alias: Option<&EdgeLengthCacheAlias>,
    costs: &Arc<GraphCostCache>,
) -> Value {
    let Some(alias) = alias else {
        return json!({"native_cost_owner_pointer":pointer(Some(Arc::as_ptr(costs) as usize)),
            "cache_nil":true,"cache_pointer":pointer(None),"cache_entries":Value::Null,
            "is_current":!costs.edge_length_cache_enabled(),
            "cache_entry_observer_uses_clone":false,"actual_map_address_and_traversal_observed":true});
    };
    let observed = alias.diagnostic_observe();
    let cache = map_observation(Some(&observed.cache));
    json!({"native_cost_owner_pointer":pointer(Some(observed.cost_owner_address)),
        "cache_nil":false,"cache_pointer":pointer(observed.cache.reference_address),
        "cache_entries":cache.get("cache_entries").cloned().unwrap_or(Value::Null),
        "native_actual_cache":cache,
        "native_current_cache_reference_pointer":pointer(observed.current_cache_reference_address),
        "is_current":observed.is_current,"cache_entry_observer_uses_clone":false,
        "actual_map_address_and_traversal_observed":true})
}

fn occupied_entries(owner: &ArenaGraph, map: &OccupiedPointMap) -> Value {
    if map.is_nil() {
        return Value::Null;
    }
    json!(map.observed_entries().into_iter().map(|(point,node)|
        json!({"point":symmetry_point(Some(point)),"node":owner_frame_probe_node(owner,node)}))
        .collect::<Vec<_>>())
}

fn snapshot_observation(
    owner: &ArenaGraph,
    snapshot: &OwnerOptimizerMutationSnapshot,
    used: u64,
) -> Value {
    let rows = snapshot.captured_geometry().enumerate().map(|(index,(node,geometry))| {
        json!({"index":index,"node":owner_frame_probe_node(owner,Some(node)),
            "captured_point_nil":geometry.position.is_none(),
            "captured_point_value":symmetry_point(Some(geometry.position.unwrap_or(Point{x:0.0,y:0.0}))),
            "captured_width":bits(geometry.rect.size.width),
            "captured_height":bits(geometry.rect.size.height),
            "captured_point_pointer":Value::Null,
            "native_saved_rect_origin":symmetry_point(Some(geometry.rect.origin))})
    }).collect::<Vec<_>>();
    let costs = snapshot.cost_snapshot().map(|costs| {
        let observed = costs.diagnostic_observe();
        json!({"snapshot_pointer":pointer(Some(observed.snapshot_address)),
            "native_cost_owner_pointer":pointer(Some(observed.cost_owner_address)),
            "original_cache_nil":observed.original_cache.is_none(),
            "original_cache":map_observation(observed.original_cache.as_ref()),
            "saved_cache_nil":observed.saved_cache.is_none(),
            "saved_cache":map_observation(observed.saved_cache.as_ref()),
            "owner_current_cache":map_observation(observed.owner_current_cache.as_ref()),
            "crossing_cost_bits":observed.saved_routing_bits[0],
            "turn_cost_bits":observed.saved_routing_bits[1],
            "non_center_cost_bits":observed.saved_routing_bits[2],
            "native_saved_routing_field_pointers":observed.saved_routing_addresses.map(|p|pointer(Some(p))),
            "native_owner_routing_field_pointers":observed.owner_routing_addresses.map(|p|pointer(Some(p))),
            "native_owner_routing_bits":observed.owner_routing_bits})
    });
    json!({"scratch":{"scratch_pointer":owner_frame_probe_address(Some(snapshot)),
            "complete":snapshot.is_complete(),"receiver":snapshot.receiver().map(|id|id.0),
            "node_length":rows.len()},
        "captured_nodes":rows,"node_capacity_rows":Value::Null,
        "actual_cost_snapshot":costs,"actual_direct_guard_used":used,
        "native_saved_point_heap_alias_observed":false,
        "native_scratch_capacity_rows_observed":false})
}

struct PassControls<'a> {
    request: &'a Value,
    nodes: Vec<NodeId>,
    receiver: OwnerGraphId,
    temporary: OwnerGraphId,
    retained_header: SharedNodeSlice,
    replacement_header: SharedNodeSlice,
    retained_cache: Option<EdgeLengthCacheAlias>,
    selected_costs: Arc<GraphCostCache>,
    occupied: &'a RefCell<OccupiedPointMap>,
    retained_occupied: OccupiedPointMap,
    observer: bool,
    polls: Cell<u64>,
    phase: Cell<&'static str>,
    callback_mutation_applied: Cell<bool>,
    additional_reads: Cell<u64>,
    unexpected_immutable_polls: Cell<u64>,
    observations: RefCell<Vec<Value>>,
    occupied_observations: RefCell<Vec<Value>>,
    current_yield: RefCell<Option<(Point, Option<NodeId>)>>,
    range_original: RefCell<Option<OccupiedPointMap>>,
    range_copied: RefCell<Option<OccupiedPointMap>>,
    seen_yield_points: RefCell<Vec<Point>>,
    runtime_yield_mutation_applied: Cell<bool>,
    panic_payload: Arc<&'static str>,
}

impl PassControls<'_> {
    fn text(&self, key: &str) -> &str {
        self.request.get(key).and_then(Value::as_str).unwrap_or("")
    }
    fn state(&self, owner: &ArenaGraph) -> Value {
        let current = self.occupied.borrow();
        let member = owner
            .owner_graph_view(self.receiver)
            .expect("actual selected receiver");
        let selected_header = member.nodes.header();
        json!({"nodes":self.nodes.iter().map(|&node|owner_frame_probe_node(owner,Some(node))).collect::<Vec<_>>(),
            "selected_graph_pointer":owner_frame_probe_address(Some(member)),
            "selected_member_header":{"nil":selected_header.is_nil(),"length":selected_header.len(),"capacity":selected_header.capacity(),
                "offset":selected_header.offset(),
                "data_pointer":pointer(selected_header.data_address()),
                "backing_pointer":pointer(selected_header.backing_address()),
                "logical_items":selected_header.iter_slots().map(|node|owner_frame_probe_node(owner,node)).collect::<Vec<_>>(),
                "capacity_items":selected_header.capacity_slots().into_iter().map(|node|owner_frame_probe_node(owner,node)).collect::<Vec<_>>()},
            "selected_root_header":owner_frame_probe_map_header(owner,self.receiver,None),
            "selected_C_header":owner_frame_probe_map_header(owner,self.receiver,Some(self.nodes[0])),
            "selected_D_header":owner_frame_probe_map_header(owner,self.receiver,Some(self.nodes[3])),
            "original_C_header":owner_frame_probe_map_header(owner,OwnerGraphId(0),Some(self.nodes[0])),
            "temporary_C_header":owner_frame_probe_map_header(owner,self.temporary,Some(self.nodes[0])),
            "retained_authoring_C_header":owner_frame_probe_header(owner,&self.retained_header),
            "preallocated_replacement_header":owner_frame_probe_header(owner,&self.replacement_header),
            "selected_raw_costs":cache_observation(owner,self.receiver),
            "original_raw_costs":cache_observation(owner,OwnerGraphId(0)),
            "temporary_raw_costs":cache_observation(owner,self.temporary),
            "retained_original_cache":retained_cache_observation(self.retained_cache.as_ref(),&self.selected_costs),
            "occupied_nil":current.is_nil(),"occupied_map_pointer":pointer(current.map_address()),
            "occupied_entries":occupied_entries(owner,&current),
            "retained_old_occupied_nil":self.retained_occupied.is_nil(),
            "retained_old_occupied_pointer":pointer(self.retained_occupied.map_address()),
            "retained_old_occupied_entries":occupied_entries(owner,&self.retained_occupied),
            "direct_snapshot":Value::Null,
            "native_snapshot_at_actual_poll_available":false,
            "native_point_addresses_are_inline_fields":true})
    }
    fn direct_state(
        &self,
        owner: &ArenaGraph,
        snapshot: &OwnerOptimizerMutationSnapshot,
        used: u64,
    ) -> Value {
        let mut state = self.state(owner);
        state["direct_snapshot"] = snapshot_observation(owner, snapshot, used);
        state
    }
    fn mutate_geometry_and_caches(&self, owner: &mut ArenaGraph) {
        for (index, &id) in self.nodes.iter().enumerate() {
            let live = &mut owner.nodes[id.0 as usize];
            let point = live.position.map_or(
                Point {
                    x: (700 + index) as f64,
                    y: -700.0 - index as f64,
                },
                |point| Point {
                    x: point.x + 17.25,
                    y: point.y - 13.5,
                },
            );
            live.position = Some(point);
            live.rect.origin = point;
            live.rect.size.width += 0.25;
            live.rect.size.height -= 0.5;
        }
        self.selected_costs
            .publish_deserialized_routing((31.25, 37.5, 41.75));
        self.selected_costs.store_edge_length(7001, 9.25);
        if let Some(alias) = &self.retained_cache {
            alias.store(8001, 51.25);
        }
    }
    fn mutate_actual_yield(&self, owner: &ArenaGraph) -> bool {
        let Some((point, node)) = *self.current_yield.borrow() else {
            return false;
        };
        let Some(original) = self.range_original.borrow().as_ref().cloned() else {
            return false;
        };
        let before = self.state(owner);
        let mut selected = point;
        match self.text("profile") {
            "occupied-live-current-value-change" => {
                original.insert(point, Some(self.nodes[2]));
            }
            "occupied-live-unvisited-value-change" => {
                // Source chooses an arbitrary currently unvisited reflexive key
                // in a second map range. Selection order is not source-bound;
                // its actual within-call selected-key relation is retained.
                let seen = self.seen_yield_points.borrow();
                let Some(candidate) = original.find_unvisited_point(&seen) else {
                    return false;
                };
                selected = candidate;
                original.insert(candidate, Some(self.nodes[2]));
            }
            "occupied-live-current-map-field-replacement" => {
                let replacement = OccupiedPointMap::new();
                replacement.insert(Point { x: 999.2, y: 999.4 }, Some(self.nodes[2]));
                *self.occupied.borrow_mut() = replacement;
            }
            _ => return false,
        }
        self.occupied_observations.borrow_mut().push(json!({
            "phase":"actual_yield_poll_control_mutation","actual_poll_ordinal":self.polls.get(),
            "actual_current_point":symmetry_point(Some(point)),
            "actual_current_node":owner_frame_probe_node(owner,node),
            "authored_selected_point":symmetry_point(Some(selected)),"applied":true,
            "before":before,"after":self.state(owner),
            "native_actual_copied_snapshot_entries":self.range_copied.borrow().as_ref().map(|map|occupied_entries(owner,map)),
            "native_actual_loaded_pair":true,
            "fixed_source_map_selected_key_or_order_qualified":false}));
        true
    }
    fn mutate_callback(&self, owner: &mut ArenaGraph) {
        match self.text("profile") {
            "capture-future-forward-slot-change" => {
                self.retained_header
                    .write_capacity_slot(62, Some(self.nodes[3]))
                    .expect("authored future slot");
            }
            "capture-loaded-forward-slot-change" => {
                self.retained_header
                    .write_capacity_slot(61, None)
                    .expect("authored loaded slot");
            }
            "capture-replaced-forward-header" => {
                owner
                    .containers
                    .replace_header(Some(self.nodes[0]), self.replacement_header.clone());
            }
            _ => {
                self.mutate_geometry_and_caches(owner);
                if !self.retained_occupied.is_nil() {
                    self.retained_occupied
                        .insert(Point { x: 900.2, y: 900.4 }, Some(self.nodes[1]));
                }
                let replacement = OccupiedPointMap::new();
                replacement.insert(Point { x: 999.2, y: 999.4 }, Some(self.nodes[2]));
                *self.occupied.borrow_mut() = replacement;
            }
        }
    }
}

impl Context for PassControls<'_> {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        // Empty Shuffle performs no polls. A future accidental immutable
        // dispatch is counted and makes the diagnostic binding visibly fail.
        self.unexpected_immutable_polls
            .set(self.unexpected_immutable_polls.get() + 1);
        None
    }
    fn occupied_capture_state(
        &self,
        owner: &ArenaGraph,
        phase: OccupiedCapturePhase,
        point: Option<Point>,
        node: Option<NodeId>,
        original: &OccupiedPointMap,
        copied: &OccupiedPointMap,
    ) {
        *self.range_original.borrow_mut() = Some(original.clone());
        *self.range_copied.borrow_mut() = Some(copied.clone());
        let phase_name = match phase {
            OccupiedCapturePhase::BeforeStep => {
                let point = point.expect("actual yielded key before Step");
                *self.current_yield.borrow_mut() = Some((point, node));
                self.seen_yield_points.borrow_mut().push(point);
                "actual_yield_before_step"
            }
            OccupiedCapturePhase::AfterCopy => "actual_copied_after_assignment",
            OccupiedCapturePhase::Complete => {
                *self.current_yield.borrow_mut() = None;
                "actual_completed_copy_before_rollback_arm"
            }
        };
        let mut row = json!({"phase":phase_name,
            "source_range_original_pointer":pointer(original.map_address()),
            "source_range_original_entries":occupied_entries(owner,original),
            "source_copied_snapshot_pointer":pointer(copied.map_address()),
            "source_copied_snapshot_entries":occupied_entries(owner,copied),
            "optimizer_current_map_pointer":pointer(self.occupied.borrow().map_address()),
            "native_map_alias_identity_measured":true,"source_map_heap_address_identity_qualified":false});
        if phase != OccupiedCapturePhase::Complete {
            row["point"] = symmetry_point(point);
            row["node"] = owner_frame_probe_node(owner, node);
        }
        if phase == OccupiedCapturePhase::BeforeStep {
            row["polls_before_step"] = json!(self.polls.get());
        }
        self.occupied_observations.borrow_mut().push(row);
    }
    fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        let finish = match self.phase.get() {
            "inner" => "inner",
            "outer" => "outer",
            _ => "",
        };
        let before = self.state(owner);
        let mutate_here = match self.text("profile") {
            "capture-future-forward-slot-change"
            | "capture-loaded-forward-slot-change"
            | "capture-replaced-forward-header"
            | "capture-error-side-effects-before-arm" => poll == 3,
            "cost-cache-charge-cancel64"
            | "cost-copy-finish-cancel"
            | "cost-count-before-copy-live-change" => poll == 2,
            "occupied-copy-cancel-before-arm" => poll == 4,
            "occupied-original-map-armed-cancel"
            | "occupied-original-nil-armed-panic"
            | "inner-finish-versus-outer-finish" => finish == self.text("finish_target"),
            _ => false,
        };
        if mutate_here && !self.callback_mutation_applied.get() {
            self.mutate_callback(owner);
            self.callback_mutation_applied.set(true);
        }
        if self.text("profile").starts_with("occupied-live-")
            && poll == 4
            && !self.runtime_yield_mutation_applied.get()
        {
            self.runtime_yield_mutation_applied
                .set(self.mutate_actual_yield(owner));
        }
        let target = self.text("finish_target");
        let cancel = self
            .request
            .get("cancel_at_poll")
            .and_then(Value::as_u64)
            .is_some_and(|n| n != 0 && n == poll)
            || (!target.is_empty() && target == finish && self.text("outcome") == "error");
        let panic = self
            .request
            .get("panic_at_poll")
            .and_then(Value::as_u64)
            .is_some_and(|n| n != 0 && n == poll)
            || (!target.is_empty() && target == finish && self.text("outcome") == "panic");
        let additional = if self.observer {
            self.additional_reads.set(self.additional_reads.get() + 1);
            Some(self.state(owner))
        } else {
            None
        };
        self.observations
            .borrow_mut()
            .push(json!({"op":"actual-native-operation-context-poll",
            "actual_poll_ordinal":poll,"native_phase":self.phase.get(),
            "state_before_control_mutation":before,"state":self.state(owner),
            "control_mutation_applied_here":mutate_here,
            "additional_read_only_observer_state":additional,
            "actual_native_finish_boundary":finish,"returns_canceled":cancel,"panics":panic,
            "source_caller_stack_observed":false,"snapshot_at_actual_poll_observed":false}));
        if panic {
            panic_any(self.panic_payload.clone())
        }
        cancel.then_some(ContextError::Cancelled)
    }
}

pub(super) fn diagnostic_owner_pass_mutation_probe(request: &Value) -> Value {
    let profile = request.get("profile").and_then(Value::as_str).unwrap_or("");
    let mode = request
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("snapshot");
    if !matches!(mode, "snapshot" | "optimize-empty-worklist") {
        return json!({"supported":false,"request":request,"request_error":"unknown owner pass mode"});
    }
    if !matches!(
        profile,
        "selected-receiver-bfs-retained-closure"
            | "non-container-keyed-children"
            | "duplicate-cycle-forward-bfs"
            | "nil-top-left-and-dimensions"
            | "capture-future-forward-slot-change"
            | "capture-loaded-forward-slot-change"
            | "capture-replaced-forward-header"
            | "selected-cache-original-alias-restore"
            | "selected-cache-original-nil-restore"
            | "release-and-reuse-is-inert"
            | "nil-child-budget-precedence"
            | "cost-cache-charge-cancel64"
            | "cost-copy-finish-cancel"
            | "cost-count-before-copy-live-change"
            | "capture-error-side-effects-before-arm"
            | "occupied-copy-cancel-before-arm"
            | "occupied-original-map-armed-cancel"
            | "occupied-original-nil-armed-panic"
            | "inner-finish-versus-outer-finish"
            | "finite-fractional-signed-zero-occupied-keys"
            | "nan-nonreflexive-duplicate-occupied-keys"
            | "occupied-present-nil-value"
            | "occupied-live-current-value-change"
            | "occupied-live-unvisited-value-change"
            | "occupied-live-current-map-field-replacement"
    ) {
        return json!({"supported":false,"request":request,"request_error":"unknown owner pass profile"});
    }
    let mut prepared = match prepared_owner_frame_probe_fixture() {
        Ok(prepared) => prepared,
        Err(error) => return json!({"supported":false,"request_error":error}),
    };
    let receiver = prepared.receiver;
    let temporary = prepared
        .fixture
        .prepared_owner
        .as_ref()
        .expect("actual prepared temporary")
        .view;
    let owner = &mut prepared.fixture.original.graph;
    owner
        .owner_graph_cost_cache(temporary)
        .expect("actual canonical temporary cost owner")
        .store_edge_length(SYMMETRY_SENTINEL_KEY, 19.0);
    owner
        .restore_owner_graph_routing_costs(temporary, (11.0, 13.0, 17.0))
        .expect("actual canonical temporary routing owner");
    let nodes = [1, 2, 3, 1000, 1001]
        .map(|tala| {
            owner
                .nodes
                .iter()
                .position(|node| node.tala_id == tala)
                .map(|index| NodeId(index as u32))
                .expect("actual canonical fixture node")
        })
        .to_vec();
    owner.set_position(nodes[2], Point { x: 500.0, y: 100.0 });
    owner.clusters.clear();
    owner.sequences.clear();
    for node in &mut owner.nodes {
        node.herd_assignment = None;
    }
    let long_header = profile.contains("forward")
        || mode == "optimize-empty-worklist"
        || profile == "capture-error-side-effects-before-arm";
    if long_header {
        owner.containers.replace_header(
            Some(nodes[0]),
            SharedNodeSlice::from_nodes(&vec![nodes[1]; 65]).unwrap(),
        );
    }
    match profile {
        "selected-receiver-bfs-retained-closure" => {
            owner
                .set_owner_graph_reference(
                    OwnerGraphId(0),
                    &[nodes[1]],
                    false,
                    &mut OwnerGraphOwnershipJournal::default(),
                )
                .unwrap();
        }
        "non-container-keyed-children" => {
            owner.nodes[nodes[2].0 as usize].is_container = false;
            owner.containers.replace_header(
                Some(nodes[2]),
                SharedNodeSlice::from_nodes(&[nodes[3]]).unwrap(),
            );
        }
        "duplicate-cycle-forward-bfs" => {
            let members = owner.owner_graph_member_header(receiver).unwrap();
            let members = owner
                .append_owner_member_header(&members, &[Some(nodes[0])])
                .unwrap();
            owner
                .set_owner_graph_member_header(receiver, members)
                .unwrap();
            owner.containers.replace_header(
                Some(nodes[0]),
                SharedNodeSlice::from_nodes(&[nodes[1], nodes[1]]).unwrap(),
            );
            owner.containers.replace_header(
                Some(nodes[1]),
                SharedNodeSlice::from_nodes(&[nodes[0]]).unwrap(),
            );
        }
        "nil-child-budget-precedence" => {
            owner.containers.replace_header(
                Some(nodes[0]),
                SharedNodeSlice::from_slots(&[None]).unwrap(),
            );
        }
        "nil-top-left-and-dimensions" => {
            let child = &mut owner.nodes[nodes[1].0 as usize];
            child.position = None;
            child.rect.size.width = f64::from_bits(0x7ff8000000000042);
            child.rect.size.height = -0.0;
        }
        _ => {}
    }
    let retained_header = owner
        .containers
        .get_header(&Some(nodes[0]))
        .expect("authored C header");
    let replacement_header = SharedNodeSlice::from_nodes(&vec![nodes[3]; 65]).unwrap();
    let selected_costs = owner.owner_graph_cost_cache(receiver).unwrap().clone();
    selected_costs.publish_deserialized_routing((-0.0, 1.25, 2.5));
    if profile == "selected-cache-original-nil-restore" {
        selected_costs.disable_edge_length_cache();
    }
    if matches!(
        profile,
        "selected-cache-original-alias-restore" | "cost-copy-finish-cancel"
    ) {
        selected_costs.store_edge_length(17, 3.25);
    }
    if matches!(
        profile,
        "cost-cache-charge-cancel64" | "cost-count-before-copy-live-change"
    ) {
        for index in 0..65 {
            selected_costs.store_edge_length(index + 100, index as f64 + 0.25);
        }
    }
    let retained_cache = selected_costs.edge_length_cache_alias();
    let mut occupied_map = OccupiedPointMap::new();
    for index in 0..65 {
        occupied_map.insert(
            Point {
                x: index as f64 + 0.2,
                y: index as f64 + 0.4,
            },
            Some(nodes[0]),
        );
    }
    match profile {
        "occupied-original-nil-armed-panic" => occupied_map = OccupiedPointMap::default(),
        "finite-fractional-signed-zero-occupied-keys" => {
            occupied_map = OccupiedPointMap::new();
            occupied_map.insert(Point { x: 0.2, y: 0.2 }, Some(nodes[0]));
            occupied_map.insert(Point { x: 0.4, y: 0.4 }, Some(nodes[2]));
            occupied_map.insert(Point { x: 0.0, y: 0.0 }, Some(nodes[0]));
            occupied_map.insert(Point { x: -0.0, y: -0.0 }, Some(nodes[1]));
        }
        "nan-nonreflexive-duplicate-occupied-keys" => {
            occupied_map = OccupiedPointMap::new();
            let key = Point {
                x: f64::from_bits(0x7ff8000000000042),
                y: 1.0,
            };
            for node in [Some(nodes[0]), Some(nodes[2]), None] {
                occupied_map.insert(key, node);
            }
        }
        "occupied-present-nil-value" => {
            occupied_map = OccupiedPointMap::new();
            occupied_map.insert(Point { x: 0.2, y: 0.4 }, None);
            occupied_map.insert(Point { x: 0.4, y: 0.2 }, Some(nodes[2]));
        }
        _ => {}
    }
    let retained_occupied = occupied_map.clone();
    let occupied = RefCell::new(occupied_map);
    let controls = PassControls {
        request,
        nodes,
        receiver,
        temporary,
        retained_header,
        replacement_header,
        retained_cache,
        selected_costs,
        occupied: &occupied,
        retained_occupied,
        observer: request
            .get("observer")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        polls: Cell::new(0),
        phase: Cell::new("capture"),
        callback_mutation_applied: Cell::new(false),
        additional_reads: Cell::new(0),
        unexpected_immutable_polls: Cell::new(0),
        observations: RefCell::new(Vec::new()),
        occupied_observations: RefCell::new(Vec::new()),
        current_yield: RefCell::new(None),
        range_original: RefCell::new(None),
        range_copied: RefCell::new(None),
        seen_yield_points: RefCell::new(Vec::new()),
        runtime_yield_mutation_applied: Cell::new(false),
        panic_payload: Arc::new("actual source pass boundary panic"),
    };
    let mut snapshot = OwnerOptimizerMutationSnapshot::default();
    let mut direct_used = 0;
    let mut direct_calls = 0_u64;
    let mut direct_guard = None;
    let mut events = Vec::new();
    let mut error_value = None;
    let mut panic_caught = false;
    let mut panic_exact = false;
    let mut panic_pointer = None;
    let mut panic_type = None;
    let mut panic_value = None;
    let limit = request
        .get("work_limit")
        .and_then(Value::as_u64)
        .unwrap_or(1000);
    let entry = controls.direct_state(owner, &snapshot, direct_used);
    let operation = catch_unwind(AssertUnwindSafe(|| -> Result<(), OptimizationError> {
        if mode == "optimize-empty-worklist" {
            let mut rng =
                GoRng::new_untraced(request.get("seed").and_then(Value::as_i64).unwrap_or(1729));
            let result = with_owner_graph_optimizer_mutation(
                owner,
                receiver,
                &occupied,
                &mut snapshot,
                &controls,
                limit,
                |owner, _occupied, guard| {
                    let mut empty: [NodeId; 0] = [];
                    rng.shuffle_guarded(&mut empty, guard)?;
                    controls.phase.set("inner");
                    guard.finish_with_owner(owner)?;
                    controls.phase.set("outer");
                    Ok(())
                },
            );
            events.push(json!({"phase":"after_actual_optimize_return","state":controls.direct_state(owner,&snapshot,direct_used)}));
            return result;
        }
        direct_guard = None;
        let captured =
            match OptimizationWorkGuard::new_with_owner(Some(&controls), LOCATION, limit, owner) {
                Ok(guard) => {
                    direct_guard = Some(guard);
                    direct_calls += 1;
                    snapshot.capture_into(
                        owner,
                        receiver,
                        direct_guard.as_mut().expect("actual direct guard"),
                    )
                }
                Err(error) => Err(error),
            };
        direct_used = direct_guard.as_ref().map_or(0, OptimizationWorkGuard::used);
        if let Err(error) = captured {
            events.push(json!({"phase":"after_failed_capture","state":controls.direct_state(owner,&snapshot,direct_used)}));
            snapshot.restore(owner)?;
            events.push(json!({"phase":"after_actual_nil_returned_restore","state":controls.direct_state(owner,&snapshot,direct_used)}));
            return Err(error);
        }
        events.push(json!({"phase":"after_capture","state":controls.direct_state(owner,&snapshot,direct_used)}));
        controls.mutate_geometry_and_caches(owner);
        events.push(json!({"phase":"after_authored_mutation","state":controls.direct_state(owner,&snapshot,direct_used)}));
        if profile == "release-and-reuse-is-inert" {
            snapshot.release();
            events.push(json!({"phase":"after_release_without_restore","state":controls.direct_state(owner,&snapshot,direct_used)}));
            direct_guard = None;
            direct_guard = Some(OptimizationWorkGuard::new_with_owner(
                Some(&controls),
                LOCATION,
                limit,
                owner,
            )?);
            direct_calls += 1;
            let guard = direct_guard.as_mut().expect("actual second direct guard");
            let captured = snapshot.capture_into(owner, receiver, guard);
            direct_used = guard.used();
            captured?;
            events.push(json!({"phase":"after_second_actual_capture","state":controls.direct_state(owner,&snapshot,direct_used)}));
            controls.mutate_geometry_and_caches(owner);
        }
        if request.get("outcome").and_then(Value::as_str) == Some("restore") {
            snapshot.restore(owner)?;
            events.push(json!({"phase":"after_actual_restore","state":controls.direct_state(owner,&snapshot,direct_used)}));
        }
        snapshot.release();
        events.push(json!({"phase":"after_actual_release","state":controls.direct_state(owner,&snapshot,direct_used)}));
        Ok(())
    }));
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
    direct_used = direct_guard.as_ref().map_or(0, OptimizationWorkGuard::used);
    let after = controls.direct_state(owner, &snapshot, direct_used);
    let alias_stored = if let Some(alias) = &controls.retained_cache {
        alias.store(0x434f444558, 123.25);
        true
    } else {
        false
    };
    let lookup = controls.selected_costs.lookup_edge_length(0x434f444558);
    let alias_proof = json!({"sentinel_write_succeeded":alias_stored,"selected_actual_lookup_present":lookup.is_some(),
        "selected_actual_lookup_value":bits(lookup.unwrap_or(0.0)),
        "during_alias_sentinel_state":controls.direct_state(owner,&snapshot,direct_used)});
    if let Some(alias) = &controls.retained_cache {
        alias.remove(0x434f444558);
    }
    json!({"case":request.get("case"),"observer":controls.observer,"supported":true,
        "probe":"owner-pass-mutation-snapshot","request":request,
        "result":{"entry":entry,"events":events,"after_operation":after,
            "after_alias_sentinel_removed":controls.direct_state(owner,&snapshot,direct_used),
            "alias_proof":alias_proof,"error":error_value,"panic_caught":panic_caught,
            "panic_exact_payload":panic_exact,"panic_actual_type":panic_type,
            "panic_actual_value":panic_value,"panic_actual_pointer":panic_pointer,
            "additional_read_only_observer_reads":controls.additional_reads.get(),"polls":controls.polls.get(),
            "operation_context_observations":std::mem::take(&mut *controls.observations.borrow_mut()),
            "actual_optimize_local_guard_used":Value::Null,"actual_direct_guard_used":direct_used,
            "callback_mutation_applied":controls.callback_mutation_applied.get(),
            "occupied_runtime_observations":std::mem::take(&mut *controls.occupied_observations.borrow_mut()),
            "actual_last_range_original_entries":controls.range_original.borrow().as_ref().map(|map|occupied_entries(owner,map)),
            "actual_last_copied_snapshot_entries":controls.range_copied.borrow().as_ref().map(|map|occupied_entries(owner,map)),
            "runtime_yield_mutation_applied":controls.runtime_yield_mutation_applied.get(),"unexpected_immutable_context_polls":controls.unexpected_immutable_polls.get()},
        "actual_native_direct_capture_into_invoked":direct_calls>0,
        "actual_direct_capture_into_call_count":direct_calls,
        "native_owner_transaction_wrapper_invoked":mode=="optimize-empty-worklist",
        "native_internal_capture_call_count_observed":false,
        "actual_optimize_internal_capture_into_call_count":Value::Null,
        "literal_empty_worklist_control":mode=="optimize-empty-worklist",
        "ordinary_nil_clusters_sequences_herds":false,
        "native_empty_clusters_sequences_nil_herds":true,"source_nil_group_map_header_identity_qualified":false,"diagnostic_runtime_yield_observer_available":true,
        "runtime_local_control_requires_instrumented_observer":profile.starts_with("occupied-live-"),
        "runtime_local_control_qualified_by_this_call":!profile.starts_with("occupied-live-")||controls.runtime_yield_mutation_applied.get(),
        "native_occupied_copy_actual_shared_alias_available":controls.range_copied.borrow().is_some(),
        "native_snapshot_at_actual_poll_available":false,"native_scratch_capacity_rows_observed":false,
        "native_current_cache_raw_address_and_order_observed":true,
        "source_heap_point_alias_identity_qualified":false,"source_private_optimizer_guard_qualified":false,
        "unordered_map_iteration_trace_parity":false,"nonreflexive_nan_map_slot_identity_qualified":false,
        "full_operation_trace_parity":false,"whole_optimizer_or_production_qualified":false,"full_goal_complete":false})
}
