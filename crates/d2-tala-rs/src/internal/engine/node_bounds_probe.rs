// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Prepared graph bounds observed through the production geometry helpers.
#![cfg(feature = "diagnostic-traces")]
use super::*;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Request {
    name: String,
    graph: Value,
    #[serde(default)]
    decoded_membership: bool,
    #[serde(default)]
    decoded_records: bool,
    #[serde(default)]
    sequence_stage: Option<i64>,
    #[serde(default)]
    edge_program: Option<Vec<Value>>,
    #[serde(default)]
    map_program: Option<Vec<Value>>,
    #[serde(default)]
    selection: Vec<i64>,
    #[serde(default)]
    packing_validation: Option<PackingValidation>,
    #[serde(default)]
    packing_traversal: Option<PackingTraversal>,
    #[serde(default)]
    packing_bounds: Option<PackingBounds>,
    #[serde(default)]
    packing_stage: Option<PackingStage>,
    #[serde(default)]
    packing_segments: Option<PackingSegmentRequest>,
    #[serde(default)]
    packing_candidates: Option<PackingCandidateRequest>,
    #[serde(default)]
    packing_atomic: bool,
    #[serde(default)]
    packing_overlaps: bool,
    #[serde(default)]
    overlap_reference_limit: Option<i64>,
    #[serde(default)]
    overlap_invalid: String,
    #[serde(default)]
    packing_snapshot: Option<PackingSnapshot>,
    #[serde(default)]
    route_owner: Option<Value>,
    #[serde(default)]
    route_simplify: Option<Value>,
    #[serde(default)]
    owner_preflight: Option<Value>,
    #[serde(default)]
    limit: i64,
    #[serde(default)]
    cancel_at: usize,
    #[serde(default)]
    deadline: bool,
    #[serde(default)]
    loop_offsets: BTreeMap<i64, Vec<(i64, f64)>>,
    #[serde(default)]
    extended: bool,
    #[serde(default)]
    packing: bool,
    #[serde(default)]
    packing_points: Vec<[f64; 2]>,
    #[serde(default)]
    packing_shift: Option<[f64; 2]>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PackingSnapshot {
    routed: bool,
    request_prefix: bool,
    repeats: usize,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PackingValidation {
    root: Option<i64>,
    node_order: Option<Vec<i64>>,
    edge_order: Option<Vec<Option<i64>>>,
    incidence: BTreeMap<i64, Vec<Option<i64>>>,
    owners: BTreeMap<i64, String>,
    nil_endpoints: Vec<[i64; 2]>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PackingTraversal {
    operation: String,
    start: Option<i64>,
    root: Option<i64>,
    ignore: Option<i64>,
    include_nears: bool,
    near: bool,
    container_is_external: bool,
    edges_placed: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PackingBounds {
    operation: String,
    nodes: Vec<Option<i64>>,
    node: Option<i64>,
    root: Option<i64>,
    contextual: bool,
    parents: BTreeMap<i64, Option<i64>>,
    clear_positions: Vec<i64>,
}

fn observe_bounds(arena: &ArenaGraph, request: &Request, config: &PackingBounds) -> Value {
    struct Context {
        polls: RefCell<Vec<i64>>,
        used: Cell<i64>,
        cancel_at: usize,
        deadline: bool,
    }
    impl limits::Context for Context {
        fn done(&self) -> Option<bool> {
            None
        }
        fn transaction_work_used(&self, used: i64) {
            self.used.set(used);
        }
        fn error(&self) -> Option<limits::ContextError> {
            let mut polls = self.polls.borrow_mut();
            polls.push(self.used.get());
            (self.cancel_at > 0 && polls.len() >= self.cancel_at).then_some(if self.deadline {
                limits::ContextError::DeadlineExceeded
            } else {
                limits::ContextError::Cancelled
            })
        }
    }
    let context = Context {
        polls: RefCell::new(Vec::new()),
        used: Cell::new(0),
        cancel_at: request.cancel_at,
        deadline: request.deadline,
    };
    let find = |id| {
        arena
            .packing_ref_for_id(id)
            .expect("known packing bounds node")
    };
    let nodes = config.nodes.iter().map(|n| n.map(find)).collect::<Vec<_>>();
    let root = config.root.map(|id| match find(id) {
        binpack_traversal::PackingNodeRef::Node(n) => n,
        _ => panic!("bounds container is not a concrete node"),
    });
    let mut observed_guard = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<(Value, Option<String>), limits::Error> {
            observed_guard = Some(limits::WorkGuard::new(
                Some(&context),
                "BinPack",
                limits::MAX_ENGINE_WORK_UNITS,
            )?);
            let guard = observed_guard.as_mut().unwrap();
            guard.set_limit(request.limit);
            let result = match config.operation.as_str() {
                "score" => match arena.packing_score_checked(&nodes, root, guard) {
                    Ok(v) => (json!(format!("{:016x}", v.to_bits())), None),
                    Err(f) => (
                        json!(format!("{:016x}", f.value.to_bits())),
                        Some(f.error.to_string()),
                    ),
                },
                "origin" => match arena.packing_container_origin_checked(&nodes, root, guard) {
                    Ok(v) => (point(v), None),
                    Err(f) => (
                        f.value.map(point).unwrap_or(Value::Null),
                        Some(f.error.to_string()),
                    ),
                },
                mode => {
                    let value = match mode {
                        "node" => arena.packing_node_bounds_checked(
                            config.node.map(find),
                            config.contextual.then_some(nodes.as_slice()),
                            guard,
                        ),
                        "bounds" => arena.packing_bounds_checked(&nodes, guard),
                        "fixed" => arena.packing_fixed_bounds_checked(&nodes, guard),
                        _ => panic!("unknown packing bounds operation"),
                    };
                    match value {
                        Ok((a, b)) => (corners((Some(a), Some(b))), None),
                        Err(f) => (corners(f.value), Some(f.error.to_string())),
                    }
                }
            };
            Ok(result)
        },
    ));
    let used = observed_guard.as_ref().map_or(0, |guard| guard.used());
    let (value, error, panic) = match result {
        Ok(Ok((value, error))) => (value, error, None),
        Ok(Err(error)) => (Value::Null, Some(error.to_string()), None),
        Err(p) => (
            Value::Null,
            None,
            Some(
                p.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "non-string panic".into()),
            ),
        ),
    };
    json!({"name":request.name,"value":value,"error":error,"panic":panic,"used":used,"polls":context.polls.borrow().clone()})
}

fn observe_traversal(arena: &ArenaGraph, request: &Request, config: &PackingTraversal) -> Value {
    struct Context {
        polls: RefCell<Vec<i64>>,
        used: Cell<i64>,
        cancel_at: usize,
        deadline: bool,
    }
    impl limits::Context for Context {
        fn done(&self) -> Option<bool> {
            None
        }
        fn transaction_work_used(&self, used: i64) {
            self.used.set(used);
        }
        fn error(&self) -> Option<limits::ContextError> {
            let mut polls = self.polls.borrow_mut();
            polls.push(self.used.get());
            (self.cancel_at > 0 && polls.len() >= self.cancel_at).then_some(if self.deadline {
                limits::ContextError::DeadlineExceeded
            } else {
                limits::ContextError::Cancelled
            })
        }
    }
    let context = Context {
        polls: RefCell::new(Vec::new()),
        used: Cell::new(0),
        cancel_at: request.cancel_at,
        deadline: request.deadline,
    };
    let find = |id| {
        arena
            .packing_ref_for_id(id)
            .expect("known packing traversal node")
    };
    let ids = |nodes: Vec<binpack_traversal::PackingNodeRef>| {
        nodes
            .into_iter()
            .map(|node| arena.packing_ref_id(node))
            .collect::<Vec<_>>()
    };
    let mut used = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<Value, limits::Error> {
            let mut guard =
                limits::WorkGuard::new(Some(&context), "BinPack", limits::MAX_ENGINE_WORK_UNITS)?;
            guard.set_limit(request.limit);
            let result = match config.operation.as_str() {
                "reachable" => arena
                    .packing_reachable_checked(
                        find(config.start.unwrap()),
                        config.ignore.map(find),
                        config.root.map(find),
                        config.include_nears,
                        &mut guard,
                    )
                    .map(|nodes| json!(ids(nodes))),
                "external" => arena
                    .packing_external_checked(
                        find(config.start.unwrap()),
                        config.root.map(find),
                        config.near,
                        config.container_is_external,
                        &mut guard,
                    )
                    .map(|value| json!(value)),
                "subgraphs" => {
                    let root = config.root.map(|id| match find(id) {
                        binpack_traversal::PackingNodeRef::Node(node) => node,
                        _ => panic!("subgraph root has no concrete node"),
                    });
                    arena.packing_subgraphs_checked(root,config.edges_placed,&mut guard).map(|(packed,contained,cross)|json!({"packed":packed.into_iter().map(ids).collect::<Vec<_>>(),"contained":contained.into_iter().map(ids).collect::<Vec<_>>(),"cross_count":cross}))
                }
                _ => panic!("unknown packing traversal operation"),
            };
            used = guard.used();
            result
        },
    ));
    let (outcome, error, panic) = match result {
        Ok(Ok(outcome)) => (outcome, None, None),
        Ok(Err(error)) => (Value::Null, Some(error.to_string()), None),
        Err(p) => (
            Value::Null,
            None,
            Some(
                p.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "non-string panic".into()),
            ),
        ),
    };
    json!({"name":request.name,"outcome":outcome,"error":error,"panic":panic,"used":used,"polls":context.polls.borrow().clone()})
}

fn snapshot_state(arena: &ArenaGraph, input: &Value) -> Value {
    let nodes = input["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| {
            let id = record["id"].as_i64().unwrap();
            let node = arena
                .packing_ref_for_id(id)
                .expect("snapshot node is decoded");
            let (position, size, _) = arena.packing_bounds_geometry(node);
            (
                id.to_string(),
                json!([
                    position.map(point),
                    format!("{:016x}", size.width.to_bits()),
                    format!("{:016x}", size.height.to_bits())
                ]),
            )
        })
        .collect::<serde_json::Map<String, Value>>();
    let edges = arena
        .edge_records
        .iter()
        .map(|edge| {
            (
                edge.tala_id.to_string(),
                json!(
                    edge.route
                        .read()
                        .iter()
                        .copied()
                        .map(point)
                        .collect::<Vec<_>>()
                ),
            )
        })
        .collect::<serde_json::Map<String, Value>>();
    json!({"nodes":nodes,"edges":edges})
}

fn point(value: Point) -> Value {
    json!([
        format!("{:016x}", value.x.to_bits()),
        format!("{:016x}", value.y.to_bits())
    ])
}

fn bounds(value: Option<(Point, Point)>) -> Value {
    match value {
        Some((low, high)) => json!({"low": point(low), "high": point(high)}),
        None => json!({"low": null, "high": null}),
    }
}

fn corners((low, high): (Option<Point>, Option<Point>)) -> Value {
    json!({"low": low.map(point), "high": high.map(point)})
}

fn run(request: Request) -> Result<Value, Box<dyn std::error::Error>> {
    let input = match graphjson::graphjson_to_graph(&serde_json::to_vec(&request.graph)?) {
        Ok(input) => input,
        Err(error)
            if request.decoded_membership
                || request.decoded_records
                || request.sequence_stage.is_some() =>
        {
            return Ok(json!({"name": request.name, "error": error.to_string()}));
        }
        Err(error) => return Err(error),
    };
    let mut arena = ArenaGraph::from_input(&input);
    if let Some(seed) = request.sequence_stage {
        let observe = arena.sequence_lifecycle_observer();
        let mut rng = go_rng::GoRng::new(seed);
        arena.assign_sequences(&mut rng);
        let groups = arena.sequences.iter().map(|s| json!({
            "vessel":s.vessel_tala_id,
            "members":s.members.iter().map(|n|arena.nodes[n.0 as usize].tala_id).collect::<Vec<_>>(),
            "container":s.container.map(|n|arena.nodes[n.0 as usize].tala_id)
        })).collect::<Vec<_>>();
        let order = arena
            .node_order
            .iter()
            .map(|n| {
                let node = &arena.nodes[n.0 as usize];
                node.sequence
                    .map_or(node.tala_id, |i| arena.sequences[i].vessel_tala_id)
            })
            .collect::<Vec<_>>();
        let edges = arena
            .edge_order
            .read()
            .iter()
            .map(|e| arena.edge_records[e.0 as usize].tala_id)
            .collect::<Vec<_>>();
        let header = arena.owner_graph_member_header(OwnerGraphId(0)).unwrap();
        let map = arena
            .owner_graph_group_map(OwnerGraphId(0), owner_groups::OwnerGroupKind::Sequence)
            .unwrap();
        let mut keys = arena
            .owner_group_map_keys(&map)
            .unwrap()
            .iter()
            .map(|n| n.map(|n| arena.nodes[n.0 as usize].tala_id))
            .collect::<Vec<_>>();
        keys.sort();
        return Ok(
            json!({"name":request.name,"state":observe(&arena),"selection":{"groups":groups,"graph_order":order,"edges":edges,"next_rng":[rng.int63(),rng.int63(),rng.int63()]},
            "lifecycle_witness":{"members":{"nil":header.is_nil(),"len":header.len(),"cap":header.capacity(),"slots":(0..header.capacity()).map(|i|header.read_capacity_slot(i).unwrap().map(|n|arena.nodes[n.0 as usize].tala_id)).collect::<Vec<_>>()},"sequence_keys":keys}}),
        );
    }
    if request.decoded_records {
        return Ok(arena.observe_decoded_records(&request.name, &input));
    }
    if let Some(operations) = request.map_program.as_ref() {
        return Ok(arena.observe_map_program(&request.name, operations));
    }
    if let Some(operations) = request.edge_program.as_ref() {
        return Ok(arena.observe_edge_program(&request.name, operations));
    }
    if request.decoded_membership {
        return Ok(arena.observe_decoded_membership(&request.name));
    }
    // This observes the serialized, pre-layout state. from_input normally
    // initializes fixed nodes for the layout pipeline; undo that generic
    // initialization so current and requested positions stay distinct here.
    for index in 0..arena.nodes.len() {
        arena.nodes[index].position = input.initial_positions[index];
        arena.nodes[index].rect.origin = input.initial_positions[index].unwrap_or_default();
        if let Some(entries) = request.loop_offsets.get(&arena.nodes[index].tala_id) {
            arena.nodes[index].loop_offsets = Some(entries.iter().copied().collect());
        }
    }
    if let Some(config) = request.route_simplify.as_ref() {
        if config["operation"] == "standalone" {
            // Observe graphjson.Unmarshal's caller state, before layout initialization.
            arena.cell_size = arena.source_cell_size;
        }
        return Ok(routing::observe_simplify(
            &mut arena,
            &request.name,
            config,
            request.limit as u64,
        ));
    }
    if let Some(config) = request.owner_preflight.as_ref() {
        return Ok(arena.probe_owner_preflight(
            &request.name,
            config,
            request.limit,
            request.cancel_at,
            request.deadline,
        ));
    }
    if let Some(actions) = request.route_owner.as_ref() {
        return Ok(route_owner_probe::run(&mut arena, &request.name, actions));
    }
    if let Some(config) = request.packing_candidates.as_ref() {
        arena.initialize_shared_topology();
        return Ok(observe_candidates(&mut arena, &request, config));
    }
    if let Some(config) = request.packing_segments.as_ref() {
        arena.initialize_shared_topology();
        return Ok(observe_segments(&mut arena, &request, config));
    }
    if let Some(config) = request.packing_stage.as_ref() {
        arena.initialize_shared_topology();
        return Ok(observe_stage(&mut arena, &request, config));
    }
    if let Some(config) = request.packing_bounds.as_ref() {
        arena.initialize_shared_topology();
        let nodes = arena
            .nodes
            .iter()
            .map(|n| (n.tala_id, n.input_id))
            .collect::<BTreeMap<_, _>>();
        for (&id, parent) in &config.parents {
            arena.nodes[nodes[&id].0 as usize].container = parent.map(|id| nodes[&id]);
        }
        for id in &config.clear_positions {
            arena.clear_position(nodes[id]);
        }
        return Ok(observe_bounds(&arena, &request, config));
    }
    if let Some(config) = request.packing_traversal.as_ref() {
        arena.initialize_shared_topology();
        return Ok(observe_traversal(&arena, &request, config));
    }
    if let Some(validation) = request.packing_validation {
        arena.initialize_shared_topology();
        let nodes = arena
            .nodes
            .iter()
            .map(|n| (n.tala_id, n.input_id))
            .collect::<BTreeMap<_, _>>();
        let edges = arena
            .edge_records
            .iter()
            .map(|e| (e.tala_id, e.input_id))
            .collect::<BTreeMap<_, _>>();
        let root = validation.root.map(|id| nodes[&id]);
        if let Some(order) = validation.node_order {
            arena.node_order = order.iter().map(|id| nodes[id]).collect();
            arena.node_order_membership_valid = false;
        }
        if let Some(order) = validation.edge_order {
            let slots = order
                .iter()
                .map(|id| id.map(|id| edges[&id]))
                .collect::<Vec<_>>();
            let header = arena.allocate_owner_incident_header(&slots, slots.len(), true)?;
            arena.set_owner_graph_edge_header(OwnerGraphId(0), header)?;
        }
        for (id, incidence) in validation.incidence {
            let slots = incidence
                .iter()
                .map(|id| id.map(|id| edges[&id]))
                .collect::<Vec<_>>();
            let header = arena.allocate_owner_incident_header(&slots, slots.len(), true)?;
            arena.set_owner_node_incident_header(nodes[&id], header)?;
        }
        let foreign = arena.create_owner_graph_view(Vec::new(), Vec::new())?;
        for (id, owner) in validation.owners {
            let owner = match owner.as_str() {
                "nil" => None,
                "foreign" => Some(foreign),
                "current" => Some(OwnerGraphId(0)),
                _ => return Err("unknown owner".into()),
            };
            arena.set_owner_node_graph_reference(nodes[&id], owner)?;
        }
        for [id, side] in validation.nil_endpoints {
            arena.set_owner_edge_endpoint(edges[&id], side != 0, None)?;
        }
        struct ValidationContext {
            polls: RefCell<Vec<i64>>,
            used: Cell<i64>,
            cancel_at: usize,
            deadline: bool,
        }
        impl limits::Context for ValidationContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn transaction_work_used(&self, used: i64) {
                self.used.set(used);
            }
            fn error(&self) -> Option<limits::ContextError> {
                let mut polls = self.polls.borrow_mut();
                polls.push(self.used.get());
                (self.cancel_at > 0 && polls.len() >= self.cancel_at).then_some(if self.deadline {
                    limits::ContextError::DeadlineExceeded
                } else {
                    limits::ContextError::Cancelled
                })
            }
        }
        let context = ValidationContext {
            polls: RefCell::new(Vec::new()),
            used: Cell::new(0),
            cancel_at: request.cancel_at,
            deadline: request.deadline,
        };
        let mut used = 0;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<(bool, Vec<EdgeId>), binpack_guard::BinPackRouteFailure> {
                let mut guard = limits::WorkGuard::new(
                    Some(&context),
                    "BinPack",
                    limits::MAX_ENGINE_WORK_UNITS,
                )?;
                guard.set_limit(request.limit);
                let result = arena.bin_pack_complete_routes(root, &mut guard);
                used = guard.used();
                result
            },
        ));
        let (routed, incident, error, panic) = match result {
            Ok(Ok((routed, incident))) => (routed, incident, None, None),
            Ok(Err(failure)) => (
                failure.routed,
                failure.incident,
                Some(failure.error.to_string()),
                None,
            ),
            Err(p) => (
                false,
                Vec::new(),
                None,
                Some(
                    p.downcast_ref::<String>()
                        .cloned()
                        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "non-string panic".into()),
                ),
            ),
        };
        return Ok(
            json!({"name":request.name,"routed":routed,"incident":incident.iter().map(|id|arena.edge_records[id.0 as usize].tala_id).collect::<Vec<_>>(),"used":used,"polls":context.polls.into_inner(),"error":error,"panic":panic}),
        );
    }
    if request.packing_atomic || request.packing_snapshot.is_some() || request.packing_overlaps {
        if request.packing_overlaps && !arena.nodes.is_empty() {
            let node = &mut arena.nodes[0];
            match request.overlap_invalid.as_str() {
                "width_nan" => node.rect.size.width = f64::NAN,
                "negative_width" => node.rect.size.width = -1.,
                "x_inf" => {
                    node.position.as_mut().unwrap().x = f64::INFINITY;
                }
                "margin_inf" => node.layout_margins.top = f64::INFINITY,
                "loop_nan" => node.loop_offsets = Some([(0, f64::NAN)].into_iter().collect()),
                "nil_node" | "nil_incidence" | "nil_primary_edge" | "" => {}
                other => panic!("unknown overlap invalid control {other}"),
            }
        }
        arena.initialize_shared_topology();
        if request.packing_overlaps {
            let root = OwnerGraphId(0);
            match request.overlap_invalid.as_str() {
                "nil_node" => {
                    let mut slots = arena.owner_graph_member_header(root)?.visible_slots();
                    slots.push(None);
                    let header = arena.allocate_owner_member_header(&slots, slots.len())?;
                    arena.set_owner_graph_member_header(root, header)?;
                }
                "nil_incidence" => {
                    let mut slots = arena
                        .owner_node_incident_header(NodeId(0))?
                        .iter_slots()
                        .collect::<Vec<_>>();
                    slots.push(None);
                    let header = arena.allocate_owner_incident_header(&slots, slots.len(), true)?;
                    arena.set_owner_node_incident_header(NodeId(0), header)?;
                }
                "nil_primary_edge" => {
                    let mut slots = arena
                        .owner_graph_edge_header(root)?
                        .iter_slots()
                        .collect::<Vec<_>>();
                    slots.push(None);
                    let header = arena.allocate_owner_incident_header(&slots, slots.len(), true)?;
                    arena.set_owner_graph_edge_header(root, header)?;
                }
                _ => {}
            }
        }
        arena.cell_size = arena.source_cell_size;
        let before = if request.packing_overlaps {
            Value::Null
        } else if request.packing_snapshot.is_some() {
            snapshot_state(&arena, &request.graph)
        } else {
            graphjson::diagnostic_graphjson_state(&arena, &request.graph)?
        };
        struct ProbeContext {
            polls: RefCell<Vec<i64>>,
            used: Cell<i64>,
            cancel_at: usize,
            deadline: bool,
        }
        impl limits::Context for ProbeContext {
            fn done(&self) -> Option<bool> {
                None
            }
            fn transaction_work_used(&self, used: i64) {
                self.used.set(used);
            }
            fn error(&self) -> Option<limits::ContextError> {
                let mut polls = self.polls.borrow_mut();
                polls.push(self.used.get());
                (self.cancel_at > 0 && polls.len() >= self.cancel_at).then_some(if self.deadline {
                    limits::ContextError::DeadlineExceeded
                } else {
                    limits::ContextError::Cancelled
                })
            }
        }
        let context = ProbeContext {
            polls: RefCell::new(Vec::new()),
            used: Cell::new(0),
            cancel_at: request.cancel_at,
            deadline: request.deadline,
        };
        let mut used = 0;
        let mut overlap_pairs: Option<Value> = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<(), limits::Error> {
                let mut guard = limits::WorkGuard::new(
                    Some(&context),
                    "BinPack",
                    limits::MAX_ENGINE_WORK_UNITS,
                )?;
                guard.set_limit(request.limit);
                let result = if request.packing_overlaps {
                    arena
                        .transaction_overlap_snapshot_with_reference_limit(
                            &mut guard,
                            request.overlap_reference_limit.unwrap_or(4_000_000),
                        )
                        .map(|(pairs, exact)| {
                            let convert = |pairs: BTreeSet<(NodeId, NodeId)>| {
                                let mut rows = pairs
                                    .into_iter()
                                    .map(|(a, b)| {
                                        let a = arena.nodes[a.0 as usize].tala_id;
                                        let b = arena.nodes[b.0 as usize].tala_id;
                                        if a < b { [a, b] } else { [b, a] }
                                    })
                                    .collect::<Vec<_>>();
                                rows.sort();
                                rows
                            };
                            overlap_pairs =
                                Some(json!({"spacing":convert(pairs),"exact":convert(exact)}));
                        })
                } else if let Some(snapshot) = &request.packing_snapshot {
                    arena.probe_packing_snapshot(
                        &mut guard,
                        snapshot.routed,
                        snapshot.request_prefix,
                        snapshot.repeats,
                    )
                } else {
                    arena.bin_pack_atomic_with_guard(&mut guard, false)
                };
                used = guard.used();
                result
            },
        ));
        let (error, panic) = match result {
            Ok(result) => (result.err().map(|e| e.to_string()), None),
            Err(p) => (
                None,
                Some(
                    p.downcast_ref::<String>()
                        .cloned()
                        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "non-string panic".into()),
                ),
            ),
        };
        let after = if request.packing_overlaps {
            Value::Null
        } else if request.packing_snapshot.is_some() {
            snapshot_state(&arena, &request.graph)
        } else {
            graphjson::diagnostic_graphjson_state(&arena, &request.graph)?
        };
        if request.packing_overlaps {
            return Ok(
                json!({"name":request.name,"pairs":overlap_pairs,"used":used,"polls":context.polls.into_inner(),"error":error,"panic":panic}),
            );
        }
        return Ok(
            json!({"name":request.name,"before":before,"after":after,"used":used,"polls":context.polls.into_inner(),"error":error,"panic":panic}),
        );
    }
    let selected = request
        .selection
        .iter()
        .map(|id| {
            arena
                .nodes
                .iter()
                .position(|node| node.tala_id == *id)
                .map(|index| NodeId(index as u32))
                .ok_or_else(|| format!("unknown node {id}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let projected = selected
        .iter()
        .map(|id| arena.nodes[id.0 as usize].clone())
        .collect::<Vec<_>>();
    let projected_nodes = selected
        .iter()
        .copied()
        .zip(projected.iter())
        .collect::<Vec<_>>();
    let mut result = json!({
        "name": request.name,
        "ordinary": bounds(arena.ordinary_node_bounds(&selected)),
        "fixed": corners(arena.fixed_node_corners(&selected, true)),
        "projected_ordinary": bounds(ArenaGraph::external_node_bounds(&projected)),
        "projected_fixed": corners(arena.fixed_projected_node_corners(&projected_nodes)),
    });
    if request.packing {
        if selected.is_empty() || selected.iter().any(|&node| arena.position(node).is_none()) {
            return Err("packing observations require nonempty positioned selection".into());
        }
        let points = request
            .packing_points
            .iter()
            .map(|&[x, y]| Point { x, y })
            .collect::<Vec<_>>();
        result["packing"] = arena.diagnostic_bin_pack_bounds(&selected, &points);
    }
    if let Some([x, y]) = request.packing_shift {
        if selected.is_empty() || selected.iter().any(|&node| arena.position(node).is_none()) {
            return Err("packing snapshot requires nonempty positioned selection".into());
        }
        let points = request
            .packing_points
            .iter()
            .map(|&[x, y]| Point { x, y })
            .collect::<Vec<_>>();
        result["hierarchy_snapshot"] =
            arena.diagnostic_bin_pack_hierarchy_snapshot(&selected, &points, Point { x, y });
    }
    if request.extended {
        result["unrounded_fixed"] = corners(arena.fixed_node_corners(&selected, false));
        // Graph.Area requires a positioned graph. Keep invalid-state bounds
        // observable without calling a source API that dereferences nil.
        result["component_area"] = if selected.iter().all(|id| arena.position(*id).is_some()) {
            json!(format!(
                "{:016x}",
                arena.component_graph_area(&selected).to_bits()
            ))
        } else {
            Value::Null
        };
    }
    Ok(result)
}

pub fn diagnostic_node_bounds_probe(value: Value) -> Result<Value, Box<dyn std::error::Error>> {
    if value.as_array().is_some_and(|rows| {
        !rows.is_empty() && rows.iter().all(|row| row.get("hierarchy_ovg").is_some())
    }) {
        return crate::internal::routing::hierarchy_ovg_probe(value);
    }
    #[cfg(feature = "diagnostic-traces")]
    if value.as_array().is_some_and(|rows| {
        !rows.is_empty() && rows.iter().all(|row| row.get("geometry").is_some())
    }) {
        return Ok(crate::internal::routing::diagnostic_box_intersections(
            value,
        ));
    }

    #[cfg(feature = "diagnostic-traces")]
    if value.as_array().is_some_and(|rows| {
        !rows.is_empty() && rows.iter().all(|row| row.get("static_index").is_some())
    }) {
        return Ok(crate::internal::routing::static_index_probe(value));
    }
    #[cfg(feature = "diagnostic-traces")]
    if value.as_array().is_some_and(|rows| {
        !rows.is_empty() && rows.iter().all(|row| row.get("edge_set").is_some())
    }) {
        return Ok(crate::internal::routing::edge_set_probe(value));
    }
    let requests: Vec<Request> = serde_json::from_value(value)?;
    Ok(Value::Array(
        requests
            .into_iter()
            .map(run)
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PackingStage {
    groups: Vec<Vec<i64>>,
    seed: bool,
    root: [f64; 2],
    edges: bool,
    restore: bool,
    aliases: Vec<[i64; 2]>,
    clear: Vec<i64>,
}
fn observe_stage(arena: &mut ArenaGraph, request: &Request, config: &PackingStage) -> Value {
    use super::binpack_traversal::PackingNodeRef;
    struct Context {
        polls: RefCell<Vec<i64>>,
        used: Cell<i64>,
        cancel_at: usize,
        deadline: bool,
    }
    impl limits::Context for Context {
        fn done(&self) -> Option<bool> {
            None
        }
        fn transaction_work_used(&self, used: i64) {
            self.used.set(used);
        }
        fn error(&self) -> Option<limits::ContextError> {
            let mut polls = self.polls.borrow_mut();
            polls.push(self.used.get());
            (self.cancel_at > 0 && polls.len() >= self.cancel_at).then_some(if self.deadline {
                limits::ContextError::DeadlineExceeded
            } else {
                limits::ContextError::Cancelled
            })
        }
    }
    let mut nodes = arena
        .nodes
        .iter()
        .map(|n| (n.tala_id, arena.packing_ref_for_id(n.tala_id).unwrap()))
        .collect::<BTreeMap<_, _>>();
    for id in arena
        .clusters
        .iter()
        .map(|c| c.vessel_tala_id)
        .chain(arena.sequences.iter().map(|s| s.vessel_tala_id))
    {
        nodes.insert(id, arena.packing_ref_for_id(id).unwrap());
    }
    for n in nodes.values() {
        if let PackingNodeRef::Node(n) = n {
            arena.materialize_owner_node_point(*n).unwrap();
        }
    }
    for id in &config.clear {
        if let PackingNodeRef::Node(n) = nodes[id] {
            arena.clear_position(n);
        } else {
            panic!("clear requires concrete node");
        }
    }
    for [a, b] in &config.aliases {
        if let (PackingNodeRef::Node(a), PackingNodeRef::Node(b)) = (nodes[a], nodes[b]) {
            let pointer = arena.owner_node_point(b).unwrap();
            arena.bind_owner_node_point(a, pointer).unwrap();
        } else {
            panic!("alias requires concrete node");
        }
    }
    let groups = config
        .groups
        .iter()
        .map(|g| g.iter().map(|id| nodes[id]).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let context = Context {
        polls: RefCell::new(Vec::new()),
        used: Cell::new(0),
        cancel_at: request.cancel_at,
        deadline: request.deadline,
    };
    let mut saved = super::binpack_staging::PackingStaging::default();
    let mut observed_guard = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<(), limits::Error> {
            observed_guard = Some(limits::WorkGuard::new(
                Some(&context),
                "BinPack",
                request.limit,
            )?);
            let guard = observed_guard.as_mut().unwrap();
            arena.packing_stage_checked(
                &groups,
                config.seed,
                Point {
                    x: config.root[0],
                    y: config.root[1],
                },
                config.edges,
                &mut saved,
                guard,
            )?;
            if config.restore {
                saved.restore_positions_checked(arena, guard)?;
                saved.restore_routes_checked(arena, guard)?;
            }
            Ok(())
        },
    ));
    let (error, panic) = match result {
        Ok(Ok(())) => (None, None),
        Ok(Err(e)) => (Some(e.to_string()), None),
        Err(p) => (
            None,
            Some(
                p.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "non-string panic".into()),
            ),
        ),
    };
    let positions = nodes
        .iter()
        .map(|(id, n)| json!({"id":id,"value":arena.packing_bounds_geometry(*n).0.map(point)}))
        .collect::<Vec<_>>();
    let mut aliases = Vec::new();
    for (&id, &a) in &nodes {
        for (&other, &b) in &nodes {
            let equal = match (a, b) {
                (PackingNodeRef::Node(a), PackingNodeRef::Node(b)) => arena
                    .owner_node_point(a)
                    .unwrap()
                    .same_allocation(&arena.owner_node_point(b).unwrap()),
                _ => {
                    a == b
                        || (arena.packing_bounds_geometry(a).0.is_none()
                            && arena.packing_bounds_geometry(b).0.is_none())
                }
            };
            if equal {
                aliases.push([id, other]);
            }
        }
    }
    let mut edges = BTreeMap::new();
    for &n in nodes.values() {
        for e in arena.packing_incidents(n) {
            edges.insert(arena.edge_records[e.0 as usize].tala_id, e);
        }
    }
    let routes=edges.iter().map(|(id,e)|json!({"id":id,"points":arena.edge_records[e.0 as usize].route.read().iter().copied().map(point).collect::<Vec<_>>()})).collect::<Vec<_>>();
    let saved_positions=saved.positions.iter().map(|s|{let retained=match(s.node,s.point.as_ref()){(PackingNodeRef::Node(n),Some(p))=>arena.owner_node_point(n).unwrap().same_allocation(p),_=>true};json!({"id":arena.packing_ref_id(s.node),"value":point(s.value.unwrap_or_default()),"retained":retained})}).collect::<Vec<_>>();
    let saved_routes=saved.routes.iter().map(|(e,ps)|json!({"id":arena.edge_records[e.0 as usize].tala_id,"points":ps.iter().copied().map(point).collect::<Vec<_>>()})).collect::<Vec<_>>();
    json!({"name":request.name,"error":error,"panic":panic,"used":observed_guard.as_ref().map_or(0,|g|g.used()),"polls":context.polls.borrow().clone(),"positions":positions,"aliases":aliases,"routes":routes,"saved_positions":saved_positions,"saved_routes":saved_routes})
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PackingSegmentRequest {
    operation: String,
    nodes: Vec<Option<i64>>,
    edges: Vec<Option<i64>>,
    edge: Option<i64>,
    packed: Vec<Vec<Option<i64>>>,
    remaining: Vec<Vec<Option<i64>>>,
    root: Option<i64>,
    routed: bool,
    horizontal: bool,
    xgap: f64,
    ygap: f64,
    clear: Vec<i64>,
    incidence: BTreeMap<i64, Vec<Option<i64>>>,
    nil_endpoints: Vec<[i64; 2]>,
    flags: BTreeMap<i64, bool>,
    parents: BTreeMap<i64, Option<i64>>,
}
fn observe_segments(
    arena: &mut ArenaGraph,
    request: &Request,
    config: &PackingSegmentRequest,
) -> Value {
    struct Context {
        polls: RefCell<Vec<i64>>,
        used: Cell<i64>,
        cancel_at: usize,
        deadline: bool,
    }
    impl limits::Context for Context {
        fn done(&self) -> Option<bool> {
            None
        }
        fn transaction_work_used(&self, used: i64) {
            self.used.set(used);
        }
        fn error(&self) -> Option<limits::ContextError> {
            let mut polls = self.polls.borrow_mut();
            polls.push(self.used.get());
            (self.cancel_at > 0 && polls.len() >= self.cancel_at).then_some(if self.deadline {
                limits::ContextError::DeadlineExceeded
            } else {
                limits::ContextError::Cancelled
            })
        }
    }
    let nodes = arena
        .nodes
        .iter()
        .map(|n| (n.tala_id, n.input_id))
        .collect::<BTreeMap<_, _>>();
    let edges = arena
        .edge_records
        .iter()
        .map(|e| (e.tala_id, e.input_id))
        .collect::<BTreeMap<_, _>>();
    for id in &config.clear {
        arena.clear_position(nodes[id]);
    }
    for (&id, &flag) in &config.flags {
        arena
            .set_owner_cluster_vessel_flag(nodes[&id], flag)
            .unwrap();
    }
    for (&id, parent) in &config.parents {
        arena.nodes[nodes[&id].0 as usize].container = parent.map(|id| nodes[&id]);
    }
    for (id, ids) in &config.incidence {
        let slots = ids
            .iter()
            .map(|id| id.map(|id| edges[&id]))
            .collect::<Vec<_>>();
        let header = arena
            .allocate_owner_incident_header(&slots, slots.len(), true)
            .unwrap();
        arena
            .set_owner_node_incident_header(nodes[id], header)
            .unwrap();
    }
    for &[id, side] in &config.nil_endpoints {
        arena
            .set_owner_edge_endpoint(edges[&id], side != 0, None)
            .unwrap();
    }
    let find = |id| arena.packing_ref_for_id(id).expect("known segment node");
    let selected = config
        .nodes
        .iter()
        .map(|id| id.map(find))
        .collect::<Vec<_>>();
    let selected_edges = config
        .edges
        .iter()
        .map(|id| id.map(|id| edges[&id]))
        .collect::<Vec<_>>();
    let groups = |gs: &[Vec<Option<i64>>]| {
        gs.iter()
            .map(|g| g.iter().map(|id| id.map(find)).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    };
    let packed = groups(&config.packed);
    let remaining = groups(&config.remaining);
    let root = config.root.map(find);
    let context = Context {
        polls: RefCell::new(Vec::new()),
        used: Cell::new(0),
        cancel_at: request.cancel_at,
        deadline: request.deadline,
    };
    let mut state = super::binpack_segments::PackingSegments::default();
    let mut permissions = [true; 4];
    let mut observed_guard = None;
    let mut entered = false;
    let mut value = Value::Null;
    let encode = |ss: Option<Vec<(Point, Point)>>| {
        ss.map(|ss| {
            ss.into_iter()
                .map(|(a, b)| json!([point(a), point(b)]))
                .collect::<Vec<_>>()
        })
    };
    let encode_routed = |ss: Option<Vec<(EdgeId, Point, Point)>>| {
        ss.map(|ss|ss.into_iter().map(|(e,a,b)|json!({"edge":arena.edge_records[e.0 as usize].tala_id,"start":point(a),"end":point(b)})).collect::<Vec<_>>())
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<(), limits::Error> {
            observed_guard = Some(limits::WorkGuard::new(
                Some(&context),
                "BinPack",
                request.limit,
            )?);
            let guard = observed_guard.as_mut().unwrap();
            entered = true;
            match config.operation.as_str() {
                "smallest" => {
                    let result = arena.packing_smallest_deltas_checked(&remaining, guard);
                    let (x, y) = result.as_ref().copied().unwrap_or_default();
                    value = json!([
                        format!("{:016x}", x.to_bits()),
                        format!("{:016x}", y.to_bits())
                    ]);
                    result.map(|_| ())
                }
                "routed" => match arena.packing_routed_segments_checked(
                    &selected_edges,
                    config.horizontal,
                    guard,
                ) {
                    Ok(v) => {
                        value = json!(encode_routed(v));
                        Ok(())
                    }
                    Err(f) => {
                        value = json!(encode_routed(f.value));
                        Err(f.error)
                    }
                },
                "edge" | "routes" => {
                    let result = if config.operation == "edge" {
                        arena.packing_estimate_edge_checked(
                            config.edge.map(|id| edges[&id]),
                            config.xgap,
                            config.ygap,
                            guard,
                        )
                    } else {
                        arena.packing_estimate_routes_checked(
                            &selected,
                            config.xgap,
                            config.ygap,
                            guard,
                        )
                    };
                    match result {
                        Ok(v) => {
                            value = json!(encode(v));
                            Ok(())
                        }
                        Err(f) => {
                            value = json!(encode(f.value));
                            Err(f.error)
                        }
                    }
                }
                "prepare" => arena.packing_initial_segments_checked(
                    &packed,
                    &remaining,
                    root,
                    config.routed,
                    &mut state,
                    guard,
                ),
                "permissions" => {
                    arena.packing_permissions_checked(root, config.routed, &mut permissions, guard)
                }
                _ => panic!("unknown segment operation"),
            }
        },
    ));
    if entered {
        if config.operation == "prepare" {
            value = json!({"children":(!state.children.is_empty()).then(||state.children.iter().map(|&n|arena.packing_ref_id(n)).collect::<Vec<_>>()),"segments":encode(state.segments)});
        } else if config.operation == "permissions" {
            value = json!(permissions);
        }
    }
    let (error, panic) = match result {
        Ok(Ok(())) => (None, None),
        Ok(Err(e)) => (Some(e.to_string()), None),
        Err(p) => (
            None,
            Some(
                p.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "non-string panic".into()),
            ),
        ),
    };
    json!({"name":request.name,"value":value,"error":error,"panic":panic,"used":observed_guard.as_ref().map_or(0,|g|g.used()),"polls":context.polls.borrow().clone()})
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PackingCandidateRequest {
    operation: String,
    nodes: Vec<Option<i64>>,
    packed: Vec<Vec<Option<i64>>>,
    root: Option<i64>,
    routed: bool,
    consider_within: bool,
    point: Option<[f64; 2]>,
    boxes: Vec<[Option<[f64; 2]>; 2]>,
    root_point: Option<[f64; 2]>,
    occupied: Vec<[f64; 2]>,
    clear: Vec<i64>,
    incidence: BTreeMap<i64, Vec<Option<i64>>>,
    nil_endpoints: Vec<[i64; 2]>,
    hierarchies: BTreeMap<i64, bool>,
    parents: BTreeMap<i64, Option<i64>>,
}
fn observe_candidates(
    arena: &mut ArenaGraph,
    request: &Request,
    config: &PackingCandidateRequest,
) -> Value {
    struct Context {
        polls: RefCell<Vec<i64>>,
        used: Cell<i64>,
        cancel_at: usize,
        deadline: bool,
    }
    impl limits::Context for Context {
        fn done(&self) -> Option<bool> {
            None
        }
        fn transaction_work_used(&self, used: i64) {
            self.used.set(used);
        }
        fn error(&self) -> Option<limits::ContextError> {
            let mut polls = self.polls.borrow_mut();
            polls.push(self.used.get());
            (self.cancel_at > 0 && polls.len() >= self.cancel_at).then_some(if self.deadline {
                limits::ContextError::DeadlineExceeded
            } else {
                limits::ContextError::Cancelled
            })
        }
    }
    let nodes = arena
        .nodes
        .iter()
        .map(|n| (n.tala_id, n.input_id))
        .collect::<BTreeMap<_, _>>();
    let edges = arena
        .edge_records
        .iter()
        .map(|e| (e.tala_id, e.input_id))
        .collect::<BTreeMap<_, _>>();
    for id in &config.clear {
        arena.clear_position(nodes[id]);
    }
    for (&id, &flag) in &config.hierarchies {
        if flag {
            arena.assign_hierarchy_member(nodes[&id], 0, None, 0, 0);
        } else {
            arena.set_owner_node_hierarchy(nodes[&id], None).unwrap();
        }
    }
    for (&id, parent) in &config.parents {
        arena.nodes[nodes[&id].0 as usize].container = parent.map(|id| nodes[&id]);
    }
    for (id, ids) in &config.incidence {
        let slots = ids
            .iter()
            .map(|id| id.map(|id| edges[&id]))
            .collect::<Vec<_>>();
        let header = arena
            .allocate_owner_incident_header(&slots, slots.len(), true)
            .unwrap();
        arena
            .set_owner_node_incident_header(nodes[id], header)
            .unwrap();
    }
    for &[id, side] in &config.nil_endpoints {
        arena
            .set_owner_edge_endpoint(edges[&id], side != 0, None)
            .unwrap();
    }
    let find = |id| arena.packing_ref_for_id(id).expect("known segment node");
    let selected = config
        .nodes
        .iter()
        .map(|id| id.map(find))
        .collect::<Vec<_>>();

    let packed = config
        .packed
        .iter()
        .map(|g| g.iter().map(|id| id.map(find)).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let root = config.root.map(find);
    let xy = |p: [f64; 2]| Point { x: p[0], y: p[1] };
    let boxes = config
        .boxes
        .iter()
        .map(|b| (b[0].map(xy), b[1].map(xy)))
        .collect::<Vec<_>>();
    let occupied = config.occupied.iter().copied().map(xy).collect::<Vec<_>>();
    let context = Context {
        polls: RefCell::new(Vec::new()),
        used: Cell::new(0),
        cancel_at: request.cancel_at,
        deadline: request.deadline,
    };

    let encode_boxes = |boxes: Option<Vec<super::binpack_candidates::HierarchyBox>>| {
        boxes.map(|b| {
            b.into_iter()
                .map(|(a, b)| json!([a.map(point), b.map(point)]))
                .collect::<Vec<_>>()
        })
    };
    let encode_points =
        |points: Option<Vec<Point>>| points.map(|p| p.into_iter().map(point).collect::<Vec<_>>());
    let mut observed_guard = None;
    let mut value = Value::Null;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<(), limits::Error> {
            observed_guard = Some(limits::WorkGuard::new(
                Some(&context),
                "BinPack",
                request.limit,
            )?);
            let guard = observed_guard.as_mut().unwrap();
            match config.operation.as_str() {
                "hierarchy" => match arena.packing_hierarchy_boxes_checked(&packed, guard) {
                    Ok(v) => {
                        value = json!(encode_boxes(v));
                        Ok(())
                    }
                    Err(f) => {
                        value = json!(encode_boxes(f.value));
                        Err(f.error)
                    }
                },
                "inside" => {
                    let result = ArenaGraph::packing_point_in_hierarchy_checked(
                        &boxes,
                        config.point.map(xy),
                        guard,
                    );
                    value = json!(result.as_ref().copied().unwrap_or(false));
                    result.map(|_| ())
                }
                "placements" => match arena.packing_placement_candidates_checked(
                    &selected,
                    root,
                    config.consider_within,
                    config.routed,
                    guard,
                ) {
                    Ok(v) => {
                        value = json!(encode_points(v));
                        Ok(())
                    }
                    Err(f) => {
                        value = json!(encode_points(f.value));
                        Err(f.error)
                    }
                },
                "prepare" => {
                    let (boxes, candidates, low) = arena.packing_prepare_candidates_checked(
                        &packed,
                        root,
                        config.root_point.map(xy),
                        config.routed,
                        &occupied,
                        guard,
                    )?;
                    value = json!({"hierarchy":boxes.into_iter().map(|(a,b)|json!([point(a),point(b)])).collect::<Vec<_>>(),"candidates":encode_points(Some(candidates)),"packed_low":point(low)});
                    Ok(())
                }
                _ => panic!("unknown candidate operation"),
            }
        },
    ));
    let (error, panic) = match result {
        Ok(Ok(())) => (None, None),
        Ok(Err(e)) => (Some(e.to_string()), None),
        Err(p) => (
            None,
            Some(
                p.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "non-string panic".into()),
            ),
        ),
    };
    json!({"name":request.name,"value":value,"error":error,"panic":panic,"used":observed_guard.as_ref().map_or(0,|g|g.used()),"polls":context.polls.borrow().clone()})
}
