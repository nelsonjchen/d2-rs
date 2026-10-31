// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Literal constructor operands from constructor-controls.formatted.go.txt.
//! The actual constructor owns validation, selection, occupied reset and work.
//! Private transient state is not reconstructed by the context observer.
#![cfg(feature = "diagnostic-traces")]

use super::owner_member_probe::member_header_observation;
use super::*;
use crate::internal::node_near_map::NodeNearMap;
use crate::internal::owner_sizeless_constructor::{OwnerOptimizerRng, OwnerSizelessOptimizer};
use crate::internal::shared_topology::{OwnerMemberHeader, OwnerNodeToTreeMap};

fn flag(request: &Value, key: &str) -> bool {
    request.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn number(request: &Value, key: &str) -> u64 {
    request.get(key).and_then(Value::as_u64).unwrap_or(0)
}
fn text<'a>(request: &'a Value, key: &str) -> &'a str {
    request.get(key).and_then(Value::as_str).unwrap_or("")
}

fn tree_observation(owner: &ArenaGraph, tree: Option<OwnerTreeId>) -> Value {
    let Some(record) = tree.and_then(|tree| owner.owner_tree(tree)) else {
        return json!({"nil":true,"tree_pointer":pointer(None)});
    };
    let fields = record.fields();
    // The canonical record now stores the complete Go integer, including
    // values outside geo's named constants.
    let ordinal = fields.orientation;
    json!({"nil":false,"tree_pointer":owner_frame_probe_address(Some(record)),
        "node_pointer":owner_frame_probe_address(fields.node.and_then(|id|owner.topology_node(id))),
        "node_id":fields.node.and_then(|id|owner.topology_node(id)).map(|node|node.tala_id),
        "parent_pointer":owner_frame_probe_address(fields.parent.and_then(|id|owner.owner_tree(id))),
        "sentinel_edge_pointer":owner_frame_probe_address(fields.sentinel_edge.and_then(|id|owner.edge_records.get(id.0 as usize))),
        "orientation_ordinal":ordinal,"native_orientation_discriminant":fields.orientation,
        "children_header":{"nil":fields.children.is_nil(),"length":fields.children.len(),"capacity":fields.children.capacity(),
            "data_pointer":pointer(fields.children.data_address()),
            "capacity_items":(0..fields.children.capacity()).map(|index|json!({"index":index,
                "slot_pointer":pointer(fields.children.slot_address(index)),
                "tree_pointer":owner_frame_probe_address(fields.children.read_capacity_slot(index).flatten().and_then(|id|owner.owner_tree(id)))})).collect::<Vec<_>>()}})
}

fn tree_map_observation(owner: &ArenaGraph, map: &OwnerNodeToTreeMap) -> Value {
    json!({"nil":map.is_nil(),"length":map.len(),"map_pointer":pointer(map.map_address()),
        "entries":map.observed_entries().into_iter().map(|(key,tree)|json!({
            "key_pointer":owner_frame_probe_address(key.and_then(|id|owner.topology_node(id))),
            "key_id":key.and_then(|id|owner.topology_node(id)).map(|node|node.tala_id),
            "tree":tree_observation(owner,tree)})).collect::<Vec<_>>(),
        "native_map_iteration_order_qualified":false})
}

fn node_observation(owner: &ArenaGraph, id: Option<NodeId>) -> Value {
    let mut row = owner_frame_probe_node(owner, id);
    let Some(id) = id else { return row };
    let node = owner
        .topology_node(id)
        .expect("actual constructor fixture node");
    row["raw_edges"] = json!({"nil":node.edges.header().is_nil(),"length":node.edges.len(),"capacity":node.edges.capacity(),
        "data_pointer":pointer(node.edges.header().data_address()),
        "logical_items":node.edges.read().iter().enumerate().map(|(index,id)|{
            let edge=owner.edge_records.get(id.0 as usize).expect("actual raw incident edge");
            json!({"index":index,"nil":false,"edge_id":edge.tala_id,
                "edge_pointer":owner_frame_probe_address(Some(edge)),
                "slot_pointer":pointer(node.edges.header().slot_address(index)),
                "from_pointer":owner_frame_probe_address(owner.topology_node(edge.from)),
                "to_pointer":owner_frame_probe_address(owner.topology_node(edge.to))})
        }).collect::<Vec<_>>()});
    row["near_map_nil"] = json!(node.nears.is_nil());
    row["near_map_length"] = json!(node.nears.len());
    row["near_map_pointer"] = json!(pointer(node.nears.reference_address()));
    row["fixed_top_left_pointer"] = json!(owner_frame_probe_address(node.fixed_top_left.as_ref()));
    row["fixed_top_left"] = symmetry_point(node.fixed_top_left);
    row["actual_node_graph_cell_size"] = owner
        .topology_node_owner(id)
        .and_then(|graph| owner.owner_graph_cell_size(graph).ok())
        .map(bits)
        .map(Value::String)
        .unwrap_or(Value::Null);
    row
}

fn returned_observation(owner: &ArenaGraph, optimizer: Option<&OwnerSizelessOptimizer>) -> Value {
    let Some(optimizer) = optimizer else {
        return Value::Null;
    };
    let occupied = optimizer.occupied().borrow();
    json!({"optimizer_pointer":owner_frame_probe_address(Some(optimizer)),
        "actual_graph_pointer":owner_frame_probe_address(owner.owner_graph_view(optimizer.receiver())),
        "actual_optimizer_nodes_header":member_header_observation(owner,optimizer.nodes()),
        "occupied_map_pointer":pointer(occupied.map_address()),"occupied_nil":occupied.is_nil(),
        "occupied_length":occupied.observed_entries().len(),
        "occupied_entries":occupied.observed_entries().into_iter().map(|(point,node)|json!({
            "point":symmetry_point(Some(point)),"node":node_observation(owner,node)})).collect::<Vec<_>>(),
        "actual_returned_rng_pointer":pointer(optimizer.rng().map(OwnerOptimizerRng::reference_address)),
        "native_actual_successful_setup_guard_used":optimizer.setup_work_used()})
}

struct ConstructorControls<'a> {
    request: &'a Value,
    nodes: [NodeId; 5],
    receiver: OwnerGraphId,
    temporary: OwnerGraphId,
    initial: OwnerMemberHeader,
    full_backing: OwnerMemberHeader,
    replacement: OwnerMemberHeader,
    empty_replacement: OwnerMemberHeader,
    initial_tree_map: OwnerNodeToTreeMap,
    replacement_tree_map: OwnerNodeToTreeMap,
    authored_tree: Option<OwnerTreeId>,
    rng: Option<OwnerOptimizerRng>,
    returned: RefCell<Option<Box<OwnerSizelessOptimizer>>>,
    polls: Cell<u64>,
    calls: Cell<u64>,
    mutation_applied: Cell<bool>,
    additional_reads: Cell<u64>,
    immutable_polls: Cell<u64>,
    observations: RefCell<Vec<Value>>,
    rng_draws: RefCell<Vec<Value>>,
    panic_payload: Arc<&'static str>,
}

impl ConstructorControls<'_> {
    fn state(&self, owner: &ArenaGraph) -> Value {
        let graph_map = |graph| {
            tree_map_observation(owner, &owner.owner_graph_node_to_tree_map(graph).unwrap())
        };
        json!({"nodes":self.nodes.map(|node|node_observation(owner,Some(node))),
            "actual_constructor_graph_pointer":if flag(self.request,"receiver_nil"){pointer(None)}else{owner_frame_probe_address(owner.owner_graph_view(self.receiver))},
            "selected_graph_pointer":owner_frame_probe_address(owner.owner_graph_view(self.receiver)),
            "selected_cell_size":bits(owner.owner_graph_cell_size(self.receiver).unwrap()),
            "original_cell_size":bits(owner.owner_graph_cell_size(OwnerGraphId(0)).unwrap()),
            "temporary_cell_size":bits(owner.owner_graph_cell_size(self.temporary).unwrap()),
            "selected_member_header":member_header_observation(owner,&owner.owner_graph_member_header(self.receiver).unwrap()),
            "retained_initial_member_header":member_header_observation(owner,&self.initial),
            "retained_initial_full_backing":member_header_observation(owner,&self.full_backing),
            "preallocated_replacement_member_header":member_header_observation(owner,&self.replacement),
            "preallocated_empty_replacement_member_header":member_header_observation(owner,&self.empty_replacement),
            "selected_tree_map":graph_map(self.receiver),"original_tree_map":graph_map(OwnerGraphId(0)),
            "temporary_tree_map":graph_map(self.temporary),
            "retained_initial_tree_map":tree_map_observation(owner,&self.initial_tree_map),
            "preallocated_replacement_tree_map":tree_map_observation(owner,&self.replacement_tree_map),
            "authored_tree_record":tree_observation(owner,self.authored_tree),
            "selected_C_header":owner_frame_probe_map_header(owner,self.receiver,Some(self.nodes[0])),
            "selected_D_header":owner_frame_probe_map_header(owner,self.receiver,Some(self.nodes[3])),
            "selected_raw_costs":cache_observation(owner,self.receiver),"original_raw_costs":cache_observation(owner,OwnerGraphId(0)),
            "temporary_raw_costs":cache_observation(owner,self.temporary),
            "caller_rng_pointer":pointer(self.rng.as_ref().map(OwnerOptimizerRng::reference_address)),
            "caller_rng_source_pointer":pointer(self.rng.as_ref().map(|rng| &*rng.stream().borrow() as *const GoRng as usize)),
            "actual_source_constructor_calls":self.calls.get(),
            "actual_returned_optimizer":returned_observation(owner,self.returned.borrow().as_deref()),
            "native_source_heap_point_and_edge_slice_nil_identity_qualified":false})
    }
    fn mutate(&self, owner: &mut ArenaGraph) {
        match text(self.request, "action") {
            "none" => {}
            "current-to-D" => owner
                .write_owner_member_capacity_slot(&self.initial, 63, Some(self.nodes[3]))
                .unwrap(),
            "current-to-nil" => owner
                .write_owner_member_capacity_slot(&self.initial, 63, None)
                .unwrap(),
            "future-to-D" => owner
                .write_owner_member_capacity_slot(&self.initial, 64, Some(self.nodes[3]))
                .unwrap(),
            "future-to-nil" => owner
                .write_owner_member_capacity_slot(&self.initial, 64, None)
                .unwrap(),
            "replace" => owner
                .set_owner_graph_member_header(self.receiver, self.replacement.clone())
                .unwrap(),
            "replace-empty" => owner
                .set_owner_graph_member_header(self.receiver, self.empty_replacement.clone())
                .unwrap(),
            "replace-nil" => {
                let nil = owner.nil_owner_member_header();
                owner
                    .set_owner_graph_member_header(self.receiver, nil)
                    .unwrap()
            }
            "node-graph-nil" => owner
                .set_owner_node_graph_reference(self.nodes[0], None)
                .unwrap(),
            "cell-mismatch" => {
                owner
                    .set_owner_node_graph_reference(self.nodes[0], Some(OwnerGraphId(0)))
                    .unwrap();
                owner
                    .set_owner_graph_cell_size(self.receiver, 1.25)
                    .unwrap();
                owner
                    .set_owner_graph_cell_size(OwnerGraphId(0), 2.5)
                    .unwrap();
            }
            "tree-write-present-nil" => {
                let map = owner.owner_graph_node_to_tree_map(self.receiver).unwrap();
                owner
                    .write_owner_node_to_tree_entry(&map, Some(self.nodes[0]), None)
                    .unwrap()
            }
            "tree-delete-C" => {
                let map = owner.owner_graph_node_to_tree_map(self.receiver).unwrap();
                owner
                    .delete_owner_node_to_tree_entry(&map, Some(self.nodes[0]))
                    .unwrap();
            }
            "tree-replace-empty" => owner
                .set_owner_graph_node_to_tree_map(self.receiver, self.replacement_tree_map.clone())
                .unwrap(),
            _ => unreachable!("validated constructor action"),
        }
        if flag(self.request, "mutate_geometry_at_poll") {
            for id in self.nodes {
                let node = &mut owner.nodes[id.0 as usize];
                let point = node.position.as_mut().expect("literal constructor TopLeft");
                point.x += 17.25;
                point.y -= 13.5;
                node.rect.origin = *point;
                node.rect.size.width += 0.25;
                node.rect.size.height -= 0.5;
            }
        }
    }
}

impl Context for ConstructorControls<'_> {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<ContextError> {
        self.immutable_polls.set(self.immutable_polls.get() + 1);
        None
    }
    fn error_with_owner(&self, owner: &mut ArenaGraph) -> Option<ContextError> {
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        let before = self.state(owner);
        let mutate = number(self.request, "mutation_poll") != 0
            && number(self.request, "mutation_poll") == poll;
        if mutate && !self.mutation_applied.get() {
            self.mutate(owner);
            if flag(self.request, "draw_rng_at_poll") {
                let value = self
                    .rng
                    .as_ref()
                    .expect("actual supplied RNG")
                    .stream()
                    .borrow_mut()
                    .int63();
                self.rng_draws
                    .borrow_mut()
                    .push(json!({"actual_poll_ordinal":poll,"int63":value}));
            }
            self.mutation_applied.set(true);
        }
        let cancel = number(self.request, "cancel_at_poll") != 0
            && number(self.request, "cancel_at_poll") == poll;
        let panic = number(self.request, "panic_at_poll") != 0
            && number(self.request, "panic_at_poll") == poll;
        let extra = if flag(self.request, "observer") {
            self.additional_reads.set(self.additional_reads.get() + 1);
            self.state(owner)
        } else {
            Value::Null
        };
        self.observations.borrow_mut().push(json!({"actual_poll_ordinal":poll,"before":before,"after":self.state(owner),
            "mutation_applied_here":mutate,"additional_read_only_state":extra,"returns_canceled":cancel,"panics":panic,
            "actual_native_owner_context_callback":true,"native_source_caller_stack_available":false}));
        if panic {
            panic_any(Arc::clone(&self.panic_payload))
        }
        if cancel {
            Some(ContextError::Cancelled)
        } else {
            None
        }
    }
}

pub(super) fn diagnostic_owner_constructor_probe(request: &Value) -> Value {
    if !matches!(
        text(request, "initial_header"),
        "nil" | "empty0" | "empty-cap5" | "single-C" | "nil-single" | "long65" | "offset65"
    ) || !matches!(
        text(request, "action"),
        "none"
            | "current-to-D"
            | "current-to-nil"
            | "future-to-D"
            | "future-to-nil"
            | "replace"
            | "replace-nil"
            | "replace-empty"
            | "node-graph-nil"
            | "cell-mismatch"
            | "tree-write-present-nil"
            | "tree-delete-C"
            | "tree-replace-empty"
    ) || !matches!(
        text(request, "tree_case"),
        "nil"
            | "empty"
            | "selected-present-nil"
            | "selected-present-tree"
            | "foreign-present-nil"
            | "selected-nil-foreign-owner"
            | "shared-original-selected"
    ) || !matches!(
        text(request, "cell_case"),
        "equal" | "missing-graph" | "finite-mismatch" | "nan-mismatch"
    ) || (flag(request, "rng_nil") && flag(request, "draw_rng_at_poll"))
    {
        return json!({"supported":false,"request":request,"request_error":"unknown literal constructor control input"});
    }
    let mut prepared = match prepared_owner_frame_probe_fixture() {
        Ok(value) => value,
        Err(error) => return json!({"supported":false,"request_error":error}),
    };
    let receiver = prepared.receiver;
    let temporary = prepared.fixture.prepared_owner.as_ref().unwrap().view;
    let owner = &mut prepared.fixture.original.graph;
    owner
        .owner_graph_cost_cache(temporary)
        .unwrap()
        .store_edge_length(SYMMETRY_SENTINEL_KEY, 19.0);
    owner
        .restore_owner_graph_routing_costs(temporary, (11.0, 13.0, 17.0))
        .unwrap();
    let nodes = [1, 2, 3, 1000, 1001].map(|tala| {
        NodeId(
            owner
                .nodes
                .iter()
                .position(|node| node.tala_id == tala)
                .unwrap() as u32,
        )
    });
    owner.set_position(nodes[2], Point { x: 500.0, y: 100.0 });
    owner.clusters.clear();
    owner.sequences.clear();
    for graph in [OwnerGraphId(0), temporary, receiver] {
        owner.set_owner_graph_cell_size(graph, 10.0).unwrap();
        let map = owner.allocate_owner_node_to_tree_map(false);
        owner.set_owner_graph_node_to_tree_map(graph, map).unwrap();
    }
    for id in nodes {
        if let Some(graph) = owner.topology_node_owner(id) {
            owner.set_owner_graph_cell_size(graph, 10.0).unwrap()
        }
        let node = &mut owner.nodes[id.0 as usize];
        node.herd_assignment = None;
        node.nears = NodeNearMap::nil();
        node.fixed_top_left = None;
    }
    let (full_backing, initial) = match text(request, "initial_header") {
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
        "single-C" | "nil-single" => {
            let slot = if text(request, "initial_header") == "single-C" {
                Some(nodes[0])
            } else {
                None
            };
            let backing = owner.allocate_owner_member_header(&[slot], 1).unwrap();
            (backing.clone(), backing)
        }
        _ => {
            let start = if text(request, "initial_header") == "offset65" {
                2
            } else {
                0
            };
            let backing = owner
                .allocate_owner_member_header(&vec![None; start + 70], start + 70)
                .unwrap();
            let header = backing
                .reslice_with_capacity(start, start + 65, start + 70)
                .unwrap();
            for index in 0..65 {
                owner
                    .write_owner_member_capacity_slot(&header, index, Some(nodes[0]))
                    .unwrap()
            }
            for (index, node) in [nodes[2], nodes[3], nodes[1], nodes[4]]
                .into_iter()
                .enumerate()
            {
                owner
                    .write_owner_member_capacity_slot(&header, 65 + index, Some(node))
                    .unwrap()
            }
            (backing, header)
        }
    };
    owner
        .set_owner_graph_member_header(receiver, initial.clone())
        .unwrap();
    let backing = owner
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
    let replacement = backing.reslice_with_capacity(0, 2, 5).unwrap();
    let empty_replacement = owner.allocate_owner_member_header(&[], 5).unwrap();
    let mut authored_tree = None;
    let tree_case = text(request, "tree_case");
    if tree_case != "nil" {
        let target = if matches!(
            tree_case,
            "foreign-present-nil" | "shared-original-selected"
        ) {
            OwnerGraphId(0)
        } else {
            receiver
        };
        let map = owner.allocate_owner_node_to_tree_map(true);
        if tree_case != "empty" {
            if tree_case == "selected-present-tree" {
                authored_tree = Some(owner.new_owner_tree(Some(nodes[0])).unwrap())
            }
            owner
                .write_owner_node_to_tree_entry(&map, Some(nodes[0]), authored_tree)
                .unwrap();
        }
        owner.set_owner_graph_node_to_tree_map(target, map).unwrap();
        if matches!(
            tree_case,
            "foreign-present-nil" | "selected-nil-foreign-owner"
        ) {
            owner
                .set_owner_node_graph_reference(nodes[0], Some(OwnerGraphId(0)))
                .unwrap()
        }
        if tree_case == "shared-original-selected" {
            owner
                .copy_owner_graph_node_to_tree_from(receiver, OwnerGraphId(0))
                .unwrap()
        }
    }
    let initial_tree_map = owner.owner_graph_node_to_tree_map(receiver).unwrap();
    let replacement_tree_map = owner.allocate_owner_node_to_tree_map(true);
    match text(request, "cell_case") {
        "missing-graph" => owner
            .set_owner_node_graph_reference(nodes[0], None)
            .unwrap(),
        "finite-mismatch" | "nan-mismatch" => {
            owner
                .set_owner_node_graph_reference(nodes[0], Some(OwnerGraphId(0)))
                .unwrap();
            let nan = text(request, "cell_case") == "nan-mismatch";
            owner
                .set_owner_graph_cell_size(receiver, if nan { -0.0 } else { 1.25 })
                .unwrap();
            owner
                // Pinned Go math.NaN() is bits.go uvnan, including payload 1.
                .set_owner_graph_cell_size(
                    OwnerGraphId(0),
                    if nan {
                        f64::from_bits(0x7ff8_0000_0000_0001)
                    } else {
                        2.5
                    },
                )
                .unwrap();
        }
        _ => {}
    }
    if flag(request, "fixed_c") {
        owner.nodes[nodes[0].0 as usize].fixed_top_left = Some(Point { x: 80.0, y: 80.0 })
    }
    let rng = (!flag(request, "rng_nil")).then(|| {
        OwnerOptimizerRng::new(GoRng::new_untraced(
            request
                .get("random_seed")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        ))
    });
    let controls = ConstructorControls {
        request,
        nodes,
        receiver,
        temporary,
        initial,
        full_backing,
        replacement,
        empty_replacement,
        initial_tree_map,
        replacement_tree_map,
        authored_tree,
        rng,
        returned: RefCell::new(None),
        polls: Cell::new(0),
        calls: Cell::new(0),
        mutation_applied: Cell::new(false),
        additional_reads: Cell::new(0),
        immutable_polls: Cell::new(0),
        observations: RefCell::new(Vec::new()),
        rng_draws: RefCell::new(Vec::new()),
        panic_payload: Arc::new("actual source constructor boundary panic"),
    };
    let entry = controls.state(owner);
    controls.calls.set(controls.calls.get() + 1);
    let context = (!flag(request, "context_nil")).then_some(&controls as &dyn Context);
    let graph = (!flag(request, "receiver_nil")).then_some(receiver);
    let operation = catch_unwind(AssertUnwindSafe(|| {
        OwnerSizelessOptimizer::new(owner, graph, controls.rng.clone(), context)
    }));
    let mut error = None;
    let mut panic_caught = false;
    let mut panic_exact = false;
    let mut panic_type = None;
    let mut panic_value = None;
    let mut panic_pointer = None;
    match operation {
        Ok(Ok(optimizer)) => {
            controls.returned.replace(Some(Box::new(optimizer)));
        }
        Ok(Err(actual)) => error = Some(actual.to_string()),
        Err(payload) => {
            panic_caught = true;
            if let Some(actual) = payload.downcast_ref::<Arc<&'static str>>() {
                panic_exact = Arc::ptr_eq(actual, &controls.panic_payload);
                panic_type = Some("std::sync::Arc<&str>");
                panic_value = Some((**actual).to_owned());
                panic_pointer = Some(pointer(Some(Arc::as_ptr(actual) as usize)))
            } else if let Some(actual) = payload.downcast_ref::<String>() {
                panic_type = Some("String");
                panic_value = Some(actual.clone())
            } else if let Some(actual) = payload.downcast_ref::<&'static str>() {
                panic_type = Some("&str");
                panic_value = Some((*actual).to_owned())
            } else {
                panic_type = Some("unqualified native panic payload")
            }
        }
    }
    let after = controls.state(owner);
    let post_draw = controls
        .rng
        .as_ref()
        .map(|rng| rng.stream().borrow_mut().int63());
    json!({"case":request.get("case"),"observer":flag(request,"observer"),"request":request,"supported":true,"probe":"owner-sizeless-constructor",
        "result":{"entry":entry,"after_operation":after,"after_post_operation_rng_draw":controls.state(owner),
            "error":error,"panic_caught":panic_caught,"panic_exact_payload":panic_exact,"panic_actual_type":panic_type,"panic_actual_value":panic_value,"panic_actual_pointer":panic_pointer,
            "polls":controls.polls.get(),"mutation_applied":controls.mutation_applied.get(),"callback_rng_draws":controls.rng_draws.take(),
            "post_operation_rng_draw_invoked":controls.rng.is_some(),"post_operation_rng_int63":post_draw,
            "additional_read_only_observer_reads":controls.additional_reads.get(),"setup_context_observations":controls.observations.take(),
            "actual_source_constructor_calls":controls.calls.get(),"actual_private_setup_guard_used":Value::Null,
            "unexpected_immutable_context_polls":controls.immutable_polls.get()},
        "private_setup_guard_available":false,"transient_local_optimizer_observer_available":false,
        "native_source_heap_point_and_edge_slice_nil_identity_qualified":false,"native_map_iteration_order_qualified":false,
        "whole_constructor_or_optimizer_production_qualified":false,"full_operation_trace_parity":false,"full_goal_complete":false})
}
