// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
#![cfg(feature = "diagnostic-traces")]
use super::*;
use crate::internal::owner_points::SharedOwnerPoint;
use serde_json::{Value, json};

fn point_record(point: &Option<SharedOwnerPoint>, identities: &mut Vec<SharedOwnerPoint>) -> Value {
    let Some(point) = point else {
        return json!({"id":0,"value":null});
    };
    let index = identities
        .iter()
        .position(|other| point.same_allocation(other))
        .unwrap_or_else(|| {
            identities.push(point.clone());
            identities.len() - 1
        });
    let value = point.get();
    json!({"id":index+1,"value":[format!("{:016x}", value.x.to_bits()),format!("{:016x}", value.y.to_bits())]})
}
fn observe(
    graph: &ArenaGraph,
    retained: &[RoutePoints],
    kept: &[Option<SharedOwnerPoint>],
    result: &Option<RoutePoints>,
    include_nodes: bool,
    include_graph: bool,
) -> Value {
    let mut identities = Vec::new();
    let points = kept
        .iter()
        .map(|p| point_record(p, &mut identities))
        .collect::<Vec<_>>();
    let mut headers = BTreeMap::new();
    for (i, e) in graph.edge_records.iter().enumerate() {
        headers.insert(format!("edge-{i:04}"), e.route.clone());
    }
    for (i, h) in retained.iter().enumerate() {
        headers.insert(format!("retained-{i:04}"), h.clone());
    }
    if let Some(result) = result {
        headers.insert("result".into(), result.clone());
    }
    let mut slots = BTreeMap::new();
    let mut routes = BTreeMap::new();
    for (name, h) in headers {
        let full=(0..h.capacity()).map(|i| {
            let address=h.header().slot_address(i).unwrap();let next=slots.len()+1;let id=*slots.entry(address).or_insert(next);
            json!({"slot":id,"point":point_record(&h.header().read_capacity_slot(i).unwrap(),&mut identities)})
        }).collect::<Vec<_>>();
        routes.insert(
            name,
            json!({"nil":h.header().is_nil(),"len":h.len(),"cap":h.capacity(),"slots":full}),
        );
    }
    let mut output = json!({"points":points,"routes":routes});
    if include_nodes {
        output["node_points"] = json!(
            graph
                .owner_graph_view(crate::internal::OwnerGraphId(0))
                .unwrap()
                .nodes
                .require_nodes()
                .map(|node| point_record(
                    &graph.owner_node_point(node).unwrap().shared_point(),
                    &mut identities
                ))
                .collect::<Vec<_>>()
        );
    }
    if include_graph {
        let owner = crate::internal::OwnerGraphId(0);
        let costs = graph.owner_graph_routing_costs(owner).unwrap();
        output["graph_state"] = json!(
            [
                graph.owner_graph_cell_size(owner).unwrap(),
                costs.0,
                costs.1,
                costs.2
            ]
            .map(|n| format!("{:016x}", n.to_bits()))
        );
    }
    output
}
struct ProbeContext<'a> {
    config: &'a Value,
    polls: std::cell::Cell<u64>,
    callback_trace: std::cell::RefCell<(BTreeMap<String, usize>, Vec<usize>)>,
    label_work: std::cell::Cell<i64>,
    point: Option<SharedOwnerPoint>,
    tree: Option<std::sync::Arc<crate::internal::owner_tree_map::OwnerTreeRecord>>,
}
impl crate::internal::limits::Context for ProbeContext<'_> {
    fn label_placement_work_used(&self, used: i64) {
        self.label_work.set(used);
    }

    fn done(&self) -> Option<bool> {
        match self.config["context"].as_str() {
            Some("open") => Some(false),
            Some("closed") => Some(true),
            _ => None,
        }
    }
    fn error(&self) -> Option<crate::internal::limits::ContextError> {
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        if crate::internal::trace_env_enabled("D2RS_TRACE_ROUTE_CALLBACK_SITES") {
            let stack = std::backtrace::Backtrace::force_capture().to_string();
            let mut trace = self.callback_trace.borrow_mut();
            let next = trace.0.len();
            let id = *trace.0.entry(stack).or_insert(next);
            trace.1.push(id);
        }
        if self.config["tree_mutate_at"].as_u64() == Some(poll)
            && let Some(tree) = &self.tree
        {
            let mut fields = tree.fields();
            fields.orientation = self.config["tree_orientation"].as_i64().unwrap();
            tree.set_fields(fields);
        }
        if self.config["mutate_at"].as_u64() == Some(poll)
            && let Some(point) = &self.point
        {
            point.set(Point {
                x: 9876.5,
                y: -1234.5,
            });
        }
        if self.config["panic_at"].as_u64() == Some(poll) {
            panic!("route context panic");
        }
        if self.config["cancel_at"]
            .as_u64()
            .is_some_and(|at| at > 0 && poll >= at)
        {
            Some(if self.config["deadline"].as_bool().unwrap_or(false) {
                crate::internal::limits::ContextError::DeadlineExceeded
            } else {
                crate::internal::limits::ContextError::Cancelled
            })
        } else {
            None
        }
    }
}
// Keep work observation separate from the caller's actual context behavior.
struct LabelProbeContext<'a> {
    caller: &'a dyn crate::internal::limits::Context,
    used: &'a std::cell::Cell<i64>,
}
impl crate::internal::limits::Context for LabelProbeContext<'_> {
    fn done(&self) -> Option<bool> {
        self.caller.done()
    }
    fn error(&self) -> Option<crate::internal::limits::ContextError> {
        self.caller.error()
    }
    fn label_placement_work_used(&self, used: i64) {
        self.used.set(used);
    }
}
pub(in crate::internal) fn observe_simplify(
    graph: &mut ArenaGraph,
    name: &str,
    config: &Value,
    limit: u64,
) -> Value {
    let operation = config["operation"].as_str().unwrap_or_default();
    if matches!(operation, "labels" | "place-labels") {
        // graphjson.Unmarshal creates label records without private fixed bits.
        // ArenaGraph::from_input also supports the authored D2 adapter, which
        // infers these bits during layout initialization; this probe precedes it.
        for node in &mut graph.nodes {
            node.label_position_fixed = false;
            node.icon_position_fixed = false;
        }
        for edge in &mut graph.edge_records {
            edge.label_position_fixed = false;
        }
        if let Some(indices) = config["fixed_labels"].as_array() {
            for index in indices {
                graph.edge_records[index.as_u64().unwrap() as usize].label_position_fixed = true;
            }
        }
        if let Some(indices) = config["fixed_node_labels"].as_array() {
            for index in indices {
                graph.nodes[index.as_u64().unwrap() as usize].label_position_fixed = true;
            }
        }
        if let Some(indices) = config["fixed_icons"].as_array() {
            for index in indices {
                graph.nodes[index.as_u64().unwrap() as usize].icon_position_fixed = true;
            }
        }
    }
    let imports = matches!(operation, "port-imports" | "imports");
    let finalized = imports
        || matches!(
            operation,
            "port-containers"
                | "port-near"
                | "port-indexed"
                | "port-imports"
                | "containers"
                | "near"
                | "indexed"
                | "imports"
        );
    let lifecycle = finalized
        || matches!(
            operation,
            "port-centers"
                | "port-isolation"
                | "centers"
                | "isolated"
                | "containers"
                | "near"
                | "indexed"
                | "imports"
        );
    let construction = lifecycle
        || matches!(
            operation,
            "connections"
                | "port-connections"
                | "tunnel-connections"
                | "tunnel-ports"
                | "ports"
                | "intersections"
                | "edge-fill"
                | "trees"
                | "tree-ports"
                | "boundary-layers"
                | "port-boundaries"
                | "boundaries"
                | "tunnels"
        );
    let boundary_stage = matches!(
        operation,
        "centers"
            | "isolated"
            | "containers"
            | "near"
            | "indexed"
            | "imports"
            | "connections"
            | "boundary-layers"
            | "port-boundaries"
            | "boundaries"
            | "tunnels"
    );
    let detached_tunnels = matches!(operation, "tunnel-build" | "tunnel-between");
    let tunnel_ranges = operation == "tunnel-ranges";
    let include_nodes = detached_tunnels
        || tunnel_ranges
        || construction
        || matches!(
            operation,
            "nudge"
                | "balance"
                | "branch"
                | "swap"
                | "crosshatch"
                | "trace"
                | "standalone"
                | "labels"
                | "place-labels"
        )
        || operation == "finalize";
    if include_nodes {
        for node in graph
            .owner_graph_view(crate::internal::OwnerGraphId(0))
            .unwrap()
            .nodes
            .require_nodes()
        {
            graph.materialize_owner_node_point(node).unwrap();
        }
    }
    if let Some(rows) = config["node_bits"].as_array() {
        for row in rows {
            let node = crate::NodeId(row["node"].as_u64().unwrap() as u32);
            for (key, value) in row.as_object().unwrap() {
                if key == "node" {
                    continue;
                }
                let value =
                    f64::from_bits(u64::from_str_radix(value.as_str().unwrap(), 16).unwrap());
                match key.as_str() {
                    "width" => graph.nodes[node.0 as usize].rect.size.width = value,
                    "height" => graph.nodes[node.0 as usize].rect.size.height = value,
                    "x" | "y" => {
                        let mut point = graph.position(node).unwrap();
                        if key == "x" {
                            point.x = value;
                        } else {
                            point.y = value;
                        }
                        graph.set_position(node, point);
                    }
                    _ => panic!("unknown probe node field"),
                }
            }
        }
    }
    if let Some(aliases) = config["node_aliases"].as_array() {
        for alias in aliases {
            let point = graph
                .owner_node_point(crate::NodeId(alias["from"].as_u64().unwrap() as u32))
                .unwrap();
            graph
                .bind_owner_node_point(crate::NodeId(alias["node"].as_u64().unwrap() as u32), point)
                .unwrap();
        }
    }
    if let Some(shares) = config["node_shares"].as_array() {
        for share in shares {
            let node = crate::NodeId(share["node"].as_u64().unwrap() as u32);
            let point = graph.owner_node_point(node).unwrap().shared_point();
            graph.edge_records[share["edge"].as_u64().unwrap() as usize]
                .route
                .header()
                .write_capacity_slot(share["slot"].as_u64().unwrap() as usize, point)
                .unwrap();
        }
    }
    if let Some(curves) = config["curves"].as_array() {
        for edge in curves {
            graph.edge_records[edge.as_u64().unwrap() as usize].is_curve = true;
        }
    }
    if let Some(shares) = config["shares"].as_array() {
        for s in shares {
            let index = |k: &str| s[k].as_u64().unwrap() as usize;
            let point = graph.edge_records[index("from_edge")]
                .route
                .header()
                .read_capacity_slot(index("from_slot"))
                .unwrap();
            graph.edge_records[index("edge")]
                .route
                .header()
                .write_capacity_slot(index("slot"), point)
                .unwrap();
        }
    }
    if let Some(windows) = config["windows"].as_array() {
        for w in windows {
            let i = |k: &str| w[k].as_u64().unwrap() as usize;
            graph.edge_records[i("edge")]
                .route
                .replace(RoutePoints::from_header(
                    graph.edge_records[i("from_edge")]
                        .route
                        .header()
                        .reslice_with_capacity(i("low"), i("high"), i("max"))
                        .unwrap(),
                ));
        }
    }
    let retained = graph
        .edge_records
        .iter()
        .map(|e| e.route.clone())
        .collect::<Vec<_>>();
    let kept = retained
        .iter()
        .flat_map(|h| (0..h.capacity()).map(|i| h.header().read_capacity_slot(i).unwrap()))
        .collect::<Vec<_>>();
    let mut before = observe(
        graph,
        &retained,
        &kept,
        &None,
        include_nodes,
        config["operation"] == "standalone",
    );
    if matches!(operation, "labels" | "place-labels") {
        before["labels"] = label_state(graph);
    }
    let budget = route_search_guard::RouteStageWork::new(limit);
    let mut route = None;
    let mode = config["mode"].as_str().unwrap_or("once");
    let edge = config["edge"].as_u64().unwrap_or(0) as usize;
    let context = ProbeContext {
        config,
        polls: std::cell::Cell::new(0),
        callback_trace: std::cell::RefCell::new((BTreeMap::new(), Vec::new())),
        label_work: std::cell::Cell::new(0),
        point: kept.first().cloned().flatten(),
        tree: config["tree_node"].as_u64().and_then(|node| {
            graph
                .owner_graph_node_to_tree_entry(
                    crate::internal::OwnerGraphId(0),
                    Some(crate::NodeId(node as u32)),
                )
                .unwrap()
                .flatten()
                .and_then(|tree| graph.retain_probe_tree(tree))
        }),
    };
    let work_error = || {
        format!(
            "TALA route-stage work limit exceeded: TALA SimplifyEdgeRoutes work exceeds limit {limit}"
        )
    };
    let ports_runtime = route_stage_guard::RuntimeRouteContext;
    let ports_context: &dyn crate::internal::limits::Context = if config["context"]
        .as_str()
        .is_some_and(|value| value != "nil")
    {
        &context
    } else {
        &ports_runtime
    };
    let ports_budget = route_search_guard::OvgBuildBudget::with_context(ports_context);
    let mut tunnel_output = Value::Null;
    let mut range_output = Value::Null;
    let mut range_horizontal = false;
    let mut range_initialized = false;
    let mut ports_initialized = false;
    let mut imported_routes = Vec::<super::ovg_import::ImportedRoute>::new();
    let mut ports = super::ovg_ports::PortBuild::default();
    let mut final_state =
        (operation == "finalize").then(|| super::finalize::probe::State::new(graph, config));
    let final_before = final_state
        .as_ref()
        .map(super::finalize::probe::State::observe);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(state) = final_state.as_mut() {
            return state.run(graph, config, limit, ports_context);
        }

        if detached_tunnels {
            let failure = || {
                ports_budget
                    .take_context_error()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "TALA OVG resource limit exceeded".into())
            };
            ports_budget.check().map_err(|_| failure())?;
            range_initialized = true;
            let result = if operation == "tunnel-between" {
                let node = |key: &str, default: i64| config[key].as_i64().unwrap_or(default);
                let a = node("first", 0);
                let b = node("second", 1);
                super::ovg_tunnels::between(
                    graph,
                    &graph.graph_node_order(),
                    (a >= 0).then_some(crate::NodeId(a as u32)),
                    (b >= 0).then_some(crate::NodeId(b as u32)),
                    &ports_budget,
                )
            } else {
                super::ovg_tunnels::build(graph, &graph.graph_node_order(), &ports_budget).map(Some)
            }
            .map_err(|_| failure())?;
            if let Some(tunnels) = result {
                tunnel_output = super::ovg_tunnels::observe(graph, &tunnels);
            }
            return Ok(());
        }
        if tunnel_ranges {
            let failure = || {
                ports_budget
                    .take_context_error()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "TALA OVG resource limit exceeded".into())
            };
            ports_budget.check().map_err(|_| failure())?;
            range_initialized = true;
            let first = crate::NodeId(config["first"].as_u64().unwrap_or(0) as u32);
            let second = crate::NodeId(config["second"].as_u64().unwrap_or(1) as u32);
            if let Some((ranges, horizontal)) = super::tunnels::tunnel_ranges_between(
                graph,
                &graph.graph_node_order(),
                first,
                second,
                config["filter_short"].as_bool().unwrap_or(false),
                &ports_budget,
            )
            .map_err(|_| failure())?
            {
                range_horizontal = horizontal;
                range_output = json!(
                    ranges
                        .iter()
                        .map(|(lo, hi)| [
                            format!("{:016x}", lo.to_bits()),
                            format!("{:016x}", hi.to_bits())
                        ])
                        .collect::<Vec<_>>()
                );
            }
            return Ok(());
        }
        if construction {
            let failure = || {
                ports_budget
                    .take_context_error()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "TALA OVG resource limit exceeded".into())
            };
            ports_budget.check().map_err(|_| failure())?;
            ports_initialized = true;
            let nodes = graph.graph_node_order();
            ports
                .add_ports(graph, &nodes, &ports_budget)
                .map_err(|_| failure())?;
            if !matches!(
                operation,
                "ports"
                    | "tree-ports"
                    | "tunnel-ports"
                    | "port-connections"
                    | "tunnel-connections"
                    | "port-centers"
                    | "port-isolation"
                    | "port-containers"
                    | "port-near"
                    | "port-indexed"
                    | "port-imports"
            ) {
                ports
                    .add_nodes_intersections(graph, &nodes, &ports_budget)
                    .map_err(|_| failure())?;
            }
            if matches!(operation, "edge-fill" | "trees") || boundary_stage {
                ports
                    .add_edge_nodes(graph, &nodes, &ports_budget)
                    .map_err(|_| failure())?;
            }
            if matches!(operation, "trees" | "tree-ports") || boundary_stage {
                ports
                    .add_tree_nodes(graph, &nodes, &ports_budget)
                    .map_err(|_| failure())?;
            }
            if boundary_stage {
                let bounds = super::edge_nodes::tight_bounds(graph, &nodes, &ports_budget)
                    .map_err(|_| failure())?;
                ports
                    .add_boundary_layers(graph, &nodes, bounds, &ports_budget)
                    .map_err(|_| failure())?;
                if operation != "boundary-layers" {
                    ports
                        .add_port_boundary_nodes(graph, &nodes, &nodes, bounds, &ports_budget)
                        .map_err(|_| failure())?;
                }
                if matches!(
                    operation,
                    "boundaries"
                        | "tunnels"
                        | "connections"
                        | "centers"
                        | "isolated"
                        | "containers"
                        | "near"
                        | "indexed"
                        | "imports"
                ) {
                    ports
                        .add_corners(graph, &nodes, bounds, &ports_budget)
                        .map_err(|_| failure())?;
                }
            }
            if matches!(
                operation,
                "tunnel-ports"
                    | "tunnels"
                    | "connections"
                    | "tunnel-connections"
                    | "centers"
                    | "isolated"
                    | "containers"
                    | "near"
                    | "indexed"
                    | "imports"
            ) {
                ports
                    .add_tunnels(graph, &nodes, &ports_budget)
                    .map_err(|_| failure())?;
            }
            if matches!(
                operation,
                "connections"
                    | "port-connections"
                    | "tunnel-connections"
                    | "centers"
                    | "isolated"
                    | "containers"
                    | "near"
                    | "indexed"
                    | "imports"
            ) {
                ports
                    .connect_nodes(graph, &nodes, &ports_budget)
                    .map_err(|message| {
                        ports_budget
                            .take_context_error()
                            .map(|e| e.to_string())
                            .unwrap_or(message)
                    })?;
            }
            if matches!(
                operation,
                "port-centers"
                    | "port-near"
                    | "port-indexed"
                    | "port-imports"
                    | "centers"
                    | "isolated"
                    | "containers"
                    | "near"
                    | "indexed"
                    | "imports"
            ) {
                ports
                    .connect_centers(graph, &nodes, &ports_budget)
                    .map_err(|_| failure())?;
            }
            if matches!(
                operation,
                "port-isolation" | "isolated" | "containers" | "near" | "indexed" | "imports"
            ) {
                ports
                    .remove_isolated(&ports_budget)
                    .map_err(|_| failure())?;
            }
            if matches!(
                operation,
                "port-containers"
                    | "port-indexed"
                    | "port-imports"
                    | "containers"
                    | "near"
                    | "indexed"
                    | "imports"
            ) {
                ports
                    .map_containers(graph, &ports_budget)
                    .map_err(|_| failure())?;
            }
            if matches!(
                operation,
                "port-near" | "port-indexed" | "port-imports" | "near" | "indexed" | "imports"
            ) {
                ports
                    .flag_near_ports(&ports_budget)
                    .map_err(|_| failure())?;
            }
            if matches!(
                operation,
                "port-indexed" | "port-imports" | "indexed" | "imports"
            ) {
                ports.assign_indices(&ports_budget).map_err(|_| failure())?;
            }
            if imports {
                let header = graph.edge_order.header();
                let selected = config["selected"]
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .map(|value| {
                                let index = value.as_i64().unwrap();
                                if index < 0 {
                                    None
                                } else {
                                    header.get(index as usize).unwrap_or_else(|| {
                                        panic!(
                                            "runtime error: index out of range [{}] with length {}",
                                            index,
                                            header.len()
                                        )
                                    })
                                }
                            })
                            .collect::<std::collections::BTreeSet<_>>()
                    })
                    .unwrap_or_default();
                for edge in graph.edge_order.header().iter_slots() {
                    ports_budget.reserve_work(1).map_err(|_| failure())?;
                    let edge = edge
                        .expect("runtime error: invalid memory address or nil pointer dereference");
                    if selected.contains(&Some(edge)) {
                        continue;
                    }
                    imported_routes.push(super::ovg_import::ImportedRoute::default());
                    ports
                        .import_edge_route(
                            graph,
                            edge,
                            imported_routes.last_mut().unwrap(),
                            &ports_budget,
                        )
                        .map_err(|_| failure())?;
                }
            }
            return Ok(());
        }
        if matches!(operation, "labels" | "place-labels") {
            let selected = config["selected"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let i = i.as_i64().unwrap();
                    (i >= 0).then_some(i as usize)
                })
                .collect::<Vec<_>>();
            let runtime = route_stage_guard::RuntimeRouteContext;
            let observed = LabelProbeContext {
                caller: if config["context"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
                {
                    &context
                } else {
                    &runtime
                },
                used: &context.label_work,
            };
            let ctx: Option<&dyn crate::internal::limits::Context> =
                (config["context"] != "nil").then_some(&observed);
            return if operation == "place-labels" {
                crate::internal::labels::place_labels_probe(graph, ctx, limit as i64)
            } else {
                crate::internal::labels::place_new_edge_labels_probe(
                    graph,
                    &selected,
                    ctx,
                    limit as i64,
                )
            }
            .map_err(|error| error.to_string());
        }
        if config["operation"] == "standalone" {
            let selected = config["selected"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| {
                    let i = i.as_i64().unwrap();
                    (i >= 0).then_some(crate::EdgeId(i as u32))
                })
                .collect::<Vec<_>>();
            let requested = selected.iter().filter_map(|edge| *edge).collect::<Vec<_>>();
            let runtime = route_stage_guard::RuntimeRouteContext;
            let ctx: Option<&dyn crate::internal::limits::Context> =
                if config["context"].as_str().is_some() {
                    (config["context"] != "nil").then_some(&context)
                } else {
                    Some(&runtime)
                };
            return standalone::run(
                graph,
                Some(crate::internal::OwnerGraphId(0)),
                ctx,
                &selected,
                &budget,
                |g, guard| standalone::body(g, &requested, 1, guard),
            )
            .map_err(|error| error.to_string());
        }
        if config["operation"] == "reorder" {
            return if config["context"].as_str().is_some() {
                crate::internal::labels::reorder_duplicates_with_context(
                    graph,
                    (config["context"] != "nil")
                        .then_some(&context as &dyn crate::internal::limits::Context),
                )
            } else {
                crate::internal::labels::reorder_duplicates_checked(graph)
            }
            .map_err(|error| error.to_string());
        }
        if config["operation"] == "trace" {
            return if config["context"].as_str().is_some() {
                trace::trace_with_context(
                    graph,
                    (config["context"] != "nil")
                        .then_some(&context as &dyn crate::internal::limits::Context),
                    &budget,
                )
            } else {
                trace::trace_edges_to_shape_border_with_budget(graph, &budget)
            }
            .map_err(|error| error.to_string());
        }
        if config["operation"] == "crosshatch" {
            return if config["context"].as_str().is_some() {
                crosshatch::crosshatch_with_context(
                    graph,
                    (config["context"] != "nil")
                        .then_some(&context as &dyn crate::internal::limits::Context),
                    &budget,
                )
            } else {
                crosshatch::crosshatch_with_budget(graph, &budget)
            }
            .map_err(|error| error.to_string());
        }
        if config["operation"] == "swap" {
            return if config["context"].as_str().is_some() {
                swap_owner::swap_with_context(
                    graph,
                    (config["context"] != "nil")
                        .then_some(&context as &dyn crate::internal::limits::Context),
                    &budget,
                )
                .map(|_| ())
            } else {
                swap_ports::swap_edge_ports_with_budget(graph, &budget)
            }
            .map_err(|error| error.to_string());
        }
        if config["operation"] == "branch" {
            return if config["context"].as_str().is_some() {
                cluster_branching::branch_with_context(
                    graph,
                    (config["context"] != "nil")
                        .then_some(&context as &dyn crate::internal::limits::Context),
                    &budget,
                )
            } else {
                cluster_branching::fix_cluster_edge_branching_with_budget(graph, &budget)
            }
            .map_err(|error| error.to_string());
        }
        if config["operation"] == "balance" {
            return if config["context"].as_str().is_some() {
                balance_owner::balance_with_context(
                    graph,
                    (config["context"] != "nil")
                        .then_some(&context as &dyn crate::internal::limits::Context),
                    &budget,
                )
            } else {
                balance::balance_edge_segments_with_budget(graph, &budget)
            }
            .map_err(|error| error.to_string());
        }
        if config["operation"] == "nudge" {
            return if config["context"].as_str().is_some() {
                nudge::nudge_with_context(
                    graph,
                    (config["context"] != "nil")
                        .then_some(&context as &dyn crate::internal::limits::Context),
                    &budget,
                )
            } else {
                nudge::nudge_edge_channels_with_budget(graph, &budget)
            }
            .map_err(|error| error.to_string());
        }
        if config["operation"] == "shortcut" {
            return match mode {
                "candidate" => {
                    let points = &graph.edge_records[edge].route;
                    route = Some(shortcut::candidate(
                        points,
                        config["start"].as_u64().unwrap() as usize,
                        config["end"].as_u64().unwrap() as usize,
                        config["horizontal"].as_bool().unwrap(),
                    ));
                    Ok(())
                }
                "atomic" => {
                    if config["context"].as_str().is_some() {
                        shortcut::shortcut_with_context(
                            graph,
                            (config["context"] != "nil")
                                .then_some(&context as &dyn crate::internal::limits::Context),
                            &budget,
                        )
                        .map_err(|error| error.to_string())
                    } else {
                        shortcut::shortcut_edge_routes_with_budget(graph, &budget)
                            .map_err(|error| error.to_string())
                    }
                }
                _ => panic!("unknown shortcut probe mode"),
            };
        }
        match mode {
            "once" => simplify::simplify_once_with_budget(graph, edge, Some(&budget))
                .map(|r| route = Some(r))
                .map_err(|()| work_error()),
            "loop" => simplify::simplify_edge_routes_with_budget_inner(graph, Some(&budget))
                .map_err(|()| work_error()),
            "atomic" => {
                if config["context"].as_str().is_some() {
                    route_stage_guard::run_atomic_route_stage(
                        graph,
                        Some(crate::internal::OwnerGraphId(0)),
                        (config["context"] != "nil")
                            .then_some(&context as &dyn crate::internal::limits::Context),
                        "SimplifyEdgeRoutes",
                        &[],
                        &budget,
                        |graph, guard| {
                            simplify::simplify_edge_routes_with_budget_inner(graph, Some(guard))
                        },
                    )
                    .map_err(|error| error.to_string())
                } else {
                    simplify::simplify_edge_routes_with_budget(graph, &budget)
                        .map_err(|error| error.to_string())
                }
            }
            _ => panic!("unknown simplify probe mode"),
        }
    }));
    let (error, panic) = match outcome {
        Ok(Ok(())) => (None, None),
        Ok(Err(error)) => (Some(error), None),
        Err(payload) => (
            None,
            Some(
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "non-string panic".into()),
            ),
        ),
    };
    let mut output = json!({"name":name,"before":before,"after":observe(graph,&retained,&kept,&route,include_nodes,config["operation"]=="standalone"),"used":budget.used(),"error":error,"panic":panic});
    if matches!(operation, "labels" | "place-labels") {
        output["after"]["labels"] = label_state(graph);
        output["used"] = json!(context.label_work.get());
    }
    if let Some(state) = &final_state {
        output["used"] = json!(state.used());
        output["final_routes"] = state.observe();
        output["final_before"] = final_before.unwrap();
    }
    if let Some(tree) = &context.tree {
        output["tree_orientation"] = json!(tree.fields().orientation);
    }
    if detached_tunnels {
        output["tunnels"] = tunnel_output;
        if range_initialized {
            output["resources"] = json!(ports_budget.observed_resources());
        }
    }
    if tunnel_ranges {
        output["ranges"] = range_output;
        output["horizontal"] = json!(range_horizontal);
        if range_initialized {
            output["resources"] = json!(ports_budget.observed_resources());
        }
    }
    if construction {
        output["ovg"] = if ports_initialized {
            if imports {
                ports.observe_imports(graph, &imported_routes)
            } else if finalized {
                ports.observe_lifecycle_stage(graph, true)
            } else if lifecycle {
                ports.observe_lifecycle(graph)
            } else if matches!(
                operation,
                "connections" | "port-connections" | "tunnel-connections"
            ) {
                ports.observe_connections(graph)
            } else if matches!(operation, "tunnel-ports" | "tunnels") {
                ports.observe_tunnels(graph)
            } else {
                ports.observe(graph)
            }
        } else {
            Value::Null
        };
        if ports_initialized {
            output["resources"] = json!(ports_budget.observed_resources());
        }
    }
    if config["context"]
        .as_str()
        .is_some_and(|value| !value.is_empty())
    {
        output["polls"] = json!(context.polls.get());
        if crate::internal::trace_env_enabled("D2RS_TRACE_ROUTE_CALLBACK_SITES") {
            let trace = context.callback_trace.borrow();
            output["callback_trace"] = json!({"stacks": trace.0, "ordered": trace.1});
        }
    }
    output
}

fn label_state(graph: &ArenaGraph) -> Value {
    json!({"edges":graph.edge_records.iter().map(|edge|json!({"position":edge.label.as_ref().map(|l|l.position.d2_name()),"percentage":format!("{:016x}",edge.label.as_ref().map_or(0.0,|l|l.percentage).to_bits())})).collect::<Vec<_>>(),
    "nodes":graph.graph_node_order().iter().map(|id|{let node=&graph.nodes[id.0 as usize];json!({"label":node.label_size.map(|_|node.label_position.d2_name()),"icon":node.icon_position.map(|p|p.d2_name())})}).collect::<Vec<_>>()})
}
