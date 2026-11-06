// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Prepared, production-scorer NormalizeGaps controls for source comparison.
//!
//! This module is deliberately absent without `diagnostic-traces`. It prepares
//! the package-local original Go rollback witness and invokes the real guarded
//! stage; it neither installs expected results nor substitutes a test scorer.

#![cfg(feature = "diagnostic-traces")]

#[path = "owner_pass_probe.rs"]
mod owner_pass_probe;

use super::*;
use crate::internal::model::EdgeLengthCacheAlias;
use crate::{Edge, Node};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};

const SENTINEL_KEY: u64 = 0x6e6f_726d_6761_7073;
const SOURCE_LOCATION: &str = "PlacementCostRollbackTransactions";

struct BackgroundContext;

impl limits::Context for BackgroundContext {
    fn done(&self) -> Option<bool> {
        None
    }

    fn error(&self) -> Option<limits::ContextError> {
        None
    }
}

// Source engine_test.go deliberately retains its background observing context
// in the WorkGuard, while pre-canceled runs with a different operation context.
struct PreCanceledContext;

impl limits::Context for PreCanceledContext {
    fn done(&self) -> Option<bool> {
        Some(true)
    }

    fn error(&self) -> Option<limits::ContextError> {
        Some(limits::ContextError::Cancelled)
    }
}

struct EntryIdentity {
    cost_owner: Arc<GraphCostCache>,
    cache_map: EdgeLengthCacheAlias,
    nodes: Vec<(NodeId, i64)>,
    edges: Vec<(EdgeId, i64)>,
}

fn bits(value: f64) -> String {
    format!("0x{:016x}", value.to_bits())
}

fn prepared_pipeline(empty: bool, reverse: bool) -> Pipeline {
    let mut input = Graph::default();
    if !empty {
        let size = Size {
            width: 10.0,
            height: 10.0,
        };
        let a = input.add_node(Node::new("a", size));
        let b = input.add_node(Node::new("b", size));
        input.tala_id_overrides[a.0 as usize] = Some(1);
        input.tala_id_overrides[b.0 as usize] = Some(2);
        let positions = if reverse {
            [1000.0, 0.0]
        } else {
            [0.0, 1000.0]
        };
        input.initial_positions[a.0 as usize] = Some(Point {
            x: positions[0],
            y: 0.0,
        });
        input.initial_positions[b.0 as usize] = Some(Point {
            x: positions[1],
            y: 0.0,
        });
        input.add_edge(if reverse {
            Edge {
                source: b,
                target: a,
            }
        } else {
            Edge {
                source: a,
                target: b,
            }
        });
    }
    let mut pipeline = Pipeline::new_untraced(&input, 0, false, false);
    let graph = &mut pipeline.graph;
    // Source NewGraph/AddNode/Connect bypasses the D2 adapter: no root key,
    // authored directions, node label, arrowhead records or edge label minima.
    graph.containers.clear();
    // ArenaGraph::from_input initializes retained tree-child slices. The
    // source NewGraph/AddNode/Connect fixture never preprocesses trees, so
    // those adapter slices must be absent alongside Graph.Containers.
    graph.preprocessed_tree_children.clear();
    graph.source_container_child_order.clear();
    graph.connected_container_child_order.clear();
    graph.directions.clear();
    graph.scoring_directions.clear();
    graph.directions_by_tala.clear();
    graph.scoring_directions_by_tala.clear();
    for node in &mut graph.nodes {
        node.label_text_present = false;
        debug_assert!(node.loop_offsets.is_none());
        node.content_insets = Insets::uniform(0.0);
    }
    for edge in &mut graph.edge_records {
        edge.tala_id = 0;
        edge.source_arrow = false;
        edge.target_arrow = false;
        edge.source_arrowhead = None;
        edge.target_arrowhead = None;
        edge.min_width = 0.0;
        edge.min_height = 0.0;
    }
    graph.cell_size = 0.0;
    graph.cost_cache.store_edge_length(SENTINEL_KEY, 17.0);
    graph.restore_routing_costs((3.0, 5.0, 7.0));
    pipeline.run_align_axis = true;
    pipeline
}

fn entry_identity(graph: &ArenaGraph) -> EntryIdentity {
    EntryIdentity {
        cost_owner: graph.cost_cache.clone(),
        cache_map: graph
            .cost_cache
            .edge_length_cache_alias()
            .expect("the prepared source graph owns an enabled cache"),
        nodes: graph
            .nodes
            .iter()
            .map(|n| (n.input_id, n.tala_id))
            .collect(),
        edges: graph
            .edge_records
            .iter()
            .map(|e| (e.input_id, e.tala_id))
            .collect(),
    }
}

fn raw_state(graph: &ArenaGraph, identity: &EntryIdentity) -> Value {
    let routing = graph.raw_routing_costs();
    let current_map = graph.cost_cache.edge_length_cache_alias();
    let mut cache = current_map
        .as_ref()
        .map(|alias| alias.entries())
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    cache.sort_by_key(|&(key, _)| key);
    let nodes = graph
        .graph_node_order()
        .into_iter()
        .map(|id| {
            let node = &graph.nodes[id.0 as usize];
            let position = node.position;
            json!({
                "id": node.tala_id,
                "x": position.map(|p| bits(p.x)),
                "y": position.map(|p| bits(p.y)),
                "width": bits(node.rect.size.width),
                "height": bits(node.rect.size.height),
                "arena_node_identity_same": identity.nodes.contains(&(node.input_id, node.tala_id)),
            })
        })
        .collect::<Vec<_>>();
    let edges = graph.edge_records.iter().enumerate().map(|(index, edge)| {
        json!({
            "index": index,
            "id": edge.tala_id,
            "from": graph.nodes[edge.from.0 as usize].tala_id,
            "to": graph.nodes[edge.to.0 as usize].tala_id,
            "arena_edge_identity_same": identity.edges.get(index) == Some(&(edge.input_id, edge.tala_id)),
        })
    }).collect::<Vec<_>>();
    let cost_owner_same = Arc::ptr_eq(&identity.cost_owner, &graph.cost_cache);
    json!({
        "raw_crossing": bits(routing.0),
        "raw_turn": bits(routing.1),
        "raw_noncenter": bits(routing.2),
        "cache_enabled": graph.cost_cache.edge_length_cache_enabled(),
        "cache_entries": cache.len(),
        "cache_map_owner_same": cost_owner_same && current_map.as_ref().is_some_and(|alias| identity.cache_map.same_map(alias)),
        "cost_owner_same": cost_owner_same,
        "cache": cache.into_iter().map(|(key, cost)| json!({"key": format!("0x{key:016x}"), "cost": bits(cost)})).collect::<Vec<_>>(),
        "cell": bits(graph.cell_size),
        "nodes": nodes,
        "edges": edges,
    })
}

fn observe(graph: &ArenaGraph, identity: &EntryIdentity, repeat: bool) -> Result<Value, String> {
    let value = raw_state(graph, identity);
    if repeat {
        for _ in 0..2 {
            if raw_state(graph, identity) != value {
                return Err("repeated raw observation changed state".into());
            }
        }
    }
    Ok(value)
}

struct ContextObservations {
    current: Value,
    moving_displaced: bool,
    site: limits::TransactionPollSite,
    work_used: i64,
    last: Option<Value>,
    events: Vec<Value>,
    accepted_observed: bool,
    cache_growth_observed: bool,
    accepted_with_cache_growth_observed: bool,
    observation_error: Option<String>,
}

/// Source codexRollbackContext.Err, using a safely borrowed actual transaction
/// view instead of retaining a graph pointer or inspecting Rust stack text.
struct ObservingContext<'identity> {
    identity: &'identity EntryIdentity,
    moving: Option<(NodeId, Point)>,
    repeat: bool,
    cancel_after_commit: bool,
    observations: RefCell<ContextObservations>,
}

impl<'identity> ObservingContext<'identity> {
    fn new(
        graph: &ArenaGraph,
        identity: &'identity EntryIdentity,
        entry: Value,
        reverse: bool,
        repeat: bool,
        cancel_after_commit: bool,
    ) -> Self {
        // The source fixture chooses a for the reversed edge and b otherwise.
        // Capture its actual original position; cancellation never tests a
        // target coordinate, expected snapshot, or a work-count threshold.
        let moving_id = NodeId(if reverse { 0 } else { 1 });
        let moving = graph
            .nodes
            .iter()
            .find(|node| node.input_id == moving_id)
            .and_then(|node| node.position.map(|position| (node.input_id, position)));
        Self {
            identity,
            moving,
            repeat,
            cancel_after_commit,
            observations: RefCell::new(ContextObservations {
                current: entry,
                moving_displaced: false,
                site: limits::TransactionPollSite::General,
                work_used: 0,
                last: None,
                events: Vec::new(),
                accepted_observed: false,
                cache_growth_observed: false,
                accepted_with_cache_growth_observed: false,
                observation_error: None,
            }),
        }
    }
}

impl limits::Context for ObservingContext<'_> {
    fn done(&self) -> Option<bool> {
        // A nil Go Done channel retains the original guard's Err polling
        // cadence. Detecting an accepted boundary never closes a channel.
        None
    }

    fn error(&self) -> Option<limits::ContextError> {
        let mut observations = self.observations.borrow_mut();
        let accepted = observations.moving_displaced
            && observations.site == limits::TransactionPollSite::OuterGeometryClone;
        let grown = observations.current["cache_entries"]
            .as_u64()
            .is_some_and(|entries| entries > 1);
        observations.accepted_observed |= accepted;
        observations.cache_growth_observed |= grown;
        observations.accepted_with_cache_growth_observed |= accepted && grown;
        if observations.last.as_ref() != Some(&observations.current) || accepted {
            let state = observations.current.clone();
            let work_used = observations.work_used;
            observations.events.push(json!({
                "op": "source-context-poll",
                "state": state,
                "work_used": work_used,
                "accepted_move_boundary": accepted,
                "cache_grown": grown,
            }));
            observations.last = Some(state);
        }
        (self.cancel_after_commit && accepted).then_some(limits::ContextError::Cancelled)
    }

    fn transaction_state(&self, graph: &ArenaGraph, site: limits::TransactionPollSite) {
        let state = observe(graph, self.identity, self.repeat);
        let moving_displaced = self.moving.is_some_and(|(id, original)| {
            graph
                .nodes
                .iter()
                .find(|node| node.input_id == id)
                .and_then(|node| node.position)
                .is_some_and(|position| position != original)
        });
        let mut observations = self.observations.borrow_mut();
        match state {
            Ok(state) => observations.current = state,
            Err(error) => {
                observations.observation_error.get_or_insert(error);
            }
        }
        observations.moving_displaced = moving_displaced;
        observations.site = site;
        // Recording is deliberately restricted to the genuine error() poll.
    }

    fn transaction_work_used(&self, used: i64) {
        self.observations.borrow_mut().work_used = used;
    }
}

#[derive(Clone, Copy)]
enum ScorerCancelSite {
    Never,
    NodeEntry,
    BeforeStore,
}

struct ScorerObservations {
    current: Value,
    site: &'static str,
    node: Option<i64>,
    iteration: Option<usize>,
    work_used: i64,
    first_scorer_cache: Option<Value>,
    events: Vec<Value>,
    scorer_entries: usize,
    node_entries: usize,
    final_polls: usize,
    cancellations: usize,
    target_states: Vec<Value>,
    observation_error: Option<String>,
}

/// A local operation context for the source's six scorer controls. The retained
/// work guard has its own BackgroundContext. Every genuine operation error poll
/// is recorded, including unchanged states; no graph pointer is retained.
struct ScorerContext<'identity> {
    identity: &'identity EntryIdentity,
    repeat: bool,
    target: ScorerCancelSite,
    observations: RefCell<ScorerObservations>,
}

impl<'identity> ScorerContext<'identity> {
    fn new(
        identity: &'identity EntryIdentity,
        entry: Value,
        repeat: bool,
        target: ScorerCancelSite,
    ) -> Self {
        Self {
            identity,
            repeat,
            target,
            observations: RefCell::new(ScorerObservations {
                current: entry,
                site: "transaction-general",
                node: None,
                iteration: None,
                work_used: 0,
                first_scorer_cache: None,
                events: Vec::new(),
                scorer_entries: 0,
                node_entries: 0,
                final_polls: 0,
                cancellations: 0,
                target_states: Vec::new(),
                observation_error: None,
            }),
        }
    }

    fn capture(
        &self,
        graph: &ArenaGraph,
        site: &'static str,
        node: Option<NodeId>,
        iteration: Option<usize>,
    ) {
        let state = observe(graph, self.identity, self.repeat);
        let node_identity = node.and_then(|id| graph.nodes.get(id.0 as usize).map(|n| n.tala_id));
        let mut observations = self.observations.borrow_mut();
        match state {
            Ok(state) => observations.current = state,
            Err(error) => {
                observations.observation_error.get_or_insert(error);
            }
        }
        if node.is_some() && node_identity.is_none() {
            observations
                .observation_error
                .get_or_insert_with(|| "scoring poll supplied an absent node".into());
        }
        observations.site = site;
        observations.node = node_identity;
        observations.iteration = iteration;
    }

    fn summary(&self, work_used: i64) -> Value {
        let observations = self.observations.borrow();
        json!({
            "context_observations": observations.events,
            "work_used": work_used,
            "scorer_entries": observations.scorer_entries,
            "node_entries": observations.node_entries,
            "final_before_store_polls": observations.final_polls,
            "cancellation_count": observations.cancellations,
            "target_states": observations.target_states,
        })
    }
}

impl limits::Context for ScorerContext<'_> {
    fn done(&self) -> Option<bool> {
        None
    }

    fn error(&self) -> Option<limits::ContextError> {
        let mut observations = self.observations.borrow_mut();
        let is_entry = observations.site == "edge-length-entry";
        let is_node = observations.site == "node-edge-length-entry";
        let is_final = observations.site == "edge-length-before-store";
        if is_entry {
            observations.scorer_entries += 1;
            if observations.first_scorer_cache.is_none() {
                observations.first_scorer_cache = Some(observations.current["cache"].clone());
            }
        }
        observations.node_entries += usize::from(is_node);
        observations.final_polls += usize::from(is_final);
        // These are actual production hook names. Counts, geometry and cache
        // contents are observations and never select the cancellation boundary.
        let cancel_here = match self.target {
            ScorerCancelSite::Never => false,
            ScorerCancelSite::NodeEntry => is_node,
            ScorerCancelSite::BeforeStore => is_final,
        };
        let state = observations.current.clone();
        if cancel_here {
            if observations.first_scorer_cache.as_ref() != Some(&state["cache"]) {
                observations.observation_error.get_or_insert_with(|| {
                    "actual cache changed before the first uncached cancellation target".into()
                });
            }
            observations.cancellations += 1;
            observations.target_states.push(state.clone());
        }
        let site = observations.site;
        let node = observations.node;
        let iteration = observations.iteration;
        let work_used = observations.work_used;
        observations.events.push(json!({
            "op": "source-operation-context-poll",
            "site": site,
            "node": node,
            "iteration": iteration,
            "state": state,
            "work_used": work_used,
            "is_scorer_entry": is_entry,
            "is_node_entry": is_node,
            "is_final_before_store": is_final,
            "returns_canceled": cancel_here,
        }));
        cancel_here.then_some(limits::ContextError::Cancelled)
    }

    fn transaction_state(&self, graph: &ArenaGraph, site: limits::TransactionPollSite) {
        let site = match site {
            limits::TransactionPollSite::General => "transaction-general",
            limits::TransactionPollSite::InnerGeometryClone => "transaction-inner-clone",
            limits::TransactionPollSite::OuterGeometryClone => "transaction-outer-clone",
        };
        self.capture(graph, site, None, None);
    }

    fn transaction_work_used(&self, used: i64) {
        self.observations.borrow_mut().work_used = used;
    }

    fn scoring_state(
        &self,
        graph: &ArenaGraph,
        site: &'static str,
        node: Option<NodeId>,
        iteration: Option<usize>,
    ) {
        self.capture(graph, site, node, iteration);
    }
}

fn scorer_profile(case: &str, observer: bool, requested_work_limit: Option<i64>) -> Value {
    let mut pipeline = prepared_pipeline(false, false);
    let is_stage = case.starts_with("stage-");
    if !is_stage {
        // Same explicit prepared option as the original direct scorer controls.
        pipeline.graph.cell_size = 10.0;
    }
    let identity = entry_identity(&pipeline.graph);
    let target = if case.ends_with("node-entry-cancel") {
        ScorerCancelSite::NodeEntry
    } else if case.ends_with("final-before-store-cancel") {
        ScorerCancelSite::BeforeStore
    } else {
        ScorerCancelSite::Never
    };
    let background = BackgroundContext;
    let work_limit = requested_work_limit.unwrap_or(gap_guard::MAX_TRANSACTION_WORK_UNITS);
    let mut warm_control = Value::Null;
    if case.starts_with("scorer-cached-hit-") {
        let warm_entry = match observe(&pipeline.graph, &identity, observer) {
            Ok(state) => state,
            Err(error) => {
                return json!({"case": case, "supported": false, "observation_error": error});
            }
        };
        let warm_context = ScorerContext::new(
            &identity,
            warm_entry.clone(),
            observer,
            ScorerCancelSite::Never,
        );
        let warm_guard = match limits::WorkGuard::new(
            Some(&background),
            "GapScoreWitnessTransactions",
            work_limit,
        ) {
            Ok(guard) => guard,
            Err(error) => {
                return json!({"case": case, "supported": false, "request_error": error.to_string()});
            }
        };
        let warm_length = match pipeline
            .graph
            .global_sized_edge_length_after_tree_restoration_checked(false, &warm_context)
        {
            Ok(length) => length,
            Err(error) => {
                return json!({"case": case, "supported": false, "warm_error": error.to_string()});
            }
        };
        let warm_after = match observe(&pipeline.graph, &identity, observer) {
            Ok(state) => state,
            Err(error) => {
                return json!({"case": case, "supported": false, "observation_error": error});
            }
        };
        if let Some(error) = &warm_context.observations.borrow().observation_error {
            return json!({"case": case, "supported": false, "observation_error": error});
        }
        warm_control = json!({
            "entry": warm_entry,
            "after": warm_after,
            "length": bits(warm_length),
            "observations": warm_context.summary(warm_guard.used()),
        });
    }
    let entry = match observe(&pipeline.graph, &identity, observer) {
        Ok(state) => state,
        Err(error) => return json!({"case": case, "supported": false, "observation_error": error}),
    };
    let context = ScorerContext::new(&identity, entry.clone(), observer, target);
    let mut guard = match limits::WorkGuard::new(
        Some(&background),
        "GapScoreWitnessTransactions",
        work_limit,
    ) {
        Ok(guard) => guard,
        Err(error) => {
            return json!({"case": case, "supported": false, "request_error": error.to_string()});
        }
    };
    let mut length = 0.0;
    let mut changed = false;
    let error = if is_stage {
        match pipeline.gap_normalization_stage_with_context(&context, &mut guard) {
            Ok(value) => {
                changed = value;
                pipeline.run_align_axis = value;
                String::new()
            }
            Err(error) => {
                pipeline.run_align_axis = false;
                error.to_string()
            }
        }
    } else {
        match pipeline
            .graph
            .global_sized_edge_length_after_tree_restoration_checked(false, &context)
        {
            Ok(value) => {
                length = value;
                String::new()
            }
            Err(error) => error.to_string(),
        }
    };
    let after = match observe(&pipeline.graph, &identity, observer) {
        Ok(state) => state,
        Err(error) => return json!({"case": case, "supported": false, "observation_error": error}),
    };
    if let Some(error) = &context.observations.borrow().observation_error {
        return json!({"case": case, "supported": false, "observation_error": error});
    }
    json!({
        "case": case,
        "observer": observer,
        "supported": true,
        "stage_executed": is_stage,
        "scorer_executed": true,
        "result": {
            "entry": entry,
            "after": after,
            "error": error,
            "align_axes_needed": changed,
            "length": bits(length),
            "observations": context.summary(guard.used()),
            "warm_control": warm_control,
            "guard_operation_contexts_distinct": true,
            "operation_done_nil": true,
        },
        "evidence_limits": {
            "scorer": "actual non-cfg(test) fallible scorer",
            "polls": "every actual operation error call; no state deduplication; Background guard polls are unobserved",
            "sites": "actual native logical scoring sites; transaction-general does not manufacture Go caller locations or stack frames",
            "state": "safe snapshots of actual passed native graph; prepared-source and active-trial scopes must be compared separately",
            "identity": "native cache owner and stable arena identities; Go allocation identities are separate",
            "concurrency": "local RefCell witness context; no parallel worker qualification",
        },
        "full_operation_trace_parity": false,
    })
}

// The direct source witness begins after NewGraph/AddNode/Connect and actual
// AbductEdges, rather than after a full-layout constructor. These literals are
// authored preparation inputs; no oracle observations or expected scores are
// installed. The production projector derives every retained endpoint record.
const SYMMETRY_SENTINEL_KEY: u64 = 0x5359_4d4d_4554_5259;
const SYMMETRY_RECOVERY_SITE: &str = "symmetry-abduction-recovery";

struct SymmetryFixture {
    original: Pipeline,
    temporary: Pipeline,
    center: NodeId,
    adapter_state_before_reset: Value,
    projection_observations: Value,
    original_before_projection: Value,
    restored_endpoint_order: Vec<(NodeId, EdgeId)>,
    prepared_owner: Option<super::hierarchy_scope::PreparedOwnerScope>,
    temporary_source_nodes: BTreeMap<NodeId, NodeId>,
}

fn tree_adapter_state(graph: &ArenaGraph) -> Value {
    json!({
        "retained_children": graph.preprocessed_tree_children.len(),
        "tree_nodes": graph.tree_routing_nodes.len(),
        "sentinels": graph.tree_sentinels.len(),
        "restored_edges": graph.restored_tree_edges.len(),
        "published_edges": graph.published_tree_node_edge_order.len(),
    })
}

fn clear_direct_source_tree_adapter(graph: &mut ArenaGraph) {
    // Source direct NewGraph/AddNode/Connect does not run ExtractTrees. The
    // Rust adapter eagerly initializes these carriers even before layout.
    graph.preprocessed_tree_children.clear();
    graph.tree_routing_nodes.clear();
    graph.tree_sentinels.clear();
    graph.restored_tree_edges.clear();
    graph.published_tree_node_edge_order.clear();
}

fn clear_direct_source_presentation(graph: &mut ArenaGraph) {
    for node in &mut graph.nodes {
        node.label_text_present = false;
        node.content_insets = Insets::uniform(0.0);
    }
    for edge in &mut graph.edge_records {
        edge.source_arrow = false;
        edge.target_arrow = false;
        edge.source_arrowhead = None;
        edge.target_arrowhead = None;
        edge.min_width = 0.0;
        edge.min_height = 0.0;
    }
}

fn clear_direct_source_directions(graph: &mut ArenaGraph) {
    graph.directions.clear();
    graph.scoring_directions.clear();
    graph.directions_by_tala.clear();
    graph.scoring_directions_by_tala.clear();
}

fn prepared_symmetry_fixture(reverse: bool, unrelated: usize) -> Result<SymmetryFixture, String> {
    // Original Rust hierarchy positions are local to their parent. The Go
    // fixture stores absolute A=(100,100), which is A_local20 + C_world80.
    let mut authored = vec![
        (1, Point { x: 80.0, y: 80.0 }, 60.0, None),
        (2, Point { x: 20.0, y: 20.0 }, 10.0, Some(1)),
        (3, Point { x: 0.0, y: 100.0 }, 10.0, None),
        (4, Point { x: 200.0, y: 100.0 }, 10.0, None),
    ];
    if unrelated != 0 {
        authored.push((
            90,
            Point {
                x: 8990.0,
                y: 8990.0,
            },
            80.0,
            None,
        ));
        authored.push((91, Point { x: 10.0, y: 10.0 }, 10.0, Some(90)));
        for index in 0..unrelated {
            authored.push((
                100 + index as i64,
                Point {
                    x: 9200.0 + index as f64 * 20.0,
                    y: 9000.0,
                },
                10.0,
                None,
            ));
        }
    }
    if reverse {
        authored.reverse();
    }
    let ids = authored
        .iter()
        .enumerate()
        .map(|(index, &(tala, _, _, _))| (tala, NodeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut input = Graph::default();
    for &(tala, position, dimension, parent) in &authored {
        let mut node = Node::new(
            format!("symmetry-{tala}"),
            Size {
                width: dimension,
                height: dimension,
            },
        );
        node.parent = parent.map(|parent| ids[&parent]);
        let id = input.add_node(node);
        input.tala_id_overrides[id.0 as usize] = Some(tala);
        input.set_initial_position(id, Some(position));
        input.set_container_flag(id, tala == 1 || tala == 90);
    }
    // This arithmetic defines the fixture's independently authored routes,
    // just as source Connect uses its authored endpoint TopLeft literals.
    let authored_world = |tala: i64| {
        let &(_, local, _, parent) = authored
            .iter()
            .find(|entry| entry.0 == tala)
            .expect("an authored edge endpoint exists");
        match parent {
            Some(parent) => {
                let parent_position = authored
                    .iter()
                    .find(|entry| entry.0 == parent)
                    .expect("an authored parent exists")
                    .1;
                Point {
                    x: parent_position.x + local.x,
                    y: parent_position.y + local.y,
                }
            }
            None => local,
        }
    };
    let mut connections = (0..unrelated)
        .map(|index| (91, 100 + index as i64, 2000 + index as i64))
        .collect::<Vec<_>>();
    if reverse {
        connections.extend([(2, 4, 12), (2, 3, 11)]);
    } else {
        connections.extend([(2, 3, 11), (2, 4, 12)]);
    }
    for &(from, to, tala) in &connections {
        let edge = input.add_edge(Edge {
            source: ids[&from],
            target: ids[&to],
        });
        input.tala_edge_ids[edge.0 as usize] = Some(tala);
        let from_position = authored_world(from);
        let to_position = authored_world(to);
        input.set_edge_route(
            edge,
            vec![
                Point {
                    x: from_position.x + 3.0,
                    y: from_position.y + 5.0,
                },
                Point {
                    x: to_position.x + 7.0,
                    y: to_position.y + 9.0,
                },
            ],
        );
    }
    finish_prepared_symmetry_fixture(input, ids[&1])
}

fn prepared_null_current_symmetry_fixture(
    reverse: bool,
    internal_count: usize,
    mixed: bool,
) -> Result<SymmetryFixture, String> {
    // Authored positions use the real native hierarchy coordinate system.
    // Source's absolute child positions are parent80 + these local inputs.
    let mut authored = vec![(1_i64, Point { x: 80.0, y: 80.0 }, 60.0, None)];
    for index in 0..internal_count.max(1) {
        authored.push((
            2 + index as i64,
            Point {
                x: 20.0 + index as f64 * 0.125,
                y: 20.0 + index as f64 * 0.25,
            },
            10.0,
            Some(1_i64),
        ));
    }
    if mixed {
        authored.extend([
            (1000, Point { x: 0.0, y: 100.0 }, 10.0, None),
            (1001, Point { x: 200.0, y: 100.0 }, 10.0, None),
        ]);
    }
    if reverse {
        authored.reverse();
    }
    let ids = authored
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.0, NodeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut input = Graph::default();
    for &(tala, position, dimension, parent) in &authored {
        let mut node = Node::new(
            format!("symmetry-null-{tala}"),
            Size {
                width: dimension,
                height: dimension,
            },
        );
        node.parent = parent.map(|parent| ids[&parent]);
        let id = input.add_node(node);
        input.tala_id_overrides[id.0 as usize] = Some(tala);
        input.set_initial_position(id, Some(position));
        input.set_container_flag(id, tala == 1);
    }
    let world = |tala| {
        let endpoint = authored
            .iter()
            .find(|entry| entry.0 == tala)
            .expect("an authored endpoint exists");
        if let Some(parent) = endpoint.3 {
            let parent_position = authored
                .iter()
                .find(|entry| entry.0 == parent)
                .expect("the authored parent exists")
                .1;
            Point {
                x: parent_position.x + endpoint.1.x,
                y: parent_position.y + endpoint.1.y,
            }
        } else {
            endpoint.1
        }
    };
    let mut connections = (0..internal_count)
        .map(|index| {
            let child = 2 + index as i64;
            if reverse {
                (child, 1, 11 + index as i64)
            } else {
                (1, child, 11 + index as i64)
            }
        })
        .collect::<Vec<_>>();
    if reverse {
        connections.reverse();
    }
    if mixed {
        connections.extend([(2, 1000, 5000), (2, 1001, 5001)]);
    }
    for (from, to, tala) in connections {
        let edge = input.add_edge(Edge {
            source: ids[&from],
            target: ids[&to],
        });
        input.tala_edge_ids[edge.0 as usize] = Some(tala);
        let from_position = world(from);
        let to_position = world(to);
        input.set_edge_route(
            edge,
            vec![
                Point {
                    x: from_position.x + 3.0,
                    y: from_position.y + 5.0,
                },
                Point {
                    x: to_position.x + 7.0,
                    y: to_position.y + 9.0,
                },
            ],
        );
    }
    finish_prepared_symmetry_fixture(input, ids[&1])
}

fn prepared_unpositioned_symmetry_fixture(
    reverse: bool,
    owner_positioned: bool,
) -> Result<SymmetryFixture, String> {
    let order = if reverse {
        [2_i64, 1_i64]
    } else {
        [1_i64, 2_i64]
    };
    let ids = order
        .iter()
        .enumerate()
        .map(|(index, &tala)| (tala, NodeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut input = Graph::default();
    for tala in order {
        let dimension = if tala == 1 { 60.0 } else { 10.0 };
        let mut node = Node::new(
            format!("symmetry-unpositioned-{tala}"),
            Size {
                width: dimension,
                height: dimension,
            },
        );
        node.parent = (tala == 2).then_some(ids[&1]);
        let id = input.add_node(node);
        input.tala_id_overrides[id.0 as usize] = Some(tala);
        input.set_container_flag(id, tala == 1);
        input.set_initial_position(
            id,
            if tala == 1 && owner_positioned {
                Some(Point { x: 80.0, y: 80.0 })
            } else {
                None
            },
        );
    }
    let (source, target) = if reverse {
        (ids[&2], ids[&1])
    } else {
        (ids[&1], ids[&2])
    };
    let edge = input.add_edge(Edge { source, target });
    input.tala_edge_ids[edge.0 as usize] = Some(11);
    // Connect's default unset route is preserved. There is no arithmetic
    // using absent endpoint coordinates and no diagnostic replacement box.
    finish_prepared_symmetry_fixture_preserving_missing_positions(input, ids[&1], true)
}

fn prepared_positioned_child_nil_owner_symmetry_fixture(
    reverse: bool,
) -> Result<SymmetryFixture, String> {
    let order = if reverse {
        [4_i64, 3, 2, 1]
    } else {
        [1_i64, 2, 3, 4]
    };
    let ids = order
        .iter()
        .enumerate()
        .map(|(index, &tala)| (tala, NodeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut input = Graph::default();
    for tala in order {
        let dimension = if tala == 1 { 60.0 } else { 10.0 };
        let mut node = Node::new(
            format!("symmetry-positioned-child-{tala}"),
            Size {
                width: dimension,
                height: dimension,
            },
        );
        node.parent = (tala == 2).then_some(ids[&1]);
        let id = input.add_node(node);
        input.tala_id_overrides[id.0 as usize] = Some(tala);
        input.set_container_flag(id, tala == 1);
        input.set_initial_position(
            id,
            match tala {
                1 => None,
                2 => Some(Point { x: 20.0, y: 20.0 }),
                3 => Some(Point { x: -80.0, y: 20.0 }),
                4 => Some(Point { x: 120.0, y: 20.0 }),
                _ => unreachable!(),
            },
        );
    }
    let edges = if reverse {
        [(12_i64, 4_i64, 2_i64), (11, 3, 2)]
    } else {
        [(11_i64, 2_i64, 3_i64), (12, 2, 4)]
    };
    for (tala, source, target) in edges {
        let edge = input.add_edge(Edge {
            source: ids[&source],
            target: ids[&target],
        });
        input.tala_edge_ids[edge.0 as usize] = Some(tala);
    }
    // Literal independently authored positions and actual default route Vec;
    // no translation or source transcript is installed. Preserve nil C.
    finish_prepared_symmetry_fixture_preserving_missing_positions(input, ids[&1], true)
}

fn symmetry_projection_observations(
    original: &ArenaGraph,
    temporary: &ArenaGraph,
    placement: &PlacementScope,
) -> Value {
    let projected_edges = temporary
        .edge_order
        .read()
        .iter()
        .map(|edge| temporary.edge_records[edge.0 as usize].tala_id)
        .collect::<Vec<_>>();
    let endpoint = |id: NodeId| {
        let node = &original.nodes[id.0 as usize];
        json!({
            "id": node.tala_id,
            "direct_projected_id": placement.old_to_new.get(&id)
                .map(|projected| temporary.nodes[projected.0 as usize].tala_id),
            "original_container": node.container
                .map(|parent| original.nodes[parent.0 as usize].tala_id),
            "original_incident_edge_order": node.edges.read().iter()
                .map(|edge| original.edge_records[edge.0 as usize].tala_id).collect::<Vec<_>>(),
        })
    };
    let omitted_edges = original.edge_order.read().iter().filter_map(|&id| {
        let edge = &original.edge_records[id.0 as usize];
        (!projected_edges.contains(&edge.tala_id)).then(|| json!({
            "id": edge.tala_id, "from": endpoint(edge.from), "to": endpoint(edge.to),
            "points": edge.route.read().iter().map(|&point| symmetry_point(Some(point))).collect::<Vec<_>>(),
            "route_present": true,
        }))
    }).collect::<Vec<_>>();
    let retained_nodes = original
        .node_order
        .iter()
        .copied()
        .filter(|node| !placement.old_to_new.contains_key(node))
        .map(endpoint)
        .collect::<Vec<_>>();
    let collapsed = temporary
        .sized_collapsed_symmetry_neighbors
        .iter()
        .map(|(&id, neighbors)| {
            json!({ "id": id, "neighbors": neighbors.iter().map(|neighbor| json!({
            "id": neighbor.tala_id,
            "owner": temporary.nodes[neighbor.owner.0 as usize].tala_id,
            "container": neighbor.container_tala_id,
            "position": symmetry_point(temporary.projected_absolute_position(*neighbor)),
            "offset": symmetry_point(Some(neighbor.offset)),
        })).collect::<Vec<_>>() })
        })
        .collect::<Vec<_>>();
    json!({
        "projected_edge_order": projected_edges,
        "source_edges_absent_from_temporary_graph": omitted_edges,
        "original_nodes_absent_from_temporary_graph": retained_nodes,
        "actual_collapsed_symmetry_neighbors": collapsed,
        "actual_record_count": temporary.sized_edge_abductions.len(),
        "native_record_model": "optional local EdgeId/current NodeId endpoints; retained owning source EdgeId and source TALA ID",
        "native_nil_slice_presence_represented": true,
        "abductions_present": temporary.has_sized_edge_abductions(),
    })
}

fn finish_prepared_symmetry_fixture(
    input: Graph,
    center_id: NodeId,
) -> Result<SymmetryFixture, String> {
    finish_prepared_symmetry_fixture_preserving_missing_positions(input, center_id, false)
}

fn finish_prepared_symmetry_fixture_preserving_missing_positions(
    mut input: Graph,
    center_id: NodeId,
    preserve_missing_positions: bool,
) -> Result<SymmetryFixture, String> {
    input.serialized_cell_size = Some(10.0);
    input.serialized_routing_costs = Some((3.0, 5.0, 7.0));
    let mut original = Pipeline::new_untraced(&input, 0, true, true);
    let original_tree_state = tree_adapter_state(&original.graph);
    clear_direct_source_tree_adapter(&mut original.graph);
    clear_direct_source_presentation(&mut original.graph);
    for (id, _) in input.nodes() {
        if let Some(position) = input.initial_positions[id.0 as usize] {
            original.graph.set_position(id, position);
        } else if preserve_missing_positions {
            original.graph.clear_position(id);
        }
    }
    original
        .graph
        .cost_cache
        .store_edge_length(SYMMETRY_SENTINEL_KEY, 17.0);
    let original_before_projection = symmetry_graph_raw(
        &original.graph,
        &symmetry_identity(&original.graph),
        "original",
    );
    let (placement, prepared_owner) = original
        .prepare_placement_scope_after(None, &BTreeSet::new())
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the actual production projector returned no root scope".to_owned())?;
    let center = placement
        .old_to_new
        .get(&center_id)
        .copied()
        .ok_or_else(|| "the actual production projector omitted center C".to_owned())?;
    let mut temporary = Pipeline::new_untraced(&placement.graph, 0, false, true);
    let temporary_tree_state = tree_adapter_state(&temporary.graph);
    temporary.trace_root = placement.root_external_id.clone();
    temporary.hydrate_placement_scope(&placement);
    // Normal hydration intentionally clears movable roots. This prepared
    // direct scorer begins with authored geometry, without fixing or forcing
    // hierarchy semantics; the ordinary position setter restores both fields.
    for (&old, &projected) in &placement.old_to_new {
        if let Some(position) = original.graph.position(old) {
            temporary.graph.set_position(projected, position);
        } else if preserve_missing_positions {
            temporary.graph.clear_position(projected);
        }
    }
    clear_direct_source_tree_adapter(&mut temporary.graph);
    clear_direct_source_presentation(&mut temporary.graph);
    // Scope materialization and the full-layout constructor clear routes;
    // source direct AbductEdges retains Connect's actual input route. Copy
    // those authored values by the actual projected/source stable edge ID.
    for edge in &mut temporary.graph.edge_records {
        if let Some(source) = original
            .graph
            .edge_records
            .iter()
            .find(|source| source.tala_id == edge.tala_id)
        {
            edge.route.replace(source.route.clone());
        } else {
            return Err("a projected edge has no actual prepared source edge identity".into());
        }
    }
    temporary.graph.cell_size = 10.0;
    temporary.graph.restore_routing_costs((11.0, 13.0, 17.0));
    temporary
        .graph
        .cost_cache
        .store_edge_length(SYMMETRY_SENTINEL_KEY, 19.0);
    // The construction interface needs its default root direction; source
    // direct NodeSymmetry has no Directions entries. Clear after projection.
    clear_direct_source_directions(&mut original.graph);
    clear_direct_source_directions(&mut temporary.graph);
    let projection_observations =
        symmetry_projection_observations(&original.graph, &temporary.graph, &placement);
    Ok(SymmetryFixture {
        original,
        temporary,
        center,
        projection_observations,
        original_before_projection,
        restored_endpoint_order: placement.restored_endpoint_order.clone(),
        temporary_source_nodes: placement
            .old_to_new
            .iter()
            .map(|(&source, &local)| (local, source))
            .collect(),
        prepared_owner,
        adapter_state_before_reset: json!({ "original": original_tree_state, "temporary": temporary_tree_state }),
    })
}

struct SymmetryGraphIdentity {
    arena: EntryIdentity,
    positions: BTreeMap<i64, Option<Point>>,
    routes: BTreeMap<i64, Vec<Point>>,
}

fn symmetry_identity(graph: &ArenaGraph) -> SymmetryGraphIdentity {
    SymmetryGraphIdentity {
        arena: entry_identity(graph),
        positions: graph
            .nodes
            .iter()
            .map(|node| (node.tala_id, node.position))
            .collect(),
        routes: graph
            .edge_records
            .iter()
            .map(|edge| (edge.tala_id, edge.route.read().clone()))
            .collect(),
    }
}

fn symmetry_point(position: Option<Point>) -> Value {
    match position {
        Some(position) => json!({ "x": bits(position.x), "y": bits(position.y) }),
        None => Value::Null,
    }
}

fn symmetry_graph_raw(graph: &ArenaGraph, identity: &SymmetryGraphIdentity, owner: &str) -> Value {
    let base = raw_state(graph, &identity.arena);
    let node_order = graph.graph_node_order();
    let nodes = node_order.iter().map(|&id| {
        let node = &graph.nodes[id.0 as usize];
        json!({
            "id": node.tala_id,
            "top_left": symmetry_point(node.position),
            "width": bits(node.rect.size.width),
            "height": bits(node.rect.size.height),
            "container": node.container.map(|parent| graph.nodes[parent.0 as usize].tala_id),
            "is_container": node.is_container,
            "incident_edge_order": node.edges.read().iter().map(|edge| graph.edge_records[edge.0 as usize].tala_id).collect::<Vec<_>>(),
            "incident_edges_present": true,
            "native_arena_owner": owner,
            "arena_node_identity_same": identity.arena.nodes.contains(&(node.input_id, node.tala_id)),
            "position_value_same": identity.positions.get(&node.tala_id) == Some(&node.position),
            "native_rect_origin": symmetry_point(Some(node.rect.origin)),
            "native_scoring_is_container": node.scoring_is_container,
            "native_fixed_top_left": symmetry_point(node.fixed_top_left),
            "native_hierarchy": format!("{:?}", node.hierarchy.clone()),
        })
    }).collect::<Vec<_>>();
    let edges = graph.edge_order.read().iter().enumerate().map(|(index, &id)| {
        let edge = &graph.edge_records[id.0 as usize];
        json!({
            "index": index,
            "id": edge.tala_id,
            "from": graph.nodes[edge.from.0 as usize].tala_id,
            "to": graph.nodes[edge.to.0 as usize].tala_id,
            "points": edge.route.read().iter().map(|&point| symmetry_point(Some(point))).collect::<Vec<_>>(),
            "arena_edge_identity_same": identity.arena.edges.contains(&(edge.input_id, edge.tala_id)),
            "route_value_same": identity.routes.get(&edge.tala_id) == Some(&edge.route.read().clone()),
        })
    }).collect::<Vec<_>>();
    let keys = std::iter::once(None).chain(node_order.iter().copied().map(Some));
    let containers = keys.filter_map(|container| graph.containers.get(&container).map(|children| {
        json!({
            "container": container.map(|node| graph.nodes[node.0 as usize].tala_id),
            "child_order": children.iter().map(|node| graph.nodes[node.0 as usize].tala_id).collect::<Vec<_>>(),
        })
    })).collect::<Vec<_>>();
    json!({
        "cell": base["cell"], "raw_crossing": base["raw_crossing"],
        "raw_turn": base["raw_turn"], "raw_noncenter": base["raw_noncenter"],
        "cache_enabled": base["cache_enabled"], "cache_map_owner_same": base["cache_map_owner_same"],
        "cache": base["cache"], "nodes": nodes, "edges": edges, "containers": containers,
        "native_tree_adapter": tree_adapter_state(graph),
    })
}

fn symmetry_state(
    original: &ArenaGraph,
    temporary: &ArenaGraph,
    original_identity: &SymmetryGraphIdentity,
    temporary_identity: &SymmetryGraphIdentity,
) -> Value {
    let projected = |endpoint: AbductedNodeRef| {
        json!({
            "id": endpoint.tala_id,
            "owner": endpoint.owner.map(|owner| temporary.nodes[owner.0 as usize].tala_id).or(endpoint.owner_tala_id),
            "container": endpoint.container_tala_id,
            "offset": symmetry_point(endpoint.offset),
            "position": symmetry_point(endpoint.projection().and_then(|endpoint| temporary.projected_absolute_position(endpoint))),
            "source_node_lineage": endpoint.source_node.map(|node| node.0),
            "source_owner_lineage": endpoint.source_owner.map(|node| node.0),
            "width": bits(endpoint.size.width), "height": bits(endpoint.size.height),
            "cluster_member": endpoint.cluster_member,
        })
    };
    let abductions = temporary
        .sized_edge_abductions
        .iter()
        .enumerate()
        .map(|(index, abduction)| {
            let local_edge = abduction.edge.and_then(|id| temporary.edge_records.get(id.0 as usize));
            let source_edge = original.edge_records.get(abduction.source_edge.0 as usize);
            let endpoint = |from: bool| {
                if let Some(edge) = local_edge {
                    let node = if from { edge.from } else { edge.to };
                    Some(temporary.nodes[node.0 as usize].tala_id)
                } else {
                    source_edge.map(|edge| {
                        let node = if from { edge.from } else { edge.to };
                        original.nodes[node.0 as usize].tala_id
                    })
                }
            };
            json!({
                "index": index, "edge_id": abduction.edge_tala_id,
                "originally_from": abduction.originally_from.map(|original| original.tala_id),
                "originally_to": abduction.originally_to.map(|original| original.tala_id),
                "originally_from_exists": abduction.originally_from.is_some(),
                "originally_to_exists": abduction.originally_to.is_some(),
                "originally_from_top_left": symmetry_point(abduction.originally_from.and_then(|endpoint| endpoint.source_node).and_then(|node| original.position(node))),
                "originally_to_top_left": symmetry_point(abduction.originally_to.and_then(|endpoint| endpoint.source_node).and_then(|node| original.position(node))),
                "current_from": abduction.current_from.and_then(|node| temporary.abduction_current_tala_id(node)),
                "current_to": abduction.current_to.and_then(|node| temporary.abduction_current_tala_id(node)),
                "current_from_exists": abduction.current_from.is_some(),
                "current_to_exists": abduction.current_to.is_some(),
                "current_from_top_left": symmetry_point(abduction.current_from.and_then(|endpoint| endpoint.local_node().map(|node| temporary.position(node)).unwrap_or(endpoint.position))),
                "current_to_top_left": symmetry_point(abduction.current_to.and_then(|endpoint| endpoint.local_node().map(|node| temporary.position(node)).unwrap_or(endpoint.position))),
                "edge_from": endpoint(true), "edge_to": endpoint(false),
                "native_local_edge": abduction.edge.map(|edge| edge.0),
                "native_source_edge_lineage": abduction.source_edge.0,
            })
        })
        .collect::<Vec<_>>();
    let projected_endpoints = temporary
        .sized_edge_abductions
        .iter()
        .enumerate()
        .map(|(index, abduction)| {
            json!({ "index": index,
                "originally_from": abduction.originally_from.map(&projected),
                "originally_to": abduction.originally_to.map(&projected),
            })
        })
        .collect::<Vec<_>>();
    let shared_children = temporary
        .trace_container_children
        .iter()
        .map(|(&container, children)| {
            json!({ "container": container, "children": children.iter().map(|node| json!({
            "id": node.tala_id, "position": symmetry_point(node.position),
            "width": bits(node.rect.size.width), "height": bits(node.rect.size.height),
        })).collect::<Vec<_>>() })
        })
        .collect::<Vec<_>>();
    json!({
        "original": symmetry_graph_raw(original, original_identity, "original"),
        "temporary": symmetry_graph_raw(temporary, temporary_identity, "temporary"),
        "abductions": abductions,
        "abductions_present": temporary.has_sized_edge_abductions(),
        "native_shared_projection": { "original_endpoints": projected_endpoints, "container_children": shared_children },
    })
}

struct SymmetryObservations {
    current: Value,
    site: &'static str,
    node: Option<i64>,
    iteration: Option<usize>,
    events: Vec<Value>,
    recovery_polls: usize,
    pair_second_polls: usize,
    cancellations: usize,
    observation_error: Option<String>,
}

struct SymmetryContext<'fixture> {
    original: &'fixture ArenaGraph,
    original_identity: &'fixture SymmetryGraphIdentity,
    temporary_identity: &'fixture SymmetryGraphIdentity,
    repeat: bool,
    cancel_site: Option<&'static str>,
    emit_raw_polls: bool,
    observations: RefCell<SymmetryObservations>,
}

impl SymmetryContext<'_> {
    fn capture(
        &self,
        graph: &ArenaGraph,
        site: &'static str,
        node: Option<NodeId>,
        iteration: Option<usize>,
    ) {
        let state = symmetry_state(
            self.original,
            graph,
            self.original_identity,
            self.temporary_identity,
        );
        let mut observations = self.observations.borrow_mut();
        if self.repeat
            && (0..2).any(|_| {
                symmetry_state(
                    self.original,
                    graph,
                    self.original_identity,
                    self.temporary_identity,
                ) != state
            })
        {
            observations
                .observation_error
                .get_or_insert_with(|| "read-only symmetry observation changed graph".into());
        }
        observations.current = state;
        observations.site = site;
        observations.node =
            node.and_then(|node| graph.nodes.get(node.0 as usize).map(|node| node.tala_id));
        observations.iteration = iteration;
    }
}

impl limits::Context for SymmetryContext<'_> {
    fn done(&self) -> Option<bool> {
        None
    }

    fn error(&self) -> Option<limits::ContextError> {
        let mut observations = self.observations.borrow_mut();
        let recovery = observations.site == SYMMETRY_RECOVERY_SITE;
        let pair_second = observations.site == "symmetry-pair-second";
        let cancel = self.cancel_site == Some(observations.site);
        observations.recovery_polls += usize::from(recovery);
        observations.pair_second_polls += usize::from(pair_second);
        observations.cancellations += usize::from(cancel);
        let event = json!({
            "op": "source-operation-context-poll", "site": observations.site,
            "node": observations.node, "iteration": observations.iteration,
            "state": observations.current,
            "is_abduction_recovery": recovery, "is_pair_second": pair_second,
            "is_container_scan": observations.site == "symmetry-container-abduction",
            "returns_canceled": cancel,
        });
        if self.emit_raw_polls {
            // A genuine scorer panic can terminate before the final JSON.
            // Retain its actual operation prefix without catching it or
            // replacing missing geometry with a diagnostic coordinate.
            eprintln!("SYMMETRY_UNPOSITIONED_NATIVE_POLL {event}");
        }
        observations.events.push(event);
        cancel.then_some(limits::ContextError::Cancelled)
    }

    fn scoring_state(
        &self,
        graph: &ArenaGraph,
        site: &'static str,
        node: Option<NodeId>,
        iteration: Option<usize>,
    ) {
        self.capture(graph, site, node, iteration);
    }

    fn transaction_state(&self, graph: &ArenaGraph, _: limits::TransactionPollSite) {
        self.capture(graph, "transaction-general", None, None);
    }
}

fn diagnostic_symmetry_abduction_probe(request: &Value) -> Value {
    let case = match request.get("case") {
        Some(Value::String(case)) => case.as_str(),
        None => "ordinary-success",
        _ => return json!({"supported": false, "request_error": "case must be a string"}),
    };
    let (reverse, unrelated, cancel_recovery) = match case {
        "ordinary-success" => (false, 0, false),
        "ordinary-source289-cancel" => (false, 0, true),
        "reversed-success" => (true, 0, false),
        "unrelated-success" => (false, 1, false),
        "sixty-five-abductions-success" => (false, 63, false),
        "sixty-five-abductions-source289-cancel" => (false, 63, true),
        _ => {
            return json!({ "case": case, "supported": false, "request_error": "unknown prepared symmetry profile" });
        }
    };
    let observer = match request.get("observer") {
        Some(Value::Bool(value)) => *value,
        None => false,
        _ => {
            return json!({ "case": case, "supported": false, "request_error": "observer must be a boolean" });
        }
    };
    let fixture = match prepared_symmetry_fixture(reverse, unrelated) {
        Ok(fixture) => fixture,
        Err(error) => {
            return json!({ "case": case, "supported": false, "preparation_error": error });
        }
    };
    let original_identity = symmetry_identity(&fixture.original.graph);
    let temporary_identity = symmetry_identity(&fixture.temporary.graph);
    let entry = symmetry_state(
        &fixture.original.graph,
        &fixture.temporary.graph,
        &original_identity,
        &temporary_identity,
    );
    if observer
        && (0..2).any(|_| {
            symmetry_state(
                &fixture.original.graph,
                &fixture.temporary.graph,
                &original_identity,
                &temporary_identity,
            ) != entry
        })
    {
        return json!({ "case": case, "supported": false, "observation_error": "read-only entry observer changed graph" });
    }
    let context = SymmetryContext {
        original: &fixture.original.graph,
        original_identity: &original_identity,
        temporary_identity: &temporary_identity,
        repeat: observer,
        cancel_site: cancel_recovery.then_some(SYMMETRY_RECOVERY_SITE),
        emit_raw_polls: false,
        observations: RefCell::new(SymmetryObservations {
            current: entry.clone(),
            site: "unobserved-entry",
            node: None,
            iteration: None,
            events: Vec::new(),
            recovery_polls: 0,
            pair_second_polls: 0,
            cancellations: 0,
            observation_error: None,
        }),
    };
    let (score, error) = match fixture.temporary.graph.sized_symmetry_checked(
        fixture.center,
        true,
        crate::internal::scoring_context::ScoreContext::new(&context),
    ) {
        Ok(score) => (score, String::new()),
        Err(error) => (0.0, error.to_string()),
    };
    let after = symmetry_state(
        &fixture.original.graph,
        &fixture.temporary.graph,
        &original_identity,
        &temporary_identity,
    );
    if observer
        && (0..2).any(|_| {
            symmetry_state(
                &fixture.original.graph,
                &fixture.temporary.graph,
                &original_identity,
                &temporary_identity,
            ) != after
        })
    {
        return json!({ "case": case, "supported": false, "observation_error": "read-only final observer changed graph" });
    }
    let observations = context.observations.borrow();
    if let Some(error) = &observations.observation_error {
        return json!({ "case": case, "supported": false, "observation_error": error });
    }
    json!({
        "probe": "symmetry-abduction", "case": case, "observer": observer,
        "supported": true, "scorer_executed": true,
        "result": {
            "entry": entry, "after": after, "score": bits(score), "error": error,
            "node_id": fixture.temporary.graph.nodes[fixture.center.0 as usize].tala_id,
            "retained_child_id": fixture.temporary.graph.sized_edge_abductions.iter()
                .filter_map(|abduction| abduction.originally_from)
                .find(|original| original.owner == Some(fixture.center)).map(|original| original.tala_id),
            "abduction_count": fixture.temporary.graph.sized_edge_abductions.len(),
            "abductions_present": fixture.temporary.graph.has_sized_edge_abductions(),
            "operation_done_nil": true,
            "context_observations": observations.events,
            "recovery_polls": observations.recovery_polls,
            "pair_second_polls": observations.pair_second_polls,
            "cancellation_count": observations.cancellations,
        },
        "construction_adapter_observations": fixture.adapter_state_before_reset,
        "evidence_limits": {
            "constructor": "actual production placement_scope_graph and shared hydration; independently authored positioned direct scorer boundary",
            "cancellation_site": "genuine symmetry-abduction-recovery only; absent production site does not cancel; source289 profile labels are historical, actual Go poll is290",
            "polls": "every actual operation context error call; no state deduplication or synthetic caller stacks",
            "graphs": "actual owning and independently materialized temporary native arenas; owner endpoints remain actual native owner endpoints, not fabricated shared Go mutations",
            "positions": "owning nested positions remain raw local values; production projected absolute positions are exposed separately",
            "identity": "stable arena/value/cache identities only; Go graph/point/edge/route pointer alias flags are not fabricated",
            "shared_projection": "actual production-derived retained endpoints and copied container-child carriers; not a claim of Go allocation aliases",
            "scope": "direct C scorer reaches actual projected retained A helper; no optimization or symmetry algorithm correction",
        },
        "full_operation_trace_parity": false,
    })
}

fn diagnostic_symmetry_null_current_probe(request: &Value) -> Value {
    let case = match request.get("case") {
        Some(Value::String(case)) => case.as_str(),
        None => "container-child-success",
        _ => return json!({"supported": false, "request_error": "case must be a string"}),
    };
    let (reverse, internal_count, mixed, cancel_site) = match case {
        "container-child-success" => (false, 1, false, None),
        "container-child-container-scan-cancel" => {
            (false, 1, false, Some("symmetry-container-abduction"))
        }
        "container-child-recovery-cancel" => (false, 1, false, Some(SYMMETRY_RECOVERY_SITE)),
        "child-container-reversed-success" => (true, 1, false, None),
        "child-container-reversed-container-scan-cancel" => {
            (true, 1, false, Some("symmetry-container-abduction"))
        }
        "child-container-reversed-recovery-cancel" => {
            (true, 1, false, Some(SYMMETRY_RECOVERY_SITE))
        }
        "sixty-five-container-child-success" => (false, 65, false, None),
        "sixty-five-container-child-container-scan-cancel" => {
            (false, 65, false, Some("symmetry-container-abduction"))
        }
        "sixty-five-container-child-recovery-cancel" => {
            (false, 65, false, Some(SYMMETRY_RECOVERY_SITE))
        }
        "mixed-internal-external-success" => (false, 1, true, None),
        "actual-present-empty-success" => (false, 0, false, None),
        _ => {
            return json!({"case": case, "supported": false, "request_error": "unknown prepared null-current profile"});
        }
    };
    let observer = match request.get("observer") {
        Some(Value::Bool(value)) => *value,
        None => false,
        _ => {
            return json!({"case": case, "supported": false, "request_error": "observer must be a boolean"});
        }
    };
    let fixture = match prepared_null_current_symmetry_fixture(reverse, internal_count, mixed) {
        Ok(fixture) => fixture,
        Err(error) => return json!({"case": case, "supported": false, "preparation_error": error}),
    };
    let original_identity = symmetry_identity(&fixture.original.graph);
    let temporary_identity = symmetry_identity(&fixture.temporary.graph);
    let entry = symmetry_state(
        &fixture.original.graph,
        &fixture.temporary.graph,
        &original_identity,
        &temporary_identity,
    );
    if observer
        && (0..2).any(|_| {
            symmetry_state(
                &fixture.original.graph,
                &fixture.temporary.graph,
                &original_identity,
                &temporary_identity,
            ) != entry
        })
    {
        return json!({"case": case, "supported": false, "observation_error": "read-only entry observer changed graph"});
    }
    let context = SymmetryContext {
        original: &fixture.original.graph,
        original_identity: &original_identity,
        temporary_identity: &temporary_identity,
        repeat: observer,
        cancel_site,
        emit_raw_polls: false,
        observations: RefCell::new(SymmetryObservations {
            current: entry.clone(),
            site: "unobserved-entry",
            node: None,
            iteration: None,
            events: Vec::new(),
            recovery_polls: 0,
            pair_second_polls: 0,
            cancellations: 0,
            observation_error: None,
        }),
    };
    let (score, error) = match fixture.temporary.graph.sized_symmetry_checked(
        fixture.center,
        true,
        crate::internal::scoring_context::ScoreContext::new(&context),
    ) {
        Ok(score) => (score, String::new()),
        Err(error) => (0.0, error.to_string()),
    };
    let after = symmetry_state(
        &fixture.original.graph,
        &fixture.temporary.graph,
        &original_identity,
        &temporary_identity,
    );
    if observer
        && (0..2).any(|_| {
            symmetry_state(
                &fixture.original.graph,
                &fixture.temporary.graph,
                &original_identity,
                &temporary_identity,
            ) != after
        })
    {
        return json!({"case": case, "supported": false, "observation_error": "read-only final observer changed graph"});
    }
    let observations = context.observations.borrow();
    if let Some(error) = &observations.observation_error {
        return json!({"case": case, "supported": false, "observation_error": error});
    }
    // Annotate the actual site of each genuine poll. This introduces no
    // additional polls and retains every equal-state observation.
    let events = observations
        .events
        .iter()
        .cloned()
        .map(|mut event| {
            let container_scan = event["site"] == "symmetry-container-abduction";
            event
                .as_object_mut()
                .expect("an event is an object")
                .insert("is_container_scan".into(), Value::Bool(container_scan));
            event
        })
        .collect::<Vec<_>>();
    let container_scan_polls = events
        .iter()
        .filter(|event| event["is_container_scan"] == true)
        .count();
    json!({
        "probe": "symmetry-null-current", "case": case, "observer": observer,
        "supported": true, "scorer_executed": true,
        "result": {
            "entry": entry, "after": after, "score": bits(score), "error": error,
            "node_id": fixture.temporary.graph.nodes[fixture.center.0 as usize].tala_id,
            "retained_child_id": fixture.original.graph.nodes.iter()
                .find(|node| node.tala_id == 2).map(|node| node.tala_id),
            "abduction_count": fixture.temporary.graph.sized_edge_abductions.len(),
            "abductions_present": fixture.temporary.graph.has_sized_edge_abductions(),
            "operation_done_nil": true, "context_observations": events,
            "source_argument_mode": "actual",
            "cancel_source_line": match cancel_site {
                Some("symmetry-container-abduction") => 136,
                Some(SYMMETRY_RECOVERY_SITE) => 290,
                _ => 0,
            },
            "container_scan_polls": container_scan_polls,
            "recovery_polls": observations.recovery_polls,
            "pair_second_polls": observations.pair_second_polls,
            "cancellation_count": observations.cancellations,
        },
        "construction_adapter_observations": fixture.adapter_state_before_reset,
        "production_projection_observations": fixture.projection_observations,
        "evidence_limits": {
            "constructor": "actual production placement_scope_graph and unchanged shared semantic hydration; movable authored geometry restored through set_position",
            "topology": "literal C-container/A-child edge11; declaration and edge direction reverse together; no expected record or incident mutation installed",
            "cancellation": "only genuine named symmetry-container-abduction (Go136) or symmetry-abduction-recovery (Go290) operation polls can cancel",
            "polls": "every actual operation-context error call and exact raw native state, including duplicates; no synthetic source frames or counters",
            "record_model": "actual optional local edge/current endpoints and retained owning source EdgeId; no fabricated local edge or endpoint",
            "abductions_present": "actual production hydration records a present slice even when empty; bare native constructor retains nil presence",
            "ownership": "owning local geometry, independently materialized temporary arena, and copied retained carriers remain actual native values and identities",
            "preparation": "production projection inventories are observed separately; no shared-Go-pointer preparation snapshot is fabricated",
            "extensions": "65 real detached records, mixed real edges, and actual present-empty projection are supported; un-abducted caller nil/present-empty source arguments remain separate unsupported controls",
        },
        "full_operation_trace_parity": false,
    })
}

fn unpositioned_original_identity_observations(fixture: &SymmetryFixture) -> Value {
    let original = &fixture.original.graph;
    let temporary = &fixture.temporary.graph;
    let originals = temporary.sized_edge_abductions.iter().map(|record| {
        let source_edge = original.edge_records.get(record.source_edge.0 as usize);
        let node = |id: Option<NodeId>| id.map(|id| {
            let node = &original.nodes[id.0 as usize];
            json!({ "id": node.tala_id, "position": symmetry_point(node.position),
                "width": bits(node.rect.size.width), "height": bits(node.rect.size.height) })
        });
        json!({ "source_edge_id": record.edge_tala_id,
            "source_from": node(source_edge.map(|edge| edge.from)),
            "source_to": node(source_edge.map(|edge| edge.to)),
            "originally_from_exists": record.originally_from.is_some(),
            "originally_to_exists": record.originally_to.is_some(),
            "originally_from_id": record.originally_from.map(|endpoint| endpoint.tala_id),
            "originally_to_id": record.originally_to.map(|endpoint| endpoint.tala_id),
            "originally_from_source_node_lineage": record.originally_from.and_then(|endpoint| endpoint.source_node).map(|node| node.0),
            "originally_to_source_node_lineage": record.originally_to.and_then(|endpoint| endpoint.source_node).map(|node| node.0),
            "originally_from_offset": symmetry_point(record.originally_from.and_then(|endpoint| endpoint.offset)),
            "originally_to_offset": symmetry_point(record.originally_to.and_then(|endpoint| endpoint.offset)),
            "current_from_id": record.current_from.and_then(|endpoint| temporary.abduction_current_tala_id(endpoint)),
            "current_to_id": record.current_to.and_then(|endpoint| temporary.abduction_current_tala_id(endpoint)),
        })
    }).collect::<Vec<_>>();
    json!({ "records": originals,
        "computed_restored_endpoint_order": fixture.restored_endpoint_order.iter().map(|&(node, edge)| json!({
            "node_id": original.nodes[node.0 as usize].tala_id,
            "edge_id": original.edge_records[edge.0 as usize].tala_id,
            "position": symmetry_point(original.position(node)),
        })).collect::<Vec<_>>() })
}

fn diagnostic_symmetry_unpositioned_probe(request: &Value) -> Value {
    let case = match request.get("case") {
        Some(Value::String(case)) => case.as_str(),
        None => "owner-positioned-child-unpositioned-success",
        _ => return json!({"supported": false, "request_error": "case must be a string"}),
    };
    let (reverse, owner_positioned, cancel_site) = match case {
        "owner-positioned-child-unpositioned-success" => (false, true, None),
        "owner-positioned-child-unpositioned-container-scan-cancel" => {
            (false, true, Some("symmetry-container-abduction"))
        }
        "owner-positioned-child-unpositioned-recovery-cancel" => {
            (false, true, Some(SYMMETRY_RECOVERY_SITE))
        }
        "child-unpositioned-owner-positioned-reversed-success" => (true, true, None),
        "child-unpositioned-owner-positioned-reversed-container-scan-cancel" => {
            (true, true, Some("symmetry-container-abduction"))
        }
        "child-unpositioned-owner-positioned-reversed-recovery-cancel" => {
            (true, true, Some(SYMMETRY_RECOVERY_SITE))
        }
        "both-unpositioned-success" => (false, false, None),
        "both-unpositioned-container-scan-cancel" => {
            (false, false, Some("symmetry-container-abduction"))
        }
        "both-unpositioned-recovery-cancel" => (false, false, Some(SYMMETRY_RECOVERY_SITE)),
        "both-unpositioned-reversed-success" => (true, false, None),
        "both-unpositioned-reversed-container-scan-cancel" => {
            (true, false, Some("symmetry-container-abduction"))
        }
        "both-unpositioned-reversed-recovery-cancel" => (true, false, Some(SYMMETRY_RECOVERY_SITE)),
        _ => {
            return json!({"case": case, "supported": false, "request_error": "unknown unpositioned-original profile"});
        }
    };
    let observer = match request.get("observer") {
        Some(Value::Bool(value)) => *value,
        None => false,
        _ => {
            return json!({"case": case, "supported": false, "request_error": "observer must be a boolean"});
        }
    };
    let fixture = match prepared_unpositioned_symmetry_fixture(reverse, owner_positioned) {
        Ok(fixture) => fixture,
        Err(error) => return json!({"case": case, "supported": false, "preparation_error": error}),
    };
    run_optional_geometry_symmetry_probe(
        "symmetry-unpositioned-original",
        case,
        observer,
        fixture,
        cancel_site,
        owner_positioned,
        false,
    )
}

fn diagnostic_symmetry_positioned_child_nil_owner_probe(request: &Value) -> Value {
    let case = match request.get("case") {
        Some(Value::String(case)) => case.as_str(),
        None => "positioned-child-nil-owner-paired-success",
        _ => return json!({"supported": false, "request_error": "case must be a string"}),
    };
    let (reverse, cancel_site) = match case {
        "positioned-child-nil-owner-paired-success" => (false, None),
        "positioned-child-nil-owner-paired-container-scan-cancel" => {
            (false, Some("symmetry-container-abduction"))
        }
        "positioned-child-nil-owner-paired-distance-filter-cancel" => {
            (false, Some("symmetry-distance-filter"))
        }
        "positioned-child-nil-owner-paired-pair-second-cancel" => {
            (false, Some("symmetry-pair-second"))
        }
        "positioned-child-nil-owner-paired-reversed-success" => (true, None),
        "positioned-child-nil-owner-paired-reversed-container-scan-cancel" => {
            (true, Some("symmetry-container-abduction"))
        }
        "positioned-child-nil-owner-paired-reversed-distance-filter-cancel" => {
            (true, Some("symmetry-distance-filter"))
        }
        "positioned-child-nil-owner-paired-reversed-pair-second-cancel" => {
            (true, Some("symmetry-pair-second"))
        }
        _ => {
            return json!({"case": case, "supported": false, "request_error": "unknown positioned-child/nil-owner profile"});
        }
    };
    let observer = match request.get("observer") {
        Some(Value::Bool(value)) => *value,
        None => false,
        _ => {
            return json!({"case": case, "supported": false, "request_error": "observer must be a boolean"});
        }
    };
    let fixture = match prepared_positioned_child_nil_owner_symmetry_fixture(reverse) {
        Ok(fixture) => fixture,
        Err(error) => return json!({"case": case, "supported": false, "preparation_error": error}),
    };
    run_optional_geometry_symmetry_probe(
        "symmetry-positioned-child-nil-owner",
        case,
        observer,
        fixture,
        cancel_site,
        false,
        true,
    )
}

fn run_optional_geometry_symmetry_probe(
    probe: &str,
    case: &str,
    observer: bool,
    mut fixture: SymmetryFixture,
    cancel_site: Option<&'static str>,
    owner_positioned: bool,
    child_positioned: bool,
) -> Value {
    let original_identity = symmetry_identity(&fixture.original.graph);
    let temporary_identity = symmetry_identity(&fixture.temporary.graph);
    let entry = symmetry_state(
        &fixture.original.graph,
        &fixture.temporary.graph,
        &original_identity,
        &temporary_identity,
    );
    let identities = unpositioned_original_identity_observations(&fixture);
    // This raw diagnostic boundary survives a genuine scorer panic. Geometry
    // is nullable; no invented default coordinate makes the scorer proceed.
    eprintln!(
        "SYMMETRY_UNPOSITIONED_NATIVE_PRE_SCORE {}",
        json!({
            "case": case, "observer": observer, "state": entry, "identities": identities,
        })
    );
    let context = SymmetryContext {
        original: &fixture.original.graph,
        original_identity: &original_identity,
        temporary_identity: &temporary_identity,
        repeat: observer,
        cancel_site,
        emit_raw_polls: true,
        observations: RefCell::new(SymmetryObservations {
            current: entry.clone(),
            site: "unobserved-entry",
            node: None,
            iteration: None,
            events: Vec::new(),
            recovery_polls: 0,
            pair_second_polls: 0,
            cancellations: 0,
            observation_error: None,
        }),
    };
    let (score, error) = match fixture.temporary.graph.sized_symmetry_checked(
        fixture.center,
        true,
        crate::internal::scoring_context::ScoreContext::new(&context),
    ) {
        Ok(score) => (score, String::new()),
        Err(error) => (0.0, error.to_string()),
    };
    let after = symmetry_state(
        &fixture.original.graph,
        &fixture.temporary.graph,
        &original_identity,
        &temporary_identity,
    );
    let (events, recovery_polls, pair_second_polls, cancellation_count) = {
        let observations = context.observations.borrow();
        if let Some(error) = &observations.observation_error {
            return json!({"case": case, "supported": false, "observation_error": error});
        }
        (
            observations.events.clone(),
            observations.recovery_polls,
            observations.pair_second_polls,
            observations.cancellations,
        )
    };
    drop(context);
    // This is the real native owner-publication restore path used after a
    // scope returns. Its computed inventory may already have lost a nil-
    // geometry original. Do not fill that missing restoration from fixtures.
    fixture
        .original
        .graph
        .restored_hierarchy_endpoint_order
        .extend(fixture.restored_endpoint_order.iter().copied());
    fixture.original.graph.sync_edge_adjacency_order();
    let restored_state = symmetry_state(
        &fixture.original.graph,
        &fixture.temporary.graph,
        &original_identity,
        &temporary_identity,
    );
    let restored_original =
        symmetry_graph_raw(&fixture.original.graph, &original_identity, "original");
    eprintln!(
        "SYMMETRY_UNPOSITIONED_NATIVE_RESTORED {}",
        json!({
            "case": case, "state": restored_state,
            "identities": unpositioned_original_identity_observations(&fixture),
        })
    );
    let container_scan_polls = events
        .iter()
        .filter(|event| event["site"] == "symmetry-container-abduction")
        .count();
    json!({ "probe": probe, "case": case, "observer": observer,
        "supported": true, "scorer_executed": true,
        "result": { "entry": entry, "after": after, "score": bits(score), "error": error,
            "node_id": fixture.temporary.graph.nodes[fixture.center.0 as usize].tala_id,
            "retained_child_id": fixture.original.graph.nodes.iter().find(|node| node.tala_id == 2).map(|node| node.tala_id),
            "owner_positioned": owner_positioned, "child_positioned": child_positioned,
            "abduction_count": fixture.temporary.graph.sized_edge_abductions.len(),
            "abductions_present": fixture.temporary.graph.has_sized_edge_abductions(),
            "operation_done_nil": true, "context_observations": events,
            "container_scan_polls": container_scan_polls, "recovery_polls": recovery_polls,
            "pair_second_polls": pair_second_polls, "cancellation_count": cancellation_count,
            "restoration": { "state": restored_state,
                "native_retained_metadata_record_count": fixture.temporary.graph.sized_edge_abductions.len(),
                "native_original_matches_pre_projection": restored_original == fixture.original_before_projection,
                "native_computed_endpoint_inventory": identities["computed_restored_endpoint_order"],
            },
        },
        "native_original_before_projection": fixture.original_before_projection,
        "native_original_identity_observations": identities,
        "production_projection_observations": fixture.projection_observations,
        "evidence_limits": {
            "geometry": "independently authored optional owner/child positions preserved through set_position/clear_position before production projection and after unchanged hydration; native rect carriers exposed separately",
            "identity": "actual owning source edge/node lineage independent of optional projected geometry; no original pointer or coordinate is fabricated",
            "restoration": "actual computed restored_endpoint_order appended and actual sync_edge_adjacency_order executed once; no copied Go restores or manufactured remaining-slice/pointer assertions",
            "source_restore": "source standalone RestoreEdgeAbductions remaining slice and shared temporary pointer incidence have no equivalent exposed native return and remain unrepresented",
            "panic": "genuine scoring panic remains a nonzero process with raw pre-score/poll stderr prefix and no final JSON; no catch or default geometry",
            "routes": "actual default present-empty native route vector; no route setter or arithmetic with absent TopLeft",
            "polls": "all genuine named operation polls retained, including duplicate raw states; diagnostics only on stderr",
        }, "full_operation_trace_parity": false,
    })
}

fn foreign_slice_state(original: &ArenaGraph, component: &ArenaGraph) -> Value {
    let records = component.sized_edge_abductions.iter().enumerate().map(|(index, record)| {
        let original_ref = |endpoint: AbductedNodeRef| json!({
            "id": endpoint.tala_id,
            "source_node": endpoint.source_node.map(|node| node.0),
            "source_owner": endpoint.source_owner.map(|node| node.0),
            "owner": endpoint.owner.map(|owner| component.nodes[owner.0 as usize].tala_id).or(endpoint.owner_tala_id),
            "local_owner": endpoint.owner.map(|owner| owner.0),
            "position": symmetry_point(endpoint.source_node.and_then(|node| original.position(node))),
            "container": endpoint.container_tala_id,
            "offset": symmetry_point(endpoint.offset),
            "width": bits(endpoint.size.width), "height": bits(endpoint.size.height),
        });
        json!({
            "index": index, "edge_id": record.edge_tala_id,
            "source_edge": record.source_edge.0,
            "local_edge": record.edge.map(|edge| edge.0),
            "current_from": record.current_from.and_then(|endpoint| component.abduction_current_tala_id(endpoint)),
            "current_from_ref": record.current_from.map(|endpoint| json!({
                "location": format!("{:?}", endpoint.location),
                "source_node": endpoint.source_node.map(|node| node.0),
                "position": symmetry_point(endpoint.local_node().map(|node| component.position(node)).unwrap_or(endpoint.position)),
            })),
            "current_to": record.current_to.and_then(|endpoint| component.abduction_current_tala_id(endpoint)),
            "current_to_ref": record.current_to.map(|endpoint| json!({
                "location": format!("{:?}", endpoint.location),
                "source_node": endpoint.source_node.map(|node| node.0),
                "position": symmetry_point(endpoint.local_node().map(|node| component.position(node)).unwrap_or(endpoint.position)),
            })),
            "source_edge_endpoints": original.edge_records.get(record.source_edge.0 as usize).map(|edge| json!({
                "id": edge.tala_id, "from": original.nodes[edge.from.0 as usize].tala_id, "to": original.nodes[edge.to.0 as usize].tala_id,
            })),
            "originally_from": record.originally_from.map(original_ref),
            "originally_to": record.originally_to.map(original_ref),
        })
    }).collect::<Vec<_>>();
    json!({
        "original": symmetry_graph_raw(original, &symmetry_identity(original), "original"),
        "component": symmetry_graph_raw(component, &symmetry_identity(component), "component"),
        "abductions": records, "abductions_present": component.has_sized_edge_abductions(),
        "native_canonical_ownership": {
            "arena": original.topology_arena_id().map(|arena| arena.0),
            "nodes": original.nodes.iter().enumerate().map(|(index, node)| json!({
                "node": node.tala_id,
                "source_node": index,
                "owner": original.topology_node_owner(NodeId(index as u32)).map(|owner| owner.0),
            })).collect::<Vec<_>>(),
        },
    })
}

struct ForeignSliceContext<'fixture> {
    original: &'fixture ArenaGraph,
    owner_names: &'fixture BTreeMap<OwnerGraphId, String>,
    canonical_members: &'fixture [NodeId],
    observer: bool,
    cancel_site: Option<&'static str>,
    cancel_iteration: usize,
    obstruction_probe: bool,
    cancel_set: usize,
    obstruction_set: Cell<Option<usize>>,
    actual_obstruction_sets_present: Cell<bool>,
    actual_obstruction_sets: RefCell<Value>,
    current: RefCell<Value>,
    events: RefCell<Vec<Value>>,
    observation_error: RefCell<Option<String>>,
}

impl limits::Context for ForeignSliceContext<'_> {
    fn done(&self) -> Option<bool> {
        None
    }

    fn error(&self) -> Option<limits::ContextError> {
        let mut event = self.current.borrow().clone();
        let cancel = self.cancel_site.is_some_and(|site| event["site"] == site)
            && event["iteration"] == self.cancel_iteration
            && (!self.obstruction_probe || event["obstruction_set"] == self.cancel_set);
        event["returns_canceled"] = json!(cancel);
        self.events.borrow_mut().push(event);
        cancel.then_some(limits::ContextError::Cancelled)
    }

    fn scoring_state(
        &self,
        graph: &ArenaGraph,
        site: &'static str,
        node: Option<NodeId>,
        iteration: Option<usize>,
    ) {
        // The cursor belongs to the actual obstruction loop. A later
        // scorer boundary has left that loop and must not retain its index.
        if self.obstruction_probe && !matches!(site, "node-obstruction-set" | "node-obstruction") {
            self.obstruction_set.set(None);
        }
        let capture = || {
            let mut state = foreign_slice_state(self.original, graph);
            if self.obstruction_probe {
                state["native_projected_obstructions"] = json!({
                    "edge_inventories": graph.sized_projected_obstructions.iter().map(|(&(node, edge), obstructions)| json!({
                        "node": graph.nodes[node.0 as usize].tala_id,
                        "edge": graph.edge_records[edge.0 as usize].tala_id,
                        "nodes": obstructions.iter().map(|obstruction| obstruction.tala_id).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                    "abduction_inventories": graph.sized_edge_abductions.iter().map(|record| json!({
                        "edge": record.edge_tala_id,
                        "from_to": record.obstructions_from_to.iter().map(|obstruction| obstruction.tala_id).collect::<Vec<_>>(),
                        "to_from": record.obstructions_to_from.iter().map(|obstruction| obstruction.tala_id).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                });
            }
            state
        };
        let state = capture();
        if self.observer && (0..2).any(|_| capture() != state) {
            self.observation_error
                .borrow_mut()
                .get_or_insert_with(|| "read-only foreign-slice observer changed graph".into());
        }
        *self.current.borrow_mut() = json!({
            "site": site, "node": node.map(|node| graph.nodes[node.0 as usize].tala_id),
            "iteration": iteration, "state": state,
        });
        if self.obstruction_probe {
            self.current.borrow_mut()["obstruction_set"] = json!(self.obstruction_set.get());
        }
    }

    fn scoring_obstruction_set(&self, set_index: usize) {
        self.obstruction_set.set(Some(set_index));
    }

    fn scoring_obstruction_inventory(
        &self,
        graph: &ArenaGraph,
        scoring_node: NodeId,
        inventory: &OrderedObstructionInventory,
    ) {
        self.actual_obstruction_sets_present
            .set(inventory.sets_present);
        let replacement_owner = inventory
            .node_replacement
            .and_then(|node| self.original.topology_node_owner(node));
        let capture = || {
            Value::Array(inventory.sets.iter().enumerate().map(|(set_index, set)| {
            let items = set.items.iter().flatten().enumerate().map(|(item_index, item)| {
                let (position, size) = graph.projected_obstruction_box(item);
                let owner = item.source_node.and_then(|node| self.original.topology_node_owner(node));
                let actual_source = item.source_node.and_then(|node| self.original.topology_node(node));
                // A canonical handle does not prove that the compact scorer
                // node is physically the original owning ArenaNode pointer.
                let same_scoring_pointer = actual_source.is_some_and(|source| std::ptr::eq(source, &graph.nodes[scoring_node.0 as usize]));
                json!({
                    "index": item_index, "node_id": item.tala_id,
                    "top_left": symmetry_point(position), "width": bits(size.width), "height": bits(size.height),
                    "container": item.container_tala_id,
                    "graph_owner": owner.and_then(|owner| self.owner_names.get(&owner)).map(String::as_str).unwrap_or("other"),
                    "belongs_to_selected_nodes": item.source_node.is_some_and(|node| self.canonical_members.contains(&node)),
                    "same_graph_as_replacement": owner == replacement_owner,
                    "pointer_same_as_scoring_node": same_scoring_pointer,
                    "pointer_same_as_node_replacement": item.source_node.is_some() && item.source_node == inventory.node_replacement,
                    "pointer_same_as_adjacent_replacement": item.source_node.is_some() && item.source_node == inventory.adjacent_replacement,
                })
            }).collect::<Vec<_>>();
            json!({
                "index": set_index, "slice_present": set.items.is_some(), "length": items.len(), "items": items,
                // Copied retained-pointer inventories do not share the
                // backing allocation of the owner's Containers slice.
                "aliases_to_shared_container_child_slice": [],
            })
        }).collect())
        };
        let observed = capture();
        if self.observer && (0..2).any(|_| capture() != observed) {
            self.observation_error
                .borrow_mut()
                .get_or_insert_with(|| "full obstruction observer changed graph".into());
        }
        *self.actual_obstruction_sets.borrow_mut() = observed;
    }
}

// These addresses are actual borrowed primitive operands. Stable IDs select
// canonical arena slots; they are never substituted for pointer measurements.
fn owner_frame_probe_address<T>(value: Option<&T>) -> String {
    format!(
        "0x{:016x}",
        value.map_or(0, |value| value as *const T as usize)
    )
}

fn owner_frame_probe_node(owner: &ArenaGraph, id: Option<NodeId>) -> Value {
    let node = id.and_then(|id| owner.topology_node(id));
    let Some(node) = node else {
        return json!({"node_pointer": owner_frame_probe_address::<ArenaNode>(None), "nil": true});
    };
    let graph = id
        .and_then(|id| owner.topology_node_owner(id))
        .and_then(|graph| owner.owner_graph_view(graph));
    json!({
        "nil": false, "node_id": node.tala_id,
        "node_pointer": owner_frame_probe_address(Some(node)),
        "graph_pointer": owner_frame_probe_address(graph),
        "top_left_pointer": owner_frame_probe_address(node.position.as_ref()),
        "top_left": symmetry_point(node.position),
        "width": bits(node.rect.size.width), "height": bits(node.rect.size.height),
        "container_pointer": owner_frame_probe_address(node.container.and_then(|id| owner.topology_node(id))),
    })
}

fn owner_frame_probe_header(owner: &ArenaGraph, header: &SharedNodeSlice) -> Value {
    let capacity = header
        .capacity_slots()
        .iter()
        .enumerate()
        .map(|(index, &node)| {
            let mut row = owner_frame_probe_node(owner, node);
            row["index"] = json!(index);
            row["slot_pointer"] = json!(format!(
                "0x{:016x}",
                header.slot_address(index).unwrap_or(0)
            ));
            row
        })
        .collect::<Vec<_>>();
    json!({
        "nil": header.is_nil(), "length": header.len(), "capacity": header.capacity(),
        "data_pointer": format!("0x{:016x}", header.data_address().unwrap_or(0)),
        "logical_items": capacity.iter().take(header.len()).cloned().collect::<Vec<_>>(),
        "capacity_items": capacity,
    })
}

fn owner_frame_probe_map_header(
    owner: &ArenaGraph,
    receiver: OwnerGraphId,
    key: Option<NodeId>,
) -> Value {
    let header = owner.containers.get_header(&key);
    json!({
        "graph_pointer": owner_frame_probe_address(owner.owner_graph_view(receiver)),
        "map_pointer": format!("0x{:016x}", owner.containers.share_map().map_address().unwrap_or(0)),
        "key_pointer": owner_frame_probe_address(key.and_then(|key| owner.topology_node(key))),
        "key_id": key.and_then(|key| owner.topology_node(key)).map(|node| node.tala_id),
        "present": header.is_some(),
        "header": owner_frame_probe_header(owner, &header.unwrap_or_else(SharedNodeSlice::nil)),
    })
}

fn owner_frame_probe_relation(name: &str, operation: &str, lhs: Value, rhs: Value) -> Value {
    let equal = lhs == rhs;
    json!({"name": name, "operation": operation, "lhs": lhs, "rhs": rhs,
        "value": if operation == "not_equal" { !equal } else { equal }})
}

struct OwnerFrameProbeContext<'a> {
    owner: &'a ArenaGraph,
    receiver: OwnerGraphId,
    canonical_nodes: &'a [NodeId],
    owner_names: &'a BTreeMap<OwnerGraphId, String>,
    foreign: NodeId,
    observer: bool,
    set_cursor: Cell<Option<usize>>,
    current: RefCell<Value>,
    events: RefCell<Vec<Value>>,
    sets: RefCell<Vec<Value>>,
    primitives: RefCell<Vec<Value>>,
    observation_error: RefCell<Option<String>>,
}

impl OwnerFrameProbeContext<'_> {
    fn state(&self) -> Value {
        json!({
            "foreign": owner_frame_probe_node(self.owner, Some(self.foreign)),
            "current_map": owner_frame_probe_map_header(self.owner, self.receiver, None),
            "original": symmetry_graph_raw(self.owner, &symmetry_identity(self.owner), "canonical-owner"),
        })
    }

    fn observe_repeat(&self, first: &Value, capture: impl Fn() -> Value) {
        if self.observer && (0..2).any(|_| capture() != *first) {
            self.observation_error.borrow_mut().get_or_insert_with(|| {
                "read-only owner-frame observation changed actual primitive state".into()
            });
        }
    }
}

impl limits::Context for OwnerFrameProbeContext<'_> {
    fn done(&self) -> Option<bool> {
        None
    }

    fn error(&self) -> Option<limits::ContextError> {
        let mut event = self.current.borrow().clone();
        event["returns_canceled"] = json!(false);
        self.events.borrow_mut().push(event);
        None
    }

    fn scoring_state(
        &self,
        graph: &ArenaGraph,
        site: &'static str,
        node: Option<NodeId>,
        iteration: Option<usize>,
    ) {
        if !matches!(site, "node-obstruction-set" | "node-obstruction") {
            self.set_cursor.set(None);
        }
        let state = self.state();
        self.observe_repeat(&state, || self.state());
        *self.current.borrow_mut() = json!({
            "site": site, "node": node.map(|node| graph.nodes[node.0 as usize].tala_id),
            "iteration": iteration, "obstruction_set": self.set_cursor.get(), "state": state,
        });
    }

    fn scoring_obstruction_set(&self, set_index: usize) {
        self.set_cursor.set(Some(set_index));
    }

    fn scoring_owner_obstruction_inventory(
        &self,
        frame: &OwnerScoreFrame<'_>,
        scoring_local: NodeId,
        node_replacement: NodeId,
        adjacent_replacement: NodeId,
        sets: &[OwnerObstructionSet],
    ) {
        if !std::ptr::eq(frame.owner(), self.owner)
            || frame.canonical_node(scoring_local).ok()
                != self.canonical_nodes.get(scoring_local.0 as usize).copied()
        {
            self.observation_error.borrow_mut().get_or_insert_with(|| {
                "scoring callback frame does not borrow the observed canonical owner mapping".into()
            });
        }
        let scoring = self.canonical_nodes[scoring_local.0 as usize];
        let scoring_node = self.owner.topology_node(scoring);
        let replacement = self.owner.topology_node(node_replacement);
        let adjacent = self.owner.topology_node(adjacent_replacement);
        let replacement_graph = self
            .owner
            .topology_node_owner(node_replacement)
            .and_then(|id| self.owner.owner_graph_view(id));
        let view = self
            .owner
            .owner_graph_view(self.receiver)
            .expect("live selected receiver");
        let map_headers = std::iter::once(None)
            .chain(
                self.owner
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(index, _)| Some(NodeId(index as u32))),
            )
            .map(|key| owner_frame_probe_map_header(self.owner, self.receiver, key))
            .collect::<Vec<_>>();
        let capture = || {
            sets.iter().enumerate().map(|(set_index, set)| {
                let header = set.header.as_ref();
                let length = header.map_or(0, |header| header.len());
                let capacity = header.map_or(0, |header| header.capacity());
                let capacity_rows = (0..capacity).map(|index| {
                    let mut row = owner_frame_probe_node(self.owner,
                        header.and_then(|header| header.read_capacity_slot(index)).flatten());
                    row["index"] = json!(index);
                    row["slot_pointer"] = json!(format!("0x{:016x}", header.and_then(|header| header.slot_address(index)).unwrap_or(0)));
                    row
                }).collect::<Vec<_>>();
                let data_address = header.and_then(|header| header.data_address());
                let header_value = json!({
                    "nil": header.is_none_or(|header| header.is_nil()), "length": length, "capacity": capacity,
                    "data_pointer": format!("0x{:016x}", data_address.unwrap_or(0)),
                    "logical_items": capacity_rows.iter().take(length).cloned().collect::<Vec<_>>(),
                    "capacity_items": capacity_rows,
                });
                let items = (0..length).map(|index| {
                    let id = set.raw_slot(index).flatten();
                    let node = id.and_then(|id| self.owner.topology_node(id));
                    let graph = id.and_then(|id| self.owner.topology_node_owner(id))
                        .and_then(|id| self.owner.owner_graph_view(id));
                    let same_pointer = |other: Option<&ArenaNode>| node.zip(other)
                        .is_some_and(|(node, other)| std::ptr::eq(node, other));
                    json!({
                        "index": index, "node_id": node.map(|node| node.tala_id),
                        "top_left": symmetry_point(node.and_then(|node| node.position)),
                        "width": node.map(|node| bits(node.rect.size.width)),
                        "height": node.map(|node| bits(node.rect.size.height)),
                        "container": node.and_then(|node| node.container).map(|id| self.owner.nodes[id.0 as usize].tala_id),
                        "graph_owner": id.and_then(|id| self.owner.topology_node_owner(id))
                            .and_then(|id| self.owner_names.get(&id)).map(String::as_str).unwrap_or("other"),
                        "belongs_to_selected_nodes": id.is_some_and(|id| view.nodes.iter_slots().any(|slot| slot == Some(id))),
                        "same_graph_as_replacement": graph.zip(replacement_graph).is_some_and(|(a, b)| std::ptr::eq(a, b)),
                        "pointer_same_as_scoring_node": same_pointer(scoring_node),
                        "pointer_same_as_node_replacement": same_pointer(replacement),
                        "pointer_same_as_adjacent_replacement": same_pointer(adjacent),
                        "raw_pointer_metadata": owner_frame_probe_node(self.owner, id),
                    })
                }).collect::<Vec<_>>();
                let aliases = map_headers.iter().filter(|row| {
                    // This is the source observer's measured address/length
                    // relation; zero-cap equality is not allocation proof.
                    !header_value["nil"].as_bool().unwrap_or(true)
                        && row["present"] == true && row["header"]["length"] == length
                        && row["header"]["data_pointer"] == header_value["data_pointer"]
                }).map(|row| row["key_id"].clone()).collect::<Vec<_>>();
                json!({
                    "index": set_index, "slice_present": header.is_some_and(|header| !header.is_nil()),
                    "length": length, "items": items, "header": header_value,
                    "aliases_to_shared_container_child_slice": aliases,
                    "container_map_headers": map_headers, "container_map_pointer": format!("0x{:016x}", self.owner.containers.share_map().map_address().unwrap_or(0)),
                    "scoring_node_pointer": owner_frame_probe_address(scoring_node),
                    "node_replacement_pointer": owner_frame_probe_address(replacement),
                    "adjacent_replacement_pointer": owner_frame_probe_address(adjacent),
                })
            }).collect::<Vec<_>>()
        };
        let values = capture();
        self.observe_repeat(&json!(values), || json!(capture()));
        *self.sets.borrow_mut() = values;
        self.primitives.borrow_mut().push(json!({
            "frame_pointer": owner_frame_probe_address(Some(frame)),
            "frame_foreign_node_pointer": format!("0x{:016x}", frame.node_address(self.foreign).expect("actual canonical foreign node")),
            "frame_foreign_point_pointer": format!("0x{:016x}", frame.position_address(self.foreign).expect("actual canonical foreign geometry").unwrap_or(0)),
            "frame_foreign_graph_pointer": format!("0x{:016x}", frame.graph_address(self.foreign).expect("actual canonical foreign Graph").unwrap_or(0)),
            "canonical_scoring_node": owner_frame_probe_node(self.owner, Some(scoring)),
            "node_replacement": owner_frame_probe_node(self.owner, Some(node_replacement)),
            "adjacent_replacement": owner_frame_probe_node(self.owner, Some(adjacent_replacement)),
            "headers": sets.iter().map(|set| json!({
                "parent": set.parent.map(|node| self.owner.nodes[node.0 as usize].tala_id),
                "present": set.header.is_some(),
                "offset": set.header.as_ref().map(|header| header.offset()),
                "backing_pointer": format!("0x{:016x}", set.header.as_ref().and_then(|header| header.backing_address()).unwrap_or(0)),
                "data_pointer": format!("0x{:016x}", set.header.as_ref().and_then(|header| header.data_address()).unwrap_or(0)),
                "length": set.header.as_ref().map(|header| header.len()),
                "capacity": set.header.as_ref().map(|header| header.capacity()),
            })).collect::<Vec<_>>(),
        }));
    }
}

struct OwnerFrameProbeFixture {
    fixture: SymmetryFixture,
    component: ArenaGraph,
    center: NodeId,
    canonical_nodes: Vec<NodeId>,
    canonical_edges: Vec<EdgeId>,
    receiver: OwnerGraphId,
    foreign: NodeId,
    owner_names: BTreeMap<OwnerGraphId, String>,
    component_ownership: OwnerGraphOwnershipJournal,
}

fn prepared_owner_frame_probe_fixture() -> Result<OwnerFrameProbeFixture, String> {
    // Same real count-one forward C/c→L construction as the existing foreign
    // obstruction control. All ownership and abduction mutations run through
    // actual production preparation and canonical owner operations.
    let authored = [
        (1_i64, Point { x: 80.0, y: 80.0 }, 60.0, None),
        // Original Graph API assigns absolute TopLeft before AddNodeToContainer.
        (2_i64, Point { x: 100.0, y: 100.0 }, 10.0, Some(1_i64)),
        (3_i64, Point { x: -80.0, y: 100.0 }, 10.0, None),
        (1000_i64, Point { x: 300.0, y: 80.0 }, 60.0, None),
        (1001_i64, Point { x: 320.0, y: 100.0 }, 10.0, Some(1000_i64)),
    ];
    let ids = authored
        .iter()
        .enumerate()
        .map(|(index, row)| (row.0, NodeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut input = Graph::default();
    for &(tala, position, dimension, parent) in &authored {
        let mut node = Node::new(
            format!("foreign-slice-{tala}"),
            Size {
                width: dimension,
                height: dimension,
            },
        );
        node.parent = parent.map(|parent| ids[&parent]);
        let id = input.add_node(node);
        input.tala_id_overrides[id.0 as usize] = Some(tala);
        input.set_initial_position(id, Some(position));
        input.set_container_flag(id, parent.is_none() && tala != 3);
    }
    let edge = input.add_edge(Edge {
        source: ids[&2],
        target: ids[&3],
    });
    input.tala_edge_ids[edge.0 as usize] = Some(2000);
    let mut fixture =
        finish_prepared_symmetry_fixture_preserving_missing_positions(input, ids[&1], true)?;
    let components = fixture.temporary.graph.split_placement_subgraphs();
    let mut component_ownership = OwnerGraphOwnershipJournal::default();
    let mut owner_names = BTreeMap::from([(OwnerGraphId(0), "original".to_owned())]);
    if let Some(prepared) = &fixture.prepared_owner {
        owner_names.insert(prepared.view, "temporary".to_owned());
    }
    let mut receiver = None;
    for (index, local_roots) in components.iter().enumerate() {
        let roots = local_roots
            .iter()
            .map(|node| fixture.temporary_source_nodes[node])
            .collect::<Vec<_>>();
        let members = roots.iter().copied().collect::<BTreeSet<_>>();
        let edges = fixture
            .original
            .graph
            .edge_order
            .read()
            .iter()
            .copied()
            .filter(|edge| {
                let edge = &fixture.original.graph.edge_records[edge.0 as usize];
                members.contains(&edge.from) && members.contains(&edge.to)
            })
            .collect();
        let view = fixture
            .original
            .graph
            .create_owner_graph_view(roots.clone(), edges)
            .map_err(|error| error.to_string())?;
        fixture
            .original
            .graph
            .set_owner_view_node_mapping(view, roots.clone())
            .map_err(|error| error.to_string())?;
        fixture
            .original
            .graph
            .set_owner_graph_reference(view, &roots, false, &mut component_ownership)
            .map_err(|error| error.to_string())?;
        owner_names.insert(view, format!("component-{index}"));
        if local_roots.contains(&fixture.center) {
            let mut descendant_ownership = OwnerGraphOwnershipJournal::default();
            fixture
                .original
                .graph
                .set_owner_graph_reference(view, &roots, true, &mut descendant_ownership)
                .map_err(|error| error.to_string())?;
            receiver = Some(view);
        }
    }
    let members = components
        .iter()
        .find(|members| members.contains(&fixture.center))
        .ok_or_else(|| "actual component splitter omitted C".to_owned())?;
    let (mut component, local_roots) = fixture.temporary.graph.induced_placement_subgraph(members);
    let center = NodeId(
        local_roots
            .iter()
            .position(|node| *node == fixture.center)
            .ok_or_else(|| "actual component induction omitted C".to_owned())? as u32,
    );
    let canonical_nodes = local_roots
        .iter()
        .map(|node| fixture.temporary_source_nodes[node])
        .collect::<Vec<_>>();
    let receiver = receiver.ok_or_else(|| "actual owning selected view omitted C".to_owned())?;
    fixture
        .original
        .graph
        .set_owner_graph_cell_size(receiver, 10.0)
        .map_err(|error| error.to_string())?;
    fixture
        .original
        .graph
        .set_owner_view_node_mapping(receiver, canonical_nodes.clone())
        .map_err(|error| error.to_string())?;
    component.cell_size = 10.0;
    let canonical_edges = fixture
        .original
        .graph
        .owner_graph_view(receiver)
        .expect("retained actual selected edge membership")
        .edges
        .read();
    Ok(OwnerFrameProbeFixture {
        fixture,
        component,
        center,
        canonical_nodes,
        canonical_edges,
        receiver,
        foreign: ids[&1000],
        owner_names,
        component_ownership,
    })
}

fn owner_frame_cost_views(prepared: &OwnerFrameProbeFixture) -> Value {
    owner_frame_cost_views_from(&prepared.fixture.original.graph, &prepared.owner_names)
}

fn owner_frame_cost_views_from(
    owner: &ArenaGraph,
    names: &BTreeMap<OwnerGraphId, String>,
) -> Value {
    let mut views = Vec::new();
    for (&id, name) in names {
        let view = owner.owner_graph_view(id).expect("retained actual view");
        let costs = owner
            .owner_graph_routing_costs(id)
            .expect("actual receiver cache");
        views.push(json!({
            "name": name,
            "graph_pointer": format!("{:p}", view),
            "cell": bits(owner.owner_graph_cell_size(id).expect("actual receiver cell")),
            "cached_crossing": bits(costs.0), "cached_turn": bits(costs.1),
            "cached_noncenter": bits(costs.2),
            "nodes": view.nodes.iter_slots().map(|node| {
                let mut row = owner_frame_probe_node(owner, node);
                let Some(node) = node else { return row; };
                let actual = &owner.nodes[node.0 as usize];
                row["incident_edge_order"] = json!(actual.edges.read().iter().map(|edge| owner.edge_records[edge.0 as usize].tala_id).collect::<Vec<_>>());
                row["historical_container_flag"] = json!(actual.is_container);
                row
            }).collect::<Vec<_>>(),
            "edges": view.edges.read().iter().map(|&edge| {
                let actual = &owner.edge_records[edge.0 as usize];
                json!({"id": actual.tala_id,
                    "from": owner.nodes[actual.from.0 as usize].tala_id,
                    "to": owner.nodes[actual.to.0 as usize].tala_id,
                    "edge_pointer": format!("{:p}", actual),
                    "points": actual.route.read().iter().map(|point| json!({"x": bits(point.x), "y": bits(point.y)})).collect::<Vec<_>>()})
            }).collect::<Vec<_>>(),
        }));
    }
    json!(views)
}

fn owner_frame_cost_step(
    prepared: &OwnerFrameProbeFixture,
    name: &str,
    observer: bool,
    direction_penalty: bool,
    capture_direction_inputs: bool,
) -> Result<Value, String> {
    let owner = &prepared.fixture.original.graph;
    let before_views = owner_frame_cost_views(prepared);
    let mut frame = OwnerScoreFrame::new(
        owner,
        owner.topology_arena_id().expect("actual owning arena"),
        prepared.receiver,
        &prepared.canonical_nodes,
    )
    .map_err(|error| error.to_string())?
    .with_abductions(
        &prepared
            .fixture
            .prepared_owner
            .as_ref()
            .expect("actual prepared records")
            .abductions,
    )
    .map_err(|error| error.to_string())?;
    // The mapping survives Graph.Edges membership changes. The concrete
    // incident pointer is retained independently of selected.Nodes/Edges.
    frame = frame
        .with_edge_mapping(&prepared.canonical_edges)
        .map_err(|error| error.to_string())?;
    let context = OwnerFrameProbeContext {
        owner,
        receiver: prepared.receiver,
        canonical_nodes: &prepared.canonical_nodes,
        owner_names: &prepared.owner_names,
        foreign: prepared.foreign,
        observer,
        set_cursor: Cell::new(None),
        current: RefCell::new(Value::Null),
        events: RefCell::new(Vec::new()),
        sets: RefCell::new(Vec::new()),
        primitives: RefCell::new(Vec::new()),
        observation_error: RefCell::new(None),
    };
    let value = prepared
        .component
        .sized_edge_length_with_owner_frame_checked(
            prepared.center,
            direction_penalty,
            false,
            false,
            false,
            true,
            &frame,
            crate::internal::scoring_context::ScoreContext::new(&context),
        )
        .map_err(|error| error.to_string())?;
    if let Some(error) = context.observation_error.borrow().as_ref() {
        return Err(error.clone());
    }
    let sets = context.sets.borrow().clone();
    let events = context.events.borrow().clone();
    let primitives = context.primitives.borrow().clone();
    let mut result = json!({"name": name, "score": bits(value), "error": "",
        "before_views": before_views, "after_views": owner_frame_cost_views(prepared),
        "actual_obstruction_sets_present": !sets.is_empty(), "actual_obstruction_sets": sets,
        "context_observations": events, "actual_frame_callbacks": primitives,
        "foreign": owner_frame_probe_node(owner, Some(prepared.foreign)),
        "current_map": owner_frame_probe_map_header(owner, prepared.receiver, None)});
    if capture_direction_inputs {
        let go_orientation = |direction: Orientation| match direction {
            Orientation::TopLeft => 0,
            Orientation::TopRight => 1,
            Orientation::BottomLeft => 2,
            Orientation::BottomRight => 3,
            Orientation::Top => 4,
            Orientation::Right => 5,
            Orientation::Bottom => 6,
            Orientation::Left => 7,
            Orientation::None => 8,
        };
        let scoring = frame
            .canonical_node(prepared.center)
            .map_err(|error| error.to_string())?;
        let actual = frame.node(scoring).map_err(|error| error.to_string())?;
        let edge = frame
            .edge(
                *actual
                    .edges
                    .read()
                    .first()
                    .ok_or("source API fixture needs an incident edge")?,
            )
            .map_err(|error| error.to_string())?;
        let adjacent = if edge.from == scoring {
            edge.to
        } else {
            edge.from
        };
        let mut replacement = scoring;
        let mut adjacent_replacement = adjacent;
        for &record in &frame
            .abductions()
            .ok_or("source API fixture needs explicit records")?
            .records
        {
            let record = owner
                .owner_edge_abduction(record)
                .ok_or("missing live record")?;
            if record.current_from == Some(scoring) && record.current_to == Some(adjacent) {
                replacement = record.originally_from.unwrap_or(scoring);
                adjacent_replacement = record.originally_to.unwrap_or(adjacent);
                break;
            }
            if record.current_to == Some(scoring) && record.current_from == Some(adjacent) {
                replacement = record.originally_to.unwrap_or(scoring);
                adjacent_replacement = record.originally_from.unwrap_or(adjacent);
                break;
            }
        }
        let mut outer = replacement;
        if frame
            .node(replacement)
            .map_err(|error| error.to_string())?
            .container
            != frame
                .node(adjacent_replacement)
                .map_err(|error| error.to_string())?
                .container
            && frame
                .raw_container_depth(replacement)
                .map_err(|error| error.to_string())?
                > frame
                    .raw_container_depth(adjacent_replacement)
                    .map_err(|error| error.to_string())?
        {
            outer = adjacent_replacement;
        }
        let directed = edge.source_arrow != edge.target_arrow;
        let semantic_from = directed.then_some(if edge.source_arrow {
            edge.to
        } else {
            edge.from
        });
        let semantic_to = directed.then_some(if edge.source_arrow {
            edge.from
        } else {
            edge.to
        });
        let root_direction = owner
            .owner_graph_direction_entry(prepared.receiver, None)
            .map_err(|error| error.to_string())?;
        let child_direction = owner
            .owner_graph_direction_entry(prepared.receiver, Some(scoring))
            .map_err(|error| error.to_string())?;
        result["direction_inputs"] = json!({
            "root_direction_present": root_direction.is_some(),
            "root_direction": root_direction.map(go_orientation).unwrap_or(0),
            "child_direction_present": child_direction.is_some(),
            "child_direction": child_direction.map(go_orientation).unwrap_or(0),
            "actual_scoring_container_direction": go_orientation(frame.container_direction(scoring).map_err(|error| error.to_string())?),
            "actual_replacement_container_direction": go_orientation(frame.container_direction(replacement).map_err(|error| error.to_string())?),
            "actual_outer_container_direction": go_orientation(frame.container_direction(outer).map_err(|error| error.to_string())?),
            "source_arrow_present": edge.source_arrow, "target_arrow_present": edge.target_arrow, "directed": directed,
            "semantic_from": owner_frame_probe_node(owner, semantic_from), "semantic_to": owner_frame_probe_node(owner, semantic_to),
            "edge_from": owner_frame_probe_node(owner, Some(edge.from)), "edge_to": owner_frame_probe_node(owner, Some(edge.to)),
        });
    }
    Ok(result)
}

struct CandidateMovementContext {
    polls: Cell<u64>,
    cancel_at: Option<u64>,
    scoring_observations: RefCell<Vec<Value>>,
    header_change: Option<CandidateMovementHeaderChange>,
}
enum CandidateMovementHeaderChange {
    Slot {
        header: SharedNodeSlice,
        index: usize,
        node: Option<NodeId>,
    },
    Replace {
        map: SharedSliceMap,
        parent: NodeId,
        header: SharedNodeSlice,
    },
}
struct CandidateMovementPanicPayload;

fn candidate_member_nodes(owner: &ArenaGraph, receiver: OwnerGraphId) -> Vec<Value> {
    let members = owner.owner_graph_view(receiver).unwrap().nodes.header();
    members
        .iter_slots()
        .enumerate()
        .map(|(index, node)| {
            let mut row = owner_frame_probe_node(owner, node);
            row["index"] = json!(index);
            row["slot_pointer"] = json!(format!(
                "0x{:016x}",
                members.slot_address(index).unwrap_or(0)
            ));
            row
        })
        .collect()
}
impl limits::Context for CandidateMovementContext {
    fn done(&self) -> Option<bool> {
        None
    }
    fn error(&self) -> Option<limits::ContextError> {
        let poll = self.polls.get() + 1;
        self.polls.set(poll);
        if poll == 2 {
            match &self.header_change {
                Some(CandidateMovementHeaderChange::Slot {
                    header,
                    index,
                    node,
                }) => {
                    header.write_capacity_slot(*index, *node).unwrap();
                }
                Some(CandidateMovementHeaderChange::Replace {
                    map,
                    parent,
                    header,
                }) => {
                    map.replace(Some(*parent), header.clone()).unwrap();
                }
                None => {}
            }
        }
        self.cancel_at
            .filter(|&at| poll >= at)
            .map(|_| limits::ContextError::Cancelled)
    }

    fn scoring_state(
        &self,
        owner: &ArenaGraph,
        site: &'static str,
        node: Option<NodeId>,
        iteration: Option<usize>,
    ) {
        self.scoring_observations.borrow_mut().push(json!({
            "site": site, "actual_poll_ordinal": self.polls.get()+1,
            "node_id": node.map(|node| owner.nodes[node.0 as usize].tala_id), "iteration": iteration,
            "nodes": (0..owner.nodes.len()).map(|index| owner_frame_probe_node(owner, Some(NodeId(index as u32)))).collect::<Vec<_>>(),
        }));
    }
}

fn diagnostic_owner_candidate_movement_probe(request: &Value) -> Value {
    let profile = request
        .get("profile")
        .and_then(Value::as_str)
        .unwrap_or("live-child");
    if !matches!(
        profile,
        "live-child"
            | "same-position"
            | "nil-child-position"
            | "nil-descendant"
            | "retained-cycle"
            | "live-header-change"
            | "capture-future-slot-change"
            | "capture-loaded-slot-change"
            | "capture-replaced-header"
    ) {
        return json!({"supported": false, "request_error": "unknown candidate movement profile"});
    }
    let outcome = request
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("commit");
    if !matches!(outcome, "commit" | "drop" | "panic") {
        return json!({"supported": false, "request_error": "unknown candidate movement outcome"});
    }
    let mut points = Vec::new();
    if let Some(authored) = request.get("points") {
        let Some(authored) = authored.as_array() else {
            return json!({"supported": false, "request_error": "points must be an array"});
        };
        for point in authored {
            let Some(point) = point.as_array().filter(|point| point.len() == 2) else {
                return json!({"supported": false, "request_error": "each point must contain x and y"});
            };
            let (Some(x), Some(y)) = (point[0].as_f64(), point[1].as_f64()) else {
                return json!({"supported": false, "request_error": "point coordinates must be numbers"});
            };
            points.push(Point { x, y });
        }
    } else if profile == "same-position" {
        points.push(Point { x: 80.0, y: 80.0 });
    } else {
        points.extend([Point { x: 90.0, y: 95.0 }, Point { x: 400.0, y: 300.0 }]);
    }
    let mut prepared = match prepared_owner_frame_probe_fixture() {
        Ok(prepared) => prepared,
        Err(error) => return json!({"supported": false, "preparation_error": error}),
    };
    let center = prepared.canonical_nodes[prepared.center.0 as usize];
    let receiver = prepared.receiver;
    let owner = &mut prepared.fixture.original.graph;
    let child = owner
        .containers
        .get_header(&Some(center))
        .unwrap()
        .get(0)
        .unwrap()
        .unwrap();
    let leaf = owner
        .owner_graph_view(receiver)
        .unwrap()
        .nodes
        .require_nodes()
        .find(|&node| node != center)
        .unwrap();
    owner.set_position(leaf, Point { x: 500.0, y: 100.0 });
    match profile {
        "nil-child-position" => owner.clear_position(child),
        "nil-descendant" => {
            owner
                .containers
                .get_header(&Some(center))
                .unwrap()
                .write_capacity_slot(0, None)
                .unwrap();
        }
        "retained-cycle" => {
            owner.nodes[child.0 as usize].is_container = true;
            owner.containers.insert(Some(child), vec![center, child]);
        }
        "capture-future-slot-change" | "capture-loaded-slot-change" | "capture-replaced-header" => {
            owner.containers.insert(Some(center), vec![child; 65]);
        }
        _ => {}
    }
    let capture = |owner: &ArenaGraph| {
        json!({
            "nodes": (0..owner.nodes.len()).map(|index| owner_frame_probe_node(owner, Some(NodeId(index as u32)))).collect::<Vec<_>>(),
            "selected_member_nodes": candidate_member_nodes(owner, receiver),
            "selected_C_header": owner_frame_probe_map_header(owner, receiver, Some(center)),
            "selected_root_header": owner_frame_probe_map_header(owner, receiver, None),
        })
    };
    let context = CandidateMovementContext {
        polls: Cell::new(0),
        scoring_observations: RefCell::new(Vec::new()),
        header_change: match profile {
            "capture-future-slot-change" => Some(CandidateMovementHeaderChange::Slot {
                header: owner.containers.get_header(&Some(center)).unwrap(),
                index: 0,
                node: Some(prepared.foreign),
            }),
            "capture-loaded-slot-change" => Some(CandidateMovementHeaderChange::Slot {
                header: owner.containers.get_header(&Some(center)).unwrap(),
                index: 1,
                node: None,
            }),
            "capture-replaced-header" => Some(CandidateMovementHeaderChange::Replace {
                map: owner.containers.share_map(),
                parent: center,
                header: SharedNodeSlice::from_nodes(&[prepared.foreign; 65]).unwrap(),
            }),
            _ => None,
        },
        cancel_at: request
            .get("cancel_at_poll")
            .and_then(Value::as_u64)
            .filter(|&poll| poll != 0),
    };
    let mapping = CanonicalScoreMapping::new(
        owner.topology_arena_id().unwrap(),
        prepared.canonical_nodes.clone(),
        prepared.canonical_edges.clone(),
    );
    let mut session = match OwnerScoreSession::new(owner, receiver, mapping, None) {
        Ok(session) => session,
        Err(error) => return json!({"supported": false, "binding_error": error.to_string()}),
    };
    let entry = capture(session.owner());
    let mut events = Vec::new();
    let mut work_used = 0;
    let mut error = None;
    let mut panic_caught = false;
    let panic_payload = Arc::new(CandidateMovementPanicPayload);
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut guard = match limits::OptimizationWorkGuard::new(
            Some(&context),
            "sizelessOptimizer.optimize",
            request
                .get("work_limit")
                .and_then(Value::as_u64)
                .unwrap_or(1000),
        ) {
            Ok(guard) => guard,
            Err(cause) => {
                error = Some(cause.to_string());
                return;
            }
        };
        let mut movement = match session.optimizer_candidate_movement(prepared.center, &mut guard) {
            Ok(movement) => movement,
            Err(cause) => {
                error = Some(cause.to_string());
                work_used = guard.used();
                return;
            }
        };
        let captured_nodes = movement.captured_nodes().collect::<Vec<_>>();
        let descendant_work = movement.descendant_work();
        events.push(json!({"phase": "after_capture", "state": capture(movement.session().owner()), "work_used": guard.used(), "polls": context.polls.get(),
            "captured_nodes": captured_nodes.iter().map(|&node| owner_frame_probe_node(movement.session().owner(), Some(node))).collect::<Vec<_>>(),
            "descendant_work": descendant_work}));
        if profile == "live-header-change" {
            movement
                .session()
                .owner()
                .containers
                .get_header(&Some(center))
                .unwrap()
                .write_capacity_slot(0, Some(prepared.foreign))
                .unwrap();
        }
        for (index, &point) in points.iter().enumerate() {
            let result = movement.move_abs(point, &mut guard);
            events.push(json!({"phase": "after_move", "index": index, "state": capture(movement.session().owner()), "work_used": guard.used(), "polls": context.polls.get(), "error": result.as_ref().err().map(ToString::to_string)}));
            work_used = guard.used();
            if let Err(cause) = result {
                error = Some(cause.to_string());
                return;
            }
        }
        work_used = guard.used();
        if outcome == "panic" {
            std::panic::panic_any(Arc::clone(&panic_payload));
        }
        if outcome == "commit" {
            movement.commit();
        }
    }));
    if run.is_err() {
        panic_caught = true;
    }
    let panic_exact_payload = run
        .as_ref()
        .err()
        .and_then(|payload| payload.downcast_ref::<Arc<CandidateMovementPanicPayload>>())
        .is_some_and(|payload| Arc::ptr_eq(payload, &panic_payload));
    let after_cleanup = capture(session.owner());
    json!({"supported": true, "probe": "owner-candidate-movement", "profile": profile, "outcome": outcome,
        "result": {"entry": entry, "events": events, "after_cleanup": after_cleanup, "error": error, "panic_caught": panic_caught, "panic_exact_payload": panic_exact_payload, "work_used": work_used, "polls": context.polls.get()},
        "full_operation_trace_parity": false, "full_goal_complete": false,
        "evidence_limits": "Actual owner candidate movement and search positional cleanup. Full optimizer pass publication, groups, placement-cost rollback and Point allocation identity remain open."})
}

fn diagnostic_owner_sizeless_candidate_probe(request: &Value) -> Value {
    let mut prepared = match prepared_owner_frame_probe_fixture() {
        Ok(prepared) => prepared,
        Err(error) => return json!({"supported": false, "preparation_error": error}),
    };
    let mut points = Vec::new();
    if let Some(authored) = request.get("points") {
        let Some(authored) = authored.as_array() else {
            return json!({"supported": false, "request_error": "points must be an array"});
        };
        for point in authored {
            if point.is_null() {
                points.push(None);
                continue;
            }
            let Some(point) = point.as_array().filter(|point| point.len() == 2) else {
                return json!({"supported": false, "request_error": "each point must contain x and y"});
            };
            let (Some(x), Some(y)) = (point[0].as_f64(), point[1].as_f64()) else {
                return json!({"supported": false, "request_error": "point coordinates must be numbers"});
            };
            points.push(Some(Point { x, y }));
        }
    } else {
        points.extend([
            Some(Point { x: 90.0, y: 95.0 }),
            Some(Point { x: 400.0, y: 300.0 }),
            Some(Point { x: 80.0, y: 80.0 }),
        ]);
    }
    let receiver = prepared.receiver;
    let center = prepared.canonical_nodes[prepared.center.0 as usize];
    let prepared_slice = request
        .get("prepared_session")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then(|| &prepared.fixture.prepared_owner.as_ref().unwrap().abductions);
    let owner = &mut prepared.fixture.original.graph;
    let leaf = owner
        .owner_graph_view(receiver)
        .unwrap()
        .nodes
        .require_nodes()
        .find(|&node| node != center)
        .unwrap();
    owner.set_position(leaf, Point { x: 500.0, y: 100.0 });
    if request
        .get("stale_workspace")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        for (index, node) in prepared.component.nodes.iter_mut().enumerate() {
            node.position = Some(Point {
                x: 9000.0 + index as f64,
                y: -9000.0,
            });
            node.rect.origin = node.position.unwrap();
        }
    }
    let capture = |owner: &ArenaGraph| {
        json!({
            "nodes": (0..owner.nodes.len()).map(|index| owner_frame_probe_node(owner, Some(NodeId(index as u32)))).collect::<Vec<_>>(),
            "selected_member_nodes": candidate_member_nodes(owner, receiver),
            "selected_C_header": owner_frame_probe_map_header(owner, receiver, Some(center)),
            "selected_root_header": owner_frame_probe_map_header(owner, receiver, None),
        })
    };
    let entry = capture(owner);
    let mapping = CanonicalScoreMapping::new(
        owner.topology_arena_id().unwrap(),
        prepared.canonical_nodes.clone(),
        prepared.canonical_edges.clone(),
    );
    let mut session = match OwnerScoreSession::new(owner, receiver, mapping, prepared_slice) {
        Ok(session) => session,
        Err(error) => return json!({"supported": false, "binding_error": error.to_string()}),
    };
    let capture_abductions = |session: &OwnerScoreSession<'_>| {
        session.abductions().map(|slice| json!({
            "present": slice.present,
            "arena": slice.arena.map(|arena| arena.0),
            "slice_pointer": owner_frame_probe_address(Some(slice)),
            "records_pointer": format!("0x{:016x}", slice.records.as_ptr() as usize),
            "records": slice.records.iter().map(|&id| {
                let record = session.owner().owner_edge_abduction(id);
                json!({"record_id": id.0, "exists": record.is_some(),
                    "source_edge": record.and_then(|record| record.source_edge).map(|edge| edge.0),
                    "originally_from": record.and_then(|record| record.originally_from).map(|node| node.0),
                    "originally_to": record.and_then(|record| record.originally_to).map(|node| node.0),
                    "current_from": record.and_then(|record| record.current_from).map(|node| node.0),
                    "current_to": record.and_then(|record| record.current_to).map(|node| node.0)})
            }).collect::<Vec<_>>()
        }))
    };
    let session_abductions_before = capture_abductions(&session);
    let context = CandidateMovementContext {
        polls: Cell::new(0),
        scoring_observations: RefCell::new(Vec::new()),
        header_change: None,
        cancel_at: request
            .get("cancel_at_poll")
            .and_then(Value::as_u64)
            .filter(|&poll| poll != 0),
    };
    let mut work_used = 0;
    let mut changed = None;
    let mut error = None;
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut guard = match limits::OptimizationWorkGuard::new(
            Some(&context),
            "sizelessOptimizer.optimize",
            request
                .get("work_limit")
                .and_then(Value::as_u64)
                .unwrap_or(1000),
        ) {
            Ok(guard) => guard,
            Err(cause) => {
                error = Some(cause.to_string());
                return;
            }
        };
        let mut optimizer = sizeless::SizelessOptimizer::with_rng(
            &mut prepared.component,
            go_rng::GoRng::new_untraced(0),
        );
        match optimizer.move_node_to_best_with_owner_nullable_session(
            prepared.center,
            &points,
            &mut session,
            &context,
            &mut guard,
        ) {
            Ok(value) => changed = Some(value),
            Err(cause) => error = Some(cause.to_string()),
        }
        work_used = guard.used();
    }));
    // The Go fixture has one set of nodes. Dense carriers are a Rust workspace:
    // refresh them also when NewGuard's initial poll exits before the checked
    // candidate API is entered. This copies actual owner positions only.
    for (local, &canonical) in prepared.canonical_nodes.iter().enumerate() {
        let actual = &session.owner().nodes[canonical.0 as usize];
        prepared.component.nodes[local].position = actual.position;
        prepared.component.nodes[local].rect.origin = actual.rect.origin;
    }
    json!({"supported": true, "probe": "owner-sizeless-candidate-search", "result": {
        "entry": entry, "after_cleanup": capture(session.owner()), "changed": changed,
        "error": error, "panic_caught": run.is_err(), "work_used": work_used, "polls": context.polls.get(),
        "workspace_positions": prepared.component.nodes.iter().map(|node| json!({"node_id": node.tala_id, "top_left": symmetry_point(node.position)})).collect::<Vec<_>>(),
        "session_abductions_before": session_abductions_before,
        "session_abductions_after": capture_abductions(&session),
        "scoring_observations": context.scoring_observations.borrow().clone()},
        "full_operation_trace_parity": false, "full_goal_complete": false,
        "evidence_limits": "Checked real-owner candidate search with live sizeless scoring. Whole optimizer pass, candidate generation/swaps/compaction, groups and production pipeline wiring remain unqualified."})
}

fn diagnostic_owner_initialize_sync_probe(request: &Value) -> Value {
    let profile = request
        .get("profile")
        .and_then(Value::as_str)
        .unwrap_or("rectangle-live-child");
    let (shape_type, width, height, fixed, map_mode, actual_initialize) = match profile {
        "rectangle-live-child" => ("", 60.0, 60.0, false, "keep", false),
        "rectangle-wide-live-child" => ("", 140.0, 100.0, false, "keep", false),
        "circle-live-child" => ("circle", 120.0, 120.0, false, "keep", false),
        "canonical-circle-live-child" => ("Circle", 120.0, 120.0, false, "keep", false),
        "rectangle-fixed-child" => ("", 60.0, 60.0, true, "keep", false),
        "empty-present-nil" => ("", 60.0, 60.0, false, "nil", false),
        "empty-allocated" => ("", 60.0, 60.0, false, "allocated", false),
        "empty-absent" => ("", 60.0, 60.0, false, "absent", false),
        "actual-initialize-then-sync" => ("", 60.0, 60.0, false, "keep", true),
        _ => return json!({"supported": false, "request_error": "unknown initialize/sync profile"}),
    };
    let mut prepared = match prepared_owner_frame_probe_fixture() {
        Ok(prepared) => prepared,
        Err(error) => return json!({"supported": false, "preparation_error": error}),
    };
    let canonical_center = prepared.canonical_nodes[prepared.center.0 as usize];
    let local_leaf = prepared
        .component
        .graph_node_order()
        .into_iter()
        .find(|node| *node != prepared.center)
        .expect("source API fixture has C and L members");
    let canonical_leaf = prepared.canonical_nodes[local_leaf.0 as usize];
    let owner = &mut prepared.fixture.original.graph;
    owner.set_position(canonical_leaf, Point { x: 500.0, y: 100.0 });
    owner.nodes[canonical_center.0 as usize].rect.size = Size { width, height };
    if !shape_type.is_empty() {
        owner.set_node_shape_type(canonical_center, shape_type);
    }
    let child = owner
        .owner_container_map(prepared.receiver)
        .unwrap()
        .get(Some(canonical_center))
        .unwrap()
        .get(0)
        .unwrap()
        .unwrap();
    if fixed {
        owner.nodes[child.0 as usize].fixed_top_left = Some(Point { x: 100.0, y: 100.0 });
    }
    match map_mode {
        "nil" => owner
            .containers
            .replace_header(Some(canonical_center), SharedNodeSlice::nil()),
        "allocated" => owner.containers.replace_header(
            Some(canonical_center),
            SharedNodeSlice::with_len_cap(0, 0).unwrap(),
        ),
        "absent" => {
            owner.containers.remove(&Some(canonical_center));
        }
        _ => {}
    }
    prepared.component.nodes[prepared.center.0 as usize]
        .rect
        .size = Size { width, height };
    if !shape_type.is_empty() {
        prepared
            .component
            .set_node_shape_type(prepared.center, shape_type);
    }
    prepared
        .component
        .set_position(local_leaf, Point { x: 500.0, y: 100.0 });
    let capture = |owner: &ArenaGraph| {
        let view = owner.owner_graph_view(prepared.receiver).unwrap();
        json!({
            "nodes": (0..owner.nodes.len()).map(|index| owner_frame_probe_node(owner, Some(NodeId(index as u32)))).collect::<Vec<_>>(),
            "selected_member_nodes": view.nodes.iter_slots().map(|node| owner_frame_probe_node(owner, node)).collect::<Vec<_>>(),
            "selected_C_header": owner_frame_probe_map_header(owner, prepared.receiver, Some(canonical_center)),
            "selected_root_header": owner_frame_probe_map_header(owner, prepared.receiver, None),
            "full_state": owner_frame_cost_views_from(owner, &prepared.owner_names),
            "native_node_storage_pointer": format!("{:p}", owner.nodes.as_ptr()),
        })
    };
    let entry = capture(owner);
    if actual_initialize {
        let mut pipeline =
            Pipeline::new_untraced(&prepared.fixture.temporary.input, 0, false, true);
        pipeline.graph = prepared.component;
        pipeline.run_initialize_nodes().unwrap();
        prepared.component = pipeline.graph;
    } else {
        prepared
            .component
            .set_position(prepared.center, Point { x: -35.0, y: -45.0 });
        prepared
            .component
            .set_position(local_leaf, Point { x: 15.0, y: 25.0 });
    }
    let mapping = CanonicalScoreMapping::new(
        owner.topology_arena_id().unwrap(),
        prepared.canonical_nodes.clone(),
        prepared.canonical_edges.clone(),
    );
    let mut session = match OwnerScoreSession::new(owner, prepared.receiver, mapping, None) {
        Ok(session) => session,
        Err(error) => return json!({"supported": false, "binding_error": error.to_string()}),
    };
    let initialize_error = session
        .assign_current_member_positions(&prepared.component)
        .err()
        .map(|error| error.to_string());
    let after_initialize = capture(session.owner());
    let sync_error = if initialize_error.is_none() {
        session
            .sync_current_nested_geometry()
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };
    let after_sync = capture(session.owner());
    let before_repeat = capture(session.owner());
    let repeat_error = if sync_error.is_none() {
        session
            .sync_current_nested_geometry()
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };
    let after_repeat = capture(session.owner());
    json!({"supported": true, "probe": "owner-initialize-sync", "profile": profile,
        "observer": request.get("observer").and_then(Value::as_bool).unwrap_or(false),
        "result": {"profile": profile, "shape_input": shape_type,
            "actual_shape_type": session.owner().node_shape_type(canonical_center),
            "actual_initialize_nodes_called": actual_initialize,
            "initialize_error": initialize_error, "sync_error": sync_error, "repeat_error": repeat_error,
            "entry": entry, "after_initialize": after_initialize, "after_sync": after_sync,
            "before_repeat": before_repeat, "after_repeat": after_repeat},
        "full_operation_trace_parity": false, "full_goal_complete": false,
        "evidence_limits": "Actual owner member positions and canonical retained-child sync. Explicit source API state profiles; full shape/label/nil/panic/work-poll/Point allocation and optimizer trial publication remain unqualified."})
}

fn diagnostic_owner_frame_cost_probe(request: &Value) -> Value {
    let observer = match request.get("observer") {
        Some(Value::Bool(value)) => *value,
        None => false,
        _ => return json!({"supported": false, "request_error": "observer must be boolean"}),
    };
    let cell = request.get("cell").and_then(Value::as_f64).unwrap_or(10.0);
    let direction_penalty = request
        .get("direction_penalty")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let capture_direction_inputs = request
        .get("capture_direction_inputs")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let input_costs = request.get("routing_costs");
    let costs = (
        input_costs
            .and_then(|value| value.get("crossing"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        input_costs
            .and_then(|value| value.get("turn"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        input_costs
            .and_then(|value| value.get("noncenter"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
    );
    let operation = request
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or("Keep");
    if !matches!(
        operation,
        "Keep" | "RestoreTurn31" | "RestoreTurnZero" | "ResetTurnCost" | "HalveTurnCost"
    ) {
        return json!({"supported": false, "request_error": "unknown selected cost operation"});
    }
    let mut prepared = match prepared_owner_frame_probe_fixture() {
        Ok(prepared) => prepared,
        Err(error) => return json!({"supported": false, "preparation_error": error}),
    };
    let owner = &mut prepared.fixture.original.graph;
    let canonical_center = prepared.canonical_nodes[prepared.center.0 as usize];
    for (field, parent) in [
        ("root_direction", None),
        ("child_direction", Some(canonical_center)),
    ] {
        if let Some(value) = request.get(field).and_then(Value::as_str) {
            let direction = match value {
                "unset" => None,
                "explicit-none" => Some(Orientation::None),
                "right" => Some(Orientation::Right),
                "left" => Some(Orientation::Left),
                "up" => Some(Orientation::Top),
                "down" => Some(Orientation::Bottom),
                _ => {
                    return json!({"supported": false, "request_error": "unknown authored direction"});
                }
            };
            if let Err(error) =
                owner.set_owner_graph_direction_entry(prepared.receiver, parent, direction)
            {
                return json!({"supported": false, "request_error": error.to_string()});
            }
        }
    }
    for (local, &canonical) in prepared.canonical_edges.iter().enumerate() {
        if let Some(value) = request.get("source_arrow").and_then(Value::as_bool) {
            owner.edge_records[canonical.0 as usize].source_arrow = value;
            prepared.component.edge_records[local].source_arrow = value;
        }
        if let Some(value) = request.get("target_arrow").and_then(Value::as_bool) {
            owner.edge_records[canonical.0 as usize].target_arrow = value;
            prepared.component.edge_records[local].target_arrow = value;
        }
    }
    owner
        .restore_owner_graph_routing_costs(OwnerGraphId(0), (3.0, 5.0, 7.0))
        .unwrap();
    owner
        .set_owner_graph_cell_size(OwnerGraphId(0), 10.0)
        .unwrap();
    let temporary = prepared
        .fixture
        .prepared_owner
        .as_ref()
        .expect("actual prepared view")
        .view;
    owner
        .restore_owner_graph_routing_costs(temporary, (11.0, 13.0, 17.0))
        .unwrap();
    owner.set_owner_graph_cell_size(temporary, 10.0).unwrap();
    owner
        .set_owner_graph_cell_size(prepared.receiver, cell)
        .unwrap();
    owner
        .restore_owner_graph_routing_costs(prepared.receiver, costs)
        .unwrap();
    if request
        .get("empty_graph_edges")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        owner
            .set_owner_view_edge_membership(prepared.receiver, Vec::new())
            .unwrap();
    }
    let initial = owner_frame_cost_views(&prepared);
    let mut steps = Vec::new();
    let mut journal = OwnerGraphOwnershipJournal::default();
    let mut run = |prepared: &OwnerFrameProbeFixture, name: &str| -> Result<(), String> {
        steps.push(owner_frame_cost_step(
            prepared,
            name,
            observer,
            direction_penalty,
            capture_direction_inputs,
        )?);
        Ok(())
    };
    let measured = (|| -> Result<Value, String> {
        run(&prepared, "before-move-foreign-owner")?;
        prepared
            .fixture
            .original
            .graph
            .translate_node_with_children(prepared.foreign, Point { x: -260.0, y: 0.0 });
        run(&prepared, "after-move-foreign-owner")?;
        prepared
            .fixture
            .original
            .graph
            .set_owner_graph_reference(prepared.receiver, &[prepared.foreign], true, &mut journal)
            .map_err(|error| error.to_string())?;
        run(&prepared, "after-actual-Graph-reassignment")?;
        let before_cost_operation = owner_frame_cost_views(&prepared);
        let owner = &prepared.fixture.original.graph;
        match operation {
            "RestoreTurn31" | "RestoreTurnZero" => {
                let mut next = owner
                    .owner_graph_routing_costs(prepared.receiver)
                    .map_err(|error| error.to_string())?;
                next.1 = if operation == "RestoreTurn31" {
                    31.0
                } else {
                    0.0
                };
                owner
                    .restore_owner_graph_routing_costs(prepared.receiver, next)
                    .map_err(|error| error.to_string())?;
            }
            "ResetTurnCost" => owner
                .reset_owner_graph_turn_cost(prepared.receiver)
                .map_err(|error| error.to_string())?,
            "HalveTurnCost" => owner
                .halve_owner_graph_turn_cost(prepared.receiver)
                .map_err(|error| error.to_string())?,
            "Keep" => {}
            _ => unreachable!(),
        }
        let after_cost_operation = owner_frame_cost_views(&prepared);
        run(&prepared, "after-actual-selected-cost-operation")?;
        run(&prepared, "same-input-repeat-score")?;
        let before_getters = owner_frame_cost_views(&prepared);
        let owner = &prepared.fixture.original.graph;
        let getters = json!({
            "first_turn": bits(owner.owner_graph_turn_cost(prepared.receiver).map_err(|error| error.to_string())?),
            "second_turn": bits(owner.owner_graph_turn_cost(prepared.receiver).map_err(|error| error.to_string())?),
            "crossing": bits(owner.owner_graph_crossing_cost(prepared.receiver).map_err(|error| error.to_string())?),
            "noncenter": bits(owner.owner_graph_non_center_port_cost(prepared.receiver).map_err(|error| error.to_string())?),
        });
        Ok(
            json!({"initial": initial, "before_cost_operation": before_cost_operation,
            "after_cost_operation": after_cost_operation, "before_getters": before_getters,
            "actual_getter_results": getters, "after_getters": owner_frame_cost_views(&prepared)}),
        )
    })();
    prepared
        .fixture
        .original
        .graph
        .restore_owner_graph_ownership(&journal);
    let owner_graph_restored = owner_frame_cost_views(&prepared);
    let scope = prepared
        .fixture
        .prepared_owner
        .as_ref()
        .expect("actual prepared scope");
    let component_remaining = prepared
        .fixture
        .original
        .graph
        .restore_owner_edge_abductions(&scope.abductions, &prepared.canonical_nodes);
    let cleanup = component_remaining.and_then(|remaining| {
        let owner = &mut prepared.fixture.original.graph;
        let roots = owner.node_order.clone();
        let final_remaining = owner.restore_owner_edge_abductions(&remaining, &roots)?;
        owner.restore_owner_graph_ownership(&prepared.component_ownership);
        owner.restore_owner_graph_ownership(&scope.ownership_journal);
        Ok(
            json!({"executed": true, "component_remaining_count": remaining.records.len(),
            "owner_remaining_count": final_remaining.records.len()}),
        )
    });
    match (measured, cleanup) {
        (Ok(mut result), Ok(mut cleanup)) => {
            cleanup["views"] = owner_frame_cost_views(&prepared);
            result["steps"] = json!(steps);
            result["actual_cost_operation"] = json!(operation);
            result["owner_graph_restored"] = owner_graph_restored;
            result["cleanup"] = cleanup;
            result["end_to_end_d2_reachability_qualified"] = json!(false);
            json!({"supported": true, "case": request.get("case"), "observer": observer, "result": result})
        }
        (Err(error), _) => json!({"supported": false, "score_error": error, "steps": steps}),
        (_, Err(error)) => {
            json!({"supported": false, "cleanup_error": error.to_string(), "steps": steps})
        }
    }
}

fn diagnostic_owner_frame_flow_probe(request: &Value) -> Value {
    let observer = match request.get("observer") {
        Some(Value::Bool(value)) => *value,
        None => false,
        _ => return json!({"supported": false, "request_error": "observer must be boolean"}),
    };
    let mode = request
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("absent");
    if !matches!(
        mode,
        "absent" | "present-nil" | "allocated-empty" | "nonempty"
    ) {
        return json!({"supported": false, "request_error": "unknown actual child-header mode"});
    }
    let mut input = Graph::default();
    let size = Size {
        width: 10.0,
        height: 10.0,
    };
    let authored = [
        ("a", Point { x: -5.0, y: -165.0 }),
        ("n", Point { x: -5.0, y: -5.0 }),
        ("b", Point { x: 155.0, y: -5.0 }),
    ];
    let roots = authored
        .iter()
        .enumerate()
        .map(|(index, (name, position))| {
            let id = input.add_node(Node::new(*name, size));
            input.tala_id_overrides[id.0 as usize] = Some(index as i64 + 1);
            input.set_initial_position(id, Some(*position));
            id
        })
        .collect::<Vec<_>>();
    input.set_container_flag(roots[1], true);
    let mut child = None;
    if mode == "nonempty" {
        let mut node = Node::new("child", size);
        node.parent = Some(roots[1]);
        let id = input.add_node(node);
        input.tala_id_overrides[id.0 as usize] = Some(4);
        input.set_initial_position(id, Some(Point { x: 500.0, y: 500.0 }));
        child = Some(id);
    }
    let edges = [
        input.add_edge(Edge {
            source: roots[0],
            target: roots[1],
        }),
        input.add_edge(Edge {
            source: roots[1],
            target: roots[2],
        }),
    ];
    for (index, &edge) in edges.iter().enumerate() {
        input.tala_edge_ids[edge.0 as usize] = Some(2000 + index as i64);
    }
    let mut owner = ArenaGraph::from_input(&input);
    for (index, &node) in roots.iter().enumerate() {
        owner.set_position(node, authored[index].1);
    }
    if let Some(child) = child {
        owner.set_position(child, Point { x: 500.0, y: 500.0 });
    }
    for &edge in &edges {
        owner.edge_records[edge.0 as usize].target_arrow = true;
    }
    owner.initialize_shared_topology();
    match mode {
        "absent" => {
            owner.containers.remove(&Some(roots[1]));
        }
        "present-nil" => owner
            .containers
            .replace_header(Some(roots[1]), SharedNodeSlice::nil()),
        "allocated-empty" => owner
            .containers
            .replace_header(Some(roots[1]), SharedNodeSlice::with_len_cap(0, 4).unwrap()),
        "nonempty" => {}
        _ => unreachable!(),
    }
    let temporary = owner
        .create_owner_graph_view(roots.clone(), edges.to_vec())
        .unwrap();
    let selected = owner
        .create_owner_graph_view(roots.clone(), edges.to_vec())
        .unwrap();
    let names = BTreeMap::from([
        (OwnerGraphId(0), "original".to_owned()),
        (temporary, "temporary".to_owned()),
        (selected, "selected".to_owned()),
    ]);
    for (view, turn) in [(OwnerGraphId(0), 5.0), (temporary, 13.0), (selected, 23.0)] {
        owner.set_owner_graph_cell_size(view, 10.0).unwrap();
        owner
            .restore_owner_graph_routing_costs(view, (3.0, turn, 7.0))
            .unwrap();
    }
    let mut ownership = OwnerGraphOwnershipJournal::default();
    owner
        .set_owner_graph_reference(selected, &roots, true, &mut ownership)
        .unwrap();
    let (mut workspace, mapping, mapped_edges) =
        owner.induced_placement_subgraph_with_edge_mapping(&roots);
    owner
        .set_owner_view_node_mapping(selected, mapping.clone())
        .unwrap();
    let records = owner
        .register_owner_edge_abductions(Vec::new(), false)
        .unwrap();
    let local_center = NodeId(mapping.iter().position(|node| *node == roots[1]).unwrap() as u32);
    let inputs = |owner: &ArenaGraph| {
        json!({
            "scoring_node": owner_frame_probe_node(owner, Some(roots[1])),
            "historical_container_flag": owner.nodes[roots[1].0 as usize].is_container,
            "actual_center_children": owner_frame_probe_map_header(owner, selected, Some(roots[1])),
            "actual_root": owner_frame_probe_map_header(owner, selected, None),
            "views": owner_frame_cost_views_from(owner, &names),
            "actual_incident_edges": owner.nodes[roots[1].0 as usize].edges.read().iter().map(|edge| {
                let actual = &owner.edge_records[edge.0 as usize];
                json!({"id": actual.tala_id, "edge_pointer": owner_frame_probe_address(Some(actual)),
                    "from": owner_frame_probe_node(owner, Some(actual.from)), "to": owner_frame_probe_node(owner, Some(actual.to)),
                    "source_arrow_present": actual.source_arrow, "target_arrow_present": actual.target_arrow,
                    "invisible": actual.is_invisible()})
            }).collect::<Vec<_>>(),
        })
    };
    let mut steps = Vec::new();
    for (index, name) in [
        "orthogonal-directed-historical-container-Turn23",
        "same-geometry-undirected-Turn23",
        "same-geometry-directed-Turn31",
        "same-actual-child-header-historical-flag-cleared-Turn31",
    ]
    .iter()
    .enumerate()
    {
        if index == 1 || index == 2 {
            for (&canonical, local) in mapped_edges.iter().zip(workspace.edge_records.iter_mut()) {
                owner.edge_records[canonical.0 as usize].target_arrow = index == 2;
                local.target_arrow = index == 2;
            }
        }
        if index == 2 {
            owner
                .restore_owner_graph_routing_costs(selected, (3.0, 31.0, 7.0))
                .unwrap();
        }
        if index == 3 {
            owner.nodes[roots[1].0 as usize].is_container = false;
        }
        let before = inputs(&owner);
        let frame = OwnerScoreFrame::new(
            &owner,
            owner.topology_arena_id().unwrap(),
            selected,
            &mapping,
        )
        .and_then(|frame| frame.with_edge_mapping(&mapped_edges))
        .and_then(|frame| frame.with_abductions(&records))
        .unwrap();
        let context = OwnerFrameProbeContext {
            owner: &owner,
            receiver: selected,
            canonical_nodes: &mapping,
            owner_names: &names,
            foreign: child.unwrap_or(roots[1]),
            observer,
            set_cursor: Cell::new(None),
            current: RefCell::new(Value::Null),
            events: RefCell::new(Vec::new()),
            sets: RefCell::new(Vec::new()),
            primitives: RefCell::new(Vec::new()),
            observation_error: RefCell::new(None),
        };
        let score = match workspace.sized_edge_length_with_owner_frame_checked(
            local_center,
            false,
            false,
            false,
            false,
            true,
            &frame,
            crate::internal::scoring_context::ScoreContext::new(&context),
        ) {
            Ok(value) => value,
            Err(error) => {
                return json!({"supported": false, "score_error": error.to_string(), "steps": steps});
            }
        };
        steps.push(
            json!({"name": name, "score": bits(score), "error": "", "inputs": before,
            "inputs_after": inputs(&owner), "context_observations": context.events.borrow().clone(),
            "actual_obstruction_sets": context.sets.borrow().clone(),
            "actual_frame_callbacks": context.primitives.borrow().clone(),
            "observation_error": context.observation_error.borrow().clone()}),
        );
    }
    owner.restore_owner_graph_ownership(&ownership);
    json!({"supported": true, "case": request.get("case"), "observer": observer,
        "result": {"mode": mode, "steps": steps, "end_to_end_d2_reachability_qualified": false}})
}

fn diagnostic_owner_frame_live_geometry_probe(request: &Value) -> Value {
    let case = request
        .get("case")
        .and_then(Value::as_str)
        .unwrap_or("api-foreign-live-geometry-owner-rescore");
    let observer = match request.get("observer") {
        Some(Value::Bool(value)) => *value,
        None => false,
        _ => return json!({"supported": false, "request_error": "observer must be a boolean"}),
    };
    if request
        .get("count")
        .is_some_and(|value| value.as_u64() != Some(1))
        || request
            .get("reverse")
            .is_some_and(|value| value.as_bool() != Some(false))
    {
        return json!({"supported": false, "request_error": "this direct source API control uses count=1 and reverse=false"});
    }
    let mut prepared = match prepared_owner_frame_probe_fixture() {
        Ok(prepared) => prepared,
        Err(error) => return json!({"supported": false, "preparation_error": error}),
    };
    let captured = match prepared.fixture.original.graph.containers.get_header(&None) {
        Some(header) => header,
        None => {
            return json!({"supported": false, "preparation_error": "actual owner root header absent"});
        }
    };
    let before_owner =
        owner_frame_probe_node(&prepared.fixture.original.graph, Some(prepared.foreign));
    let mut ownership = OwnerGraphOwnershipJournal::default();
    let mut steps = Vec::new();
    let mut primitives = Vec::new();
    let mut observation_errors = Vec::new();
    for (index, name) in [
        "before",
        "after-actual-MoveWithChildren",
        "after-actual-SetGraphReference",
    ]
    .iter()
    .enumerate()
    {
        // Each previous frame and context were dropped at the end of its
        // block. These operations require exclusive access to the owner.
        if index == 1 {
            prepared
                .fixture
                .original
                .graph
                .translate_node_with_children(prepared.foreign, Point { x: -260.0, y: 0.0 });
        } else if index == 2 {
            if let Err(error) = prepared.fixture.original.graph.set_owner_graph_reference(
                prepared.receiver,
                &[prepared.foreign],
                true,
                &mut ownership,
            ) {
                return json!({"supported": false, "ownership_error": error.to_string()});
            }
        }
        let (score, error, events, sets, primitive, observation_error) = {
            let owner = &prepared.fixture.original.graph;
            let frame = match OwnerScoreFrame::new(
                owner,
                owner.topology_arena_id().expect("actual owner arena"),
                prepared.receiver,
                &prepared.canonical_nodes,
            )
            .and_then(|frame| {
                frame.with_abductions(
                    &prepared
                        .fixture
                        .prepared_owner
                        .as_ref()
                        .expect("actual prepared owner records")
                        .abductions,
                )
            }) {
                Ok(frame) => frame,
                Err(error) => return json!({"supported": false, "frame_error": error.to_string()}),
            };
            let context = OwnerFrameProbeContext {
                owner,
                receiver: prepared.receiver,
                canonical_nodes: &prepared.canonical_nodes,
                owner_names: &prepared.owner_names,
                foreign: prepared.foreign,
                observer,
                set_cursor: Cell::new(None),
                current: RefCell::new(Value::Null),
                events: RefCell::new(Vec::new()),
                sets: RefCell::new(Vec::new()),
                primitives: RefCell::new(Vec::new()),
                observation_error: RefCell::new(None),
            };
            let result = prepared
                .component
                .sized_edge_length_with_owner_frame_checked(
                    prepared.center,
                    false,
                    false,
                    false,
                    false,
                    true,
                    &frame,
                    crate::internal::scoring_context::ScoreContext::new(&context),
                );
            let (score, error) = match result {
                Ok(score) => (score, String::new()),
                Err(error) => (0.0, error.to_string()),
            };

            (
                score,
                error,
                context.events.borrow().clone(),
                context.sets.borrow().clone(),
                context.primitives.borrow().clone(),
                context.observation_error.borrow().clone(),
            )
        };
        let owner = &prepared.fixture.original.graph;
        steps.push(json!({
            "name": name, "score": bits(score), "error": error,
            "foreign": owner_frame_probe_node(owner, Some(prepared.foreign)),
            "captured": owner_frame_probe_header(owner, &captured),
            "current_map": owner_frame_probe_map_header(owner, prepared.receiver, None),
            "actual_obstruction_sets_present": !sets.is_empty(), "actual_obstruction_sets": sets,
            "context_observations": events,
        }));
        primitives.push(json!({"step": name, "actual_frame_callbacks": primitive,
            "captured_header_offset": captured.offset(), "captured_backing_pointer": format!("0x{:016x}", captured.backing_address().unwrap_or(0)),
            "receiver_member_node_ids": owner.owner_graph_view(prepared.receiver).expect("live receiver").nodes.iter_slots().map(|node| node.map(|node| owner.nodes[node.0 as usize].tala_id)).collect::<Vec<_>>(),
        }));
        if let Some(error) = observation_error {
            observation_errors.push(error);
        }
    }
    prepared
        .fixture
        .original
        .graph
        .restore_owner_graph_ownership(&ownership);
    let after_owner_restore =
        owner_frame_probe_node(&prepared.fixture.original.graph, Some(prepared.foreign));
    let relations = vec![
        owner_frame_probe_relation(
            "foreign node pointer survives geometry and owner mutations",
            "equal",
            steps[0]["foreign"]["node_pointer"].clone(),
            steps[2]["foreign"]["node_pointer"].clone(),
        ),
        owner_frame_probe_relation(
            "captured backing survives geometry and owner mutations",
            "equal",
            steps[0]["captured"]["data_pointer"].clone(),
            steps[2]["captured"]["data_pointer"].clone(),
        ),
        owner_frame_probe_relation(
            "actual move changes live foreign geometry",
            "not_equal",
            steps[0]["foreign"]["top_left"].clone(),
            steps[1]["foreign"]["top_left"].clone(),
        ),
        owner_frame_probe_relation(
            "actual graph reassignment changes foreign Graph pointer",
            "not_equal",
            steps[1]["foreign"]["graph_pointer"].clone(),
            steps[2]["foreign"]["graph_pointer"].clone(),
        ),
        owner_frame_probe_relation(
            "actual ownership snapshot restores Graph pointer",
            "equal",
            before_owner["graph_pointer"].clone(),
            after_owner_restore["graph_pointer"].clone(),
        ),
    ];
    let result = json!({
        "control_kind": "actual-native-owner-frame-API-movement-owner-reassignment-and-rescoring",
        "end_to_end_d2_reachability_qualified": false,
        "preconditions": "Valid source Graph API C/c→L topology; explicit operation sequence moves foreign D then reassigns its owner without adding D to selected.Nodes",
        "operations": ["OwnerGraphOwnershipJournal::default", "sized_edge_length_with_owner_frame_checked", "translate_node_with_children", "sized_edge_length_with_owner_frame_checked", "set_owner_graph_reference", "sized_edge_length_with_owner_frame_checked", "restore_owner_graph_ownership"],
        "steps": steps, "after_owner_restore": after_owner_restore, "measured_relations": relations,
        "native_owner_frame_observations": primitives,
        "native_ownership_journal": {"entry_foreign": before_owner, "changed_node_count": ownership.len(), "owner_restore_executed": true},
        "native_ownership_snapshot_qualification": "Native journal records actual pre-assignment owners; Go SnapshotNodeGraphOwnership whole-graph traversal and its polls are unqualified here",
        "instrumented": true,
    });
    // Actual cleanup runs after the observed source API sequence. It is not
    // substituted for the measured Graph journal restoration above.
    let cleanup = if let Some(owner_scope) = &prepared.fixture.prepared_owner {
        let remaining = prepared
            .fixture
            .original
            .graph
            .restore_owner_edge_abductions(&owner_scope.abductions, &prepared.canonical_nodes);
        let remaining = match remaining {
            Ok(remaining) => remaining,
            Err(error) => {
                return json!({"supported": false, "cleanup_error": error.to_string(), "result": result});
            }
        };
        let roots = prepared.fixture.original.graph.node_order.clone();
        let remaining = match prepared
            .fixture
            .original
            .graph
            .restore_owner_edge_abductions(&remaining, &roots)
        {
            Ok(remaining) => remaining,
            Err(error) => {
                return json!({"supported": false, "cleanup_error": error.to_string(), "result": result});
            }
        };
        prepared
            .fixture
            .original
            .graph
            .restore_owner_graph_ownership(&prepared.component_ownership);
        prepared
            .fixture
            .original
            .graph
            .restore_owner_graph_ownership(&owner_scope.ownership_journal);
        json!({"executed": true, "remaining_records": remaining.records.len(), "remaining_present": remaining.present,
            "state": foreign_slice_state(&prepared.fixture.original.graph, &prepared.component)})
    } else {
        json!({"executed": false})
    };
    json!({"probe": "edge-length-owner-frame-live-geometry", "case": case, "observer": observer,
        "supported": observation_errors.is_empty(), "scorer_executed": true, "result": result,
        "native_preparation_cleanup": cleanup, "observation_errors": observation_errors,
        "full_operation_trace_parity": false})
}

fn diagnostic_foreign_slice_probe(
    request: &Value,
    live_current: bool,
    obstruction_probe: bool,
) -> Value {
    let count = request.get("count").and_then(Value::as_u64).unwrap_or(1) as usize;
    let reverse = request
        .get("reverse")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let observer = request
        .get("observer")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let cancel_line = request
        .get("cancel_source_line")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cancel_iteration = request
        .get("cancel_iteration")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let cancel_set = request
        .get("cancel_set")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let nil_foreign_position = request
        .get("nil_foreign_position")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !matches!(count, 1 | 65)
        || !matches!(cancel_iteration, 0 | 64)
        || !matches!(cancel_set, 0 | 1)
    {
        return json!({"supported": false, "request_error": "count must be 1 or 65; cancellation iteration must be 0 or 64"});
    }
    let cancel_site = match (obstruction_probe, cancel_line) {
        (_, 0) => None,
        (false, 136) => Some("symmetry-container-abduction"),
        (false, 290) => Some(SYMMETRY_RECOVERY_SITE),
        (true, 903) => Some("node-obstruction-set"),
        (true, 907) => Some("node-obstruction"),
        _ => {
            return json!({"supported": false, "request_error": "unsupported source cancellation line"});
        }
    };
    let mut authored = vec![
        (1_i64, Point { x: 80.0, y: 80.0 }, 60.0, None),
        (2_i64, Point { x: 20.0, y: 20.0 }, 10.0, Some(1_i64)),
    ];
    if obstruction_probe {
        authored.push((3_i64, Point { x: -80.0, y: 100.0 }, 10.0, None));
    }
    for index in 0..count {
        let owner = 1000 + 2 * index as i64;
        authored.push((
            owner,
            Point {
                x: 300.0 + 100.0 * index as f64,
                y: 80.0,
            },
            60.0,
            None,
        ));
        authored.push((owner + 1, Point { x: 20.0, y: 20.0 }, 10.0, Some(owner)));
        if live_current {
            authored.push((
                3000 + index as i64,
                Point {
                    x: 500.0 + 100.0 * index as f64,
                    y: 100.0,
                },
                10.0,
                None,
            ));
        }
    }
    if reverse {
        authored.reverse();
    }
    let ids = authored
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.0, NodeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut input = Graph::default();
    for &(tala, position, dimension, parent) in &authored {
        let mut node = Node::new(
            format!("foreign-slice-{tala}"),
            Size {
                width: dimension,
                height: dimension,
            },
        );
        node.parent = parent.map(|parent| ids[&parent]);
        let id = input.add_node(node);
        input.tala_id_overrides[id.0 as usize] = Some(tala);
        let foreign_owner = (1000..3000).contains(&tala) && tala % 2 == 0;
        input.set_initial_position(
            id,
            if obstruction_probe && nil_foreign_position && foreign_owner {
                None
            } else {
                Some(position)
            },
        );
        input.set_container_flag(
            id,
            parent.is_none() && (!live_current || tala < 3000) && (!obstruction_probe || tala != 3),
        );
    }
    let mut edge_indices = (0..count).collect::<Vec<_>>();
    if reverse {
        edge_indices.reverse();
    }
    for index in edge_indices {
        if obstruction_probe {
            continue;
        }
        let owner = ids[&(1000 + 2 * index as i64)];
        let child = ids[&(1001 + 2 * index as i64)];
        let other = if live_current {
            ids[&(3000 + index as i64)]
        } else {
            owner
        };
        let edge = input.add_edge(if reverse {
            Edge {
                source: if live_current { other } else { child },
                target: if live_current { child } else { other },
            }
        } else {
            Edge {
                source: if live_current { child } else { other },
                target: if live_current { other } else { child },
            }
        });
        input.tala_edge_ids[edge.0 as usize] = Some(2000 + index as i64);
    }
    if obstruction_probe {
        let edge = input.add_edge(if reverse {
            Edge {
                source: ids[&3],
                target: ids[&2],
            }
        } else {
            Edge {
                source: ids[&2],
                target: ids[&3],
            }
        });
        input.tala_edge_ids[edge.0 as usize] = Some(2000);
    }
    let mut fixture = match finish_prepared_symmetry_fixture_preserving_missing_positions(
        input,
        ids[&1],
        obstruction_probe,
    ) {
        Ok(fixture) => fixture,
        Err(error) => return json!({"supported": false, "preparation_error": error}),
    };
    let components = fixture.temporary.graph.split_placement_subgraphs();
    // Source SetGraphReference assigns every retained descendant as well as
    // each component root. Execute those owner operations in canonical IDs.
    let mut component_ownership = OwnerGraphOwnershipJournal::default();
    let mut owner_names = BTreeMap::from([(OwnerGraphId(0), "original".to_owned())]);
    if let Some(prepared) = &fixture.prepared_owner {
        owner_names.insert(prepared.view, "temporary".to_owned());
    }
    for (component_index, roots) in components.iter().enumerate() {
        let selected = roots.contains(&fixture.center);
        let roots = roots
            .iter()
            .map(|node| fixture.temporary_source_nodes[node])
            .collect::<Vec<_>>();
        let members = roots.iter().copied().collect::<BTreeSet<_>>();
        let edges = fixture
            .original
            .graph
            .edge_order
            .read()
            .iter()
            .copied()
            .filter(|edge| {
                let edge = &fixture.original.graph.edge_records[edge.0 as usize];
                members.contains(&edge.from) && members.contains(&edge.to)
            })
            .collect();
        let view = match fixture
            .original
            .graph
            .create_owner_graph_view(roots.clone(), edges)
        {
            Ok(view) => view,
            Err(error) => {
                return json!({"supported": false, "preparation_error": error.to_string()});
            }
        };
        if let Err(error) = fixture
            .original
            .graph
            .set_owner_view_node_mapping(view, roots.clone())
        {
            return json!({"supported": false, "preparation_error": error.to_string()});
        }
        owner_names.insert(view, format!("component-{component_index}"));
        if let Err(error) = fixture.original.graph.set_owner_graph_reference(
            view,
            &roots,
            false,
            &mut component_ownership,
        ) {
            return json!({"supported": false, "preparation_error": error.to_string()});
        }
        if selected && obstruction_probe {
            // This source boundary explicitly invokes SetGraphReference on
            // the selected roots after tracked splitting. Its descendant
            // assignments are separate from the split's root-only journal.
            let mut descendant_ownership = OwnerGraphOwnershipJournal::default();
            if let Err(error) = fixture.original.graph.set_owner_graph_reference(
                view,
                &roots,
                true,
                &mut descendant_ownership,
            ) {
                return json!({"supported": false, "preparation_error": error.to_string()});
            }
        }
    }
    let members = match components
        .iter()
        .find(|members| members.contains(&fixture.center))
    {
        Some(members) => members,
        None => {
            return json!({"supported": false, "preparation_error": "actual component splitter omitted C"});
        }
    };
    let (mut component, owner_nodes) = fixture.temporary.graph.induced_placement_subgraph(members);
    let center = NodeId(
        owner_nodes
            .iter()
            .position(|node| *node == fixture.center)
            .expect("selected component contains C") as u32,
    );
    component.cell_size = 10.0;
    let canonical_component_roots = owner_nodes
        .iter()
        .map(|node| fixture.temporary_source_nodes[node])
        .collect::<Vec<_>>();
    let entry = foreign_slice_state(&fixture.original.graph, &component);
    let context = ForeignSliceContext {
        original: &fixture.original.graph,
        owner_names: &owner_names,
        canonical_members: &canonical_component_roots,
        observer,
        cancel_site,
        cancel_iteration,
        obstruction_probe,
        cancel_set,
        obstruction_set: Cell::new(None),
        actual_obstruction_sets_present: Cell::new(false),
        actual_obstruction_sets: RefCell::new(json!([])),
        current: RefCell::new(Value::Null),
        events: RefCell::new(Vec::new()),
        observation_error: RefCell::new(None),
    };
    let result = if obstruction_probe {
        component.sized_edge_length_with_cache_mode_materialized_checked(
            center,
            false,
            None,
            false,
            false,
            false,
            true,
            crate::internal::scoring_context::ScoreContext::new(&context),
        )
    } else {
        component.sized_symmetry_checked(
            center,
            true,
            crate::internal::scoring_context::ScoreContext::new(&context),
        )
    };
    let (score, error) = match result {
        Ok(score) => (score, String::new()),
        Err(error) => (0.0, error.to_string()),
    };
    let after = foreign_slice_state(&fixture.original.graph, &component);
    let observations = context.events.borrow().clone();
    let observation_error = context.observation_error.borrow().clone();
    let actual_obstruction_sets_present = context.actual_obstruction_sets_present.get();
    let actual_obstruction_sets = context.actual_obstruction_sets.borrow().clone();
    drop(context);
    let restoration = if let Some(prepared) = &fixture.prepared_owner {
        let component_remaining = match fixture
            .original
            .graph
            .restore_owner_edge_abductions(&prepared.abductions, &canonical_component_roots)
        {
            Ok(remaining) => remaining,
            Err(error) => {
                return json!({"supported": false, "restoration_error": error.to_string()});
            }
        };
        let component_state = foreign_slice_state(&fixture.original.graph, &component);
        let roots = fixture.original.graph.node_order.clone();
        let owner_remaining = match fixture
            .original
            .graph
            .restore_owner_edge_abductions(&component_remaining, &roots)
        {
            Ok(remaining) => remaining,
            Err(error) => {
                return json!({"supported": false, "restoration_error": error.to_string()});
            }
        };
        let owner_state = foreign_slice_state(&fixture.original.graph, &component);
        fixture
            .original
            .graph
            .restore_owner_graph_ownership(&component_ownership);
        let ownership_restored_state = foreign_slice_state(&fixture.original.graph, &component);
        fixture
            .original
            .graph
            .restore_owner_graph_ownership(&prepared.ownership_journal);
        json!({
            "executed": true,
            "component_remaining_count": component_remaining.records.len(),
            "component_remaining_present": component_remaining.present,
            "owner_remaining_count": owner_remaining.records.len(),
            "owner_remaining_present": owner_remaining.present,
            "component_state": component_state,
            "owner_state": owner_state,
            "ownership_restored_state": ownership_restored_state,
            "native_preparation_cleanup_state": foreign_slice_state(&fixture.original.graph, &component),
        })
    } else {
        json!({"executed": false, "reason": "aggregate owner allocation remains projected"})
    };
    json!({
        "probe": if obstruction_probe { "edge-length-foreign-obstructions" } else if live_current { "symmetry-foreign-live-current" } else { "symmetry-foreign-slice" }, "case": request.get("case"),
        "supported": observation_error.is_none(), "scorer_executed": true,
        "observer": observer, "count": count, "reverse": reverse,
        "cancel_source_line": cancel_line, "cancel_iteration": cancel_iteration,
        "cancel_set": cancel_set, "nil_foreign_position": nil_foreign_position,
        "result": { "score": bits(score), "error": error, "entry": entry, "after": after,
            "context_observations": observations,
            "component_node_order": owner_nodes.iter().map(|node| fixture.temporary.graph.nodes[node.0 as usize].tala_id).collect::<Vec<_>>(),
            "projection": fixture.projection_observations,
            "whole_record_count_before_induction": fixture.temporary.graph.sized_edge_abductions.len(),
            "component_record_count": component.sized_edge_abductions.len(),
            "native_original_before_projection": fixture.original_before_projection,
            "native_restoration": restoration,
            "actual_obstruction_sets_present": actual_obstruction_sets_present,
            "actual_obstruction_sets": actual_obstruction_sets,
        },
        "observation_error": observation_error,
        "evidence_limits": ["Production root projection, semantic hydration, component splitting, induction and direct checked scorer; authored positions only.",
            "Canonical owning-edge mutations and owner restoration execute; compact component arrays and aggregate allocation remain separate parity gates.",
            "All actual named polls and unchanged repeated states retained; no source argument, result or poll installed."],
        "full_operation_trace_parity": false,
    })
}

/// Runs a prepared graph through the actual guarded NormalizeGaps stage.
///
/// The request accepts `case`, optional signed `work_limit`, and `observer`.
/// All six original source controls use actual context-poll observations.
/// The W−1 default comes from a separately executed successful control.
/// Returned floats are IEEE-754 bit strings; accepted cancellation is derived
/// from actual displacement at an outer geometry-clone poll.
pub fn diagnostic_gap_stage_probe(request: Value) -> Value {
    if !request.is_object() {
        return json!({"supported": false, "request_error": "request must be a JSON object"});
    }
    if let Some(probe) = request.get("probe") {
        match probe.as_str() {
            Some("symmetry-foreign-slice") => {
                return diagnostic_foreign_slice_probe(&request, false, false);
            }
            Some("symmetry-foreign-live-current") => {
                return diagnostic_foreign_slice_probe(&request, true, false);
            }
            Some("edge-length-foreign-obstructions") => {
                return diagnostic_foreign_slice_probe(&request, false, true);
            }
            Some("edge-length-owner-frame-live-geometry") => {
                return diagnostic_owner_frame_live_geometry_probe(&request);
            }
            Some("owner-initialize-sync") => {
                return diagnostic_owner_initialize_sync_probe(&request);
            }
            Some("owner-pass-mutation-snapshot") => {
                return owner_pass_probe::diagnostic_owner_pass_mutation_probe(&request);
            }
            Some("owner-member-mutation-snapshot") => {
                return owner_pass_probe::diagnostic_owner_member_probe(&request);
            }
            Some("owner-sizeless-constructor") => {
                return owner_pass_probe::diagnostic_owner_constructor_probe(&request);
            }
            Some("owner-candidate-movement") => {
                return diagnostic_owner_candidate_movement_probe(&request);
            }
            Some("owner-sizeless-candidate-search") => {
                return diagnostic_owner_sizeless_candidate_probe(&request);
            }
            Some("edge-length-owner-frame-cost-state") => {
                return diagnostic_owner_frame_cost_probe(&request);
            }
            Some("edge-length-owner-frame-flow-state") => {
                return diagnostic_owner_frame_flow_probe(&request);
            }
            Some("symmetry-abduction") => return diagnostic_symmetry_abduction_probe(&request),
            Some("symmetry-unpositioned-original") => {
                return diagnostic_symmetry_unpositioned_probe(&request);
            }
            Some("symmetry-positioned-child-nil-owner") => {
                return diagnostic_symmetry_positioned_child_nil_owner_probe(&request);
            }
            Some("symmetry-null-current") => {
                return diagnostic_symmetry_null_current_probe(&request);
            }
            _ => return json!({"supported": false, "request_error": "unknown prepared probe"}),
        }
    }
    let case = match request.get("case") {
        Some(Value::String(case)) => case.as_str(),
        None => "scored-success",
        _ => return json!({"supported": false, "request_error": "case must be a string"}),
    };
    let scorer_case = matches!(
        case,
        "stage-success"
            | "stage-node-entry-cancel"
            | "stage-final-before-store-cancel"
            | "scorer-uncached-success"
            | "scorer-cached-hit-node-entry-cancel"
            | "scorer-cached-hit-final-before-store-cancel"
    );
    if !scorer_case
        && !matches!(
            case,
            "scored-success"
                | "work-limit-after-accepted"
                | "cancel-after-accepted"
                | "no-score-before-score-cancel"
                | "pre-canceled"
                | "empty-no-score-success"
        )
    {
        return json!({"case": case, "supported": false, "request_error": "unknown prepared source profile"});
    }
    let observer = match request.get("observer") {
        Some(Value::Bool(value)) => *value,
        None => false,
        _ => {
            return json!({"case": case, "supported": false, "request_error": "observer must be a boolean"});
        }
    };
    let requested_work_limit = match request.get("work_limit") {
        Some(value) => match value.as_i64() {
            Some(value) => Some(value),
            None => {
                return json!({"case": case, "supported": false, "request_error": "work_limit must be an i64"});
            }
        },
        None => None,
    };
    if scorer_case {
        return scorer_profile(case, observer, requested_work_limit);
    }
    let reverse = case == "no-score-before-score-cancel";
    let mut pipeline = prepared_pipeline(case == "empty-no-score-success", reverse);
    let identity = entry_identity(&pipeline.graph);
    let entry = match observe(&pipeline.graph, &identity, observer) {
        Ok(value) => value,
        Err(error) => return json!({"case": case, "supported": false, "observation_error": error}),
    };
    let context = ObservingContext::new(
        &pipeline.graph,
        &identity,
        entry.clone(),
        reverse,
        observer,
        matches!(
            case,
            "cancel-after-accepted" | "no-score-before-score-cancel"
        ),
    );
    let mut work_limit = requested_work_limit.unwrap_or(gap_guard::MAX_TRANSACTION_WORK_UNITS);
    let mut guard = match limits::WorkGuard::new(Some(&context), SOURCE_LOCATION, work_limit) {
        Ok(value) => value,
        Err(error) => {
            return json!({"case": case, "supported": true, "stage_executed": false, "request_error": error.to_string(), "entry": entry});
        }
    };
    // The source W−1 profile derives its limit from an actual successful stage.
    let mut success_work_control = 0;
    let mut success_control_error = String::new();
    if case == "work-limit-after-accepted" {
        let mut control = prepared_pipeline(false, false);
        let background = BackgroundContext;
        let mut control_guard = limits::WorkGuard::new(
            Some(&background),
            SOURCE_LOCATION,
            gap_guard::MAX_TRANSACTION_WORK_UNITS,
        )
        .expect("a background source work guard is valid");
        if let Err(error) =
            control.gap_normalization_stage_with_context(&background, &mut control_guard)
        {
            success_control_error = error.to_string();
        }
        success_work_control = control_guard.used();
        if !success_control_error.is_empty() {
            return json!({"case": case, "supported": false, "stage_executed": false, "control_error": success_control_error, "success_work_control": success_work_control});
        }
        if requested_work_limit.is_none() {
            let Some(limit) = success_work_control
                .checked_sub(1)
                .filter(|limit| *limit >= 0)
            else {
                return json!({"case": case, "supported": false, "stage_executed": false, "control_error": "successful control consumed no positive work", "success_work_control": success_work_control});
            };
            work_limit = limit;
            guard.set_limit(limit);
        }
    }
    let pre_canceled = PreCanceledContext;
    let operation_context: &dyn limits::Context = if case == "pre-canceled" {
        &pre_canceled
    } else {
        &context
    };
    let result = pipeline.gap_normalization_stage_with_context(operation_context, &mut guard);
    let error = match result {
        Ok(changed) => {
            pipeline.run_align_axis = changed;
            String::new()
        }
        Err(error) => {
            pipeline.run_align_axis = false;
            error.to_string()
        }
    };
    let after = match observe(&pipeline.graph, &identity, observer) {
        Ok(value) => value,
        Err(error) => return json!({"case": case, "supported": false, "observation_error": error}),
    };
    let observations = context.observations.borrow();
    if let Some(error) = &observations.observation_error {
        return json!({"case": case, "supported": false, "stage_executed": true, "observation_error": error});
    }
    json!({
        "case": case,
        "observer": observer,
        "supported": true,
        "stage_executed": true,
        "result": {
            "entry": entry,
            "after": after,
            "error": error,
            "align_axes_needed": pipeline.run_align_axis,
            "work_used": guard.used(),
            "work_limit": work_limit,
            "success_work_control": success_work_control,
            "success_control_error": success_control_error,
            "context_observations": observations.events,
            "accepted_observed": observations.accepted_observed,
            "cache_growth_observed": observations.cache_growth_observed,
            "accepted_with_cache_growth_observed": observations.accepted_with_cache_growth_observed,
        },
        "evidence_limits": {
            "scorer": "production non-cfg(test) library scorer",
            "snapshot_map_order": "sorted content serialization, not map traversal",
            "node_edge_identity": "stable arena IDs; Go allocation/point pointer identity is not measured",
            "context_observations": "actual error polls; state-change or outer accepted-boundary deduplication; active trial/live views",
            "accepted_boundary": "actual moving-node displacement at typed outer CloneGeometryContext; no work-count trigger",
        },
        "full_operation_trace_parity": false,
    })
}
