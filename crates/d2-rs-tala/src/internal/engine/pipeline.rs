// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Ordered layout-stage driver.
//!
//! Each method advances one stage and publishes only the state required by the
//! next stage. Snapshot tracing observes these boundaries without changing the
//! production sequence.

use super::*;
use d2_rs_core::stable_sort::StableSort;

struct SizelessLayoutContext;
impl limits::Context for SizelessLayoutContext {
    fn stable_sized_owner(&self) -> bool {
        true
    }

    fn done(&self) -> Option<bool> {
        crate::internal::cancellation_done()
    }
    fn error(&self) -> Option<limits::ContextError> {
        crate::internal::cancellation_error()
    }
}

#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_reset_owner_graph_views(graph: &ArenaGraph) {
    let owner_order = graph
        .node_order
        .iter()
        .map(|node| graph.nodes[node.0 as usize].tala_id)
        .collect::<Vec<_>>();
    let owner_container_children = graph
        .containers
        .iter()
        .map(|(parent, children)| {
            let parent_tala_id = parent
                .map(|node| graph.nodes[node.0 as usize].tala_id)
                .unwrap_or(0);
            let children = children
                .iter()
                .map(|node| graph.nodes[node.0 as usize].tala_id)
                .collect();
            (parent_tala_id, children)
        })
        .collect::<BTreeMap<_, _>>();
    if let Ok(mut owners) = graph.trace_owner_node_orders.lock() {
        owners.clear();
        owners.extend(
            graph
                .nodes
                .iter()
                .map(|node| (node.tala_id, owner_order.clone())),
        );
    }
    if let Ok(mut roots) = graph.trace_root_owner_node_orders.lock() {
        roots.clear();
        // `Graph.placeNodes` emits its root entry before it recursively lays
        // out child containers. Rust constructs those scopes postorder, so
        // preserve the root's live Graph.Nodes slice before that work changes
        // the shared owner view. Zero is the stable sentinel for Go's nil root.
        roots.insert(0, owner_order);
    }
    if let Ok(mut children) = graph.trace_root_owner_container_children.lock() {
        children.clear();
        children.extend(owner_container_children);
    }
    if let Ok(mut ranks) = graph.trace_container_placement_ranks.lock() {
        ranks.clear();
    }
}

#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_record_container_placement_order(graph: &ArenaGraph, containers: &[NodeId]) {
    if let Ok(mut ranks) = graph.trace_container_placement_ranks.lock() {
        ranks.clear();
        ranks.extend(
            containers
                .iter()
                .enumerate()
                .map(|(rank, node_id)| (graph.nodes[node_id.0 as usize].tala_id, rank)),
        );
    }
}

#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_record_children_graph_owner(owner: &ArenaGraph, child_order: &[i64]) {
    if let Ok(mut owners) = owner.trace_owner_node_orders.lock() {
        for tala_id in child_order {
            let is_cluster_member = owner.clusters.iter().any(|cluster| {
                cluster
                    .members
                    .iter()
                    .any(|member| owner.nodes[member.0 as usize].tala_id == *tala_id)
            });
            if is_cluster_member {
                continue;
            }
            // Graph.placeNodes creates a fresh childrenGraph and calls
            // AddNodeUnchecked once for each direct child, changing that
            // child's Graph pointer while retaining its existing descendants.
            owners.insert(*tala_id, child_order.to_vec());
        }
    }
}

#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_record_root_graph_owner(owner: &ArenaGraph, root_tala_id: i64) {
    let owner_order = owner
        .trace_owner_node_orders
        .lock()
        .ok()
        .and_then(|owners| owners.get(&root_tala_id).cloned());
    if let Some(owner_order) = owner_order
        && let Ok(mut roots) = owner.trace_root_owner_node_orders.lock()
    {
        roots.insert(root_tala_id, owner_order);
    }
}

#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_restore_graph_reference(owner: &ArenaGraph, owner_order: &[i64]) {
    let mut visible = BTreeSet::new();
    let mut stack = owner_order.to_vec();
    while let Some(tala_id) = stack.pop() {
        if !visible.insert(tala_id) {
            continue;
        }
        let is_aggregate_carrier = owner.clusters.iter().any(|cluster| {
            cluster
                .members
                .first()
                .is_some_and(|member| owner.nodes[member.0 as usize].tala_id == tala_id)
        }) || owner.sequences.iter().any(|sequence| {
            sequence
                .members
                .first()
                .is_some_and(|member| owner.nodes[member.0 as usize].tala_id == tala_id)
        });
        if !is_aggregate_carrier {
            stack.extend(trace_root_graph_child_ids(owner, tala_id));
        }
    }
    if let Ok(mut owners) = owner.trace_owner_node_orders.lock() {
        for tala_id in visible {
            owners.insert(tala_id, owner_order.to_vec());
        }
    }
}

// Diagnostic projection of Go's shared aggregate pointer transitions.
static TRACE_CLUSTER_TRANSITION_SIZES: std::sync::OnceLock<std::sync::Mutex<BTreeMap<i64, Size>>> =
    std::sync::OnceLock::new();
#[cfg(feature = "diagnostic-traces")]
static TRACE_CLUSTER_INITIAL_SIZES: std::sync::OnceLock<std::sync::Mutex<BTreeMap<i64, Size>>> =
    std::sync::OnceLock::new();
#[cfg(feature = "diagnostic-traces")]
static TRACE_CLUSTER_PRE_RESIZE_SIZES: std::sync::OnceLock<std::sync::Mutex<BTreeMap<i64, Size>>> =
    std::sync::OnceLock::new();
static TRACE_CLUSTER_TRANSITION_POSITIONS: std::sync::OnceLock<
    std::sync::Mutex<BTreeMap<i64, Point>>,
> = std::sync::OnceLock::new();
static TRACE_SYNCED_NODES: std::sync::OnceLock<std::sync::Mutex<BTreeMap<i64, ArenaNode>>> =
    std::sync::OnceLock::new();
#[cfg(feature = "diagnostic-traces")]
static TRACE_OWNER_FRAME_NODES: std::sync::OnceLock<std::sync::Mutex<BTreeMap<i64, ArenaNode>>> =
    std::sync::OnceLock::new();
#[cfg(feature = "diagnostic-traces")]
static TRACE_AFTER_FIT_SIZE_NODES: std::sync::OnceLock<
    std::sync::Mutex<BTreeMap<String, Vec<PlacementTraceNode>>>,
> = std::sync::OnceLock::new();
#[cfg(feature = "diagnostic-traces")]
static TRACE_ROOT_GRAPH_BASELINES: std::sync::OnceLock<
    std::sync::Mutex<BTreeMap<String, BTreeMap<i64, (Option<(u64, u64)>, (u64, u64), (u64, u64))>>>,
> = std::sync::OnceLock::new();
static TRACE_BEFORE_FIT_ROOT_POSITIONS: std::sync::OnceLock<
    std::sync::Mutex<BTreeMap<String, BTreeMap<String, (Option<String>, Option<String>)>>>,
> = std::sync::OnceLock::new();
static TRACE_SOURCE_CLUSTER_ROOTS: std::sync::OnceLock<std::sync::Mutex<BTreeSet<i64>>> =
    std::sync::OnceLock::new();
/// Source sibling order captured from the owning root callback before tree
/// extraction rewrites the live `Containers` map. Recursive after-fit scopes
/// consult this immutable diagnostic view to reproduce Go's shared pointers.
static TRACE_SOURCE_CHILD_ORDER: std::sync::OnceLock<std::sync::Mutex<BTreeMap<i64, Vec<i64>>>> =
    std::sync::OnceLock::new();
#[cfg(feature = "diagnostic-traces")]
static TRACE_SOURCE_DIRECTIONS: std::sync::OnceLock<std::sync::Mutex<BTreeMap<i64, Direction>>> =
    std::sync::OnceLock::new();
#[cfg(feature = "diagnostic-traces")]
static TRACE_OWNER_VISIBLE_NODE_IDS: std::sync::OnceLock<
    std::sync::Mutex<BTreeMap<Option<i64>, BTreeSet<i64>>>,
> = std::sync::OnceLock::new();

fn trace_source_child_ids(graph: &ArenaGraph, parent_tala_id: i64) -> Option<Vec<i64>> {
    TRACE_SOURCE_CHILD_ORDER
        .get()
        .and_then(|orders| orders.lock().ok())
        .and_then(|orders| orders.get(&parent_tala_id).cloned())
        .or_else(|| {
            graph
                .source_container_child_order
                .get(&parent_tala_id)
                .cloned()
        })
}

#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_remember_owner_geometry_changes(
    graph: &ArenaGraph,
    changed_tala_ids: &BTreeSet<i64>,
) {
    if changed_tala_ids.is_empty() || !crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT") {
        return;
    }
    let mut synced = match TRACE_SYNCED_NODES
        .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
        .lock()
    {
        Ok(synced) => synced,
        Err(_) => return,
    };
    for tala_id in changed_tala_ids {
        if let Some(node) = graph
            .nodes
            .iter()
            .chain(graph.transaction_external_containers.iter())
            .chain(
                graph
                    .transaction_external_container_children
                    .values()
                    .flatten(),
            )
            .chain(
                graph
                    .transaction_external_aggregate_children
                    .values()
                    .flatten(),
            )
            .chain(graph.trace_container_children.values().flatten())
            .find(|node| node.tala_id == *tala_id)
        {
            synced.insert(*tala_id, node.clone());
        }
    }
}

#[cfg(feature = "diagnostic-traces")]
fn trace_completed_scope_precedes(
    graph: &ArenaGraph,
    earlier_scope_tala_id: i64,
    active_root_tala_id: i64,
) -> Option<bool> {
    if earlier_scope_tala_id == active_root_tala_id {
        return Some(false);
    }
    let ranks = graph.trace_container_placement_ranks.lock().ok()?;
    match (
        ranks.get(&earlier_scope_tala_id),
        ranks.get(&active_root_tala_id),
    ) {
        (Some(earlier), Some(active)) => Some(earlier < active),
        _ => None,
    }
}

#[cfg(feature = "diagnostic-traces")]
fn trace_live_owner_child_ids(
    graph: &ArenaGraph,
    parent_tala_id: i64,
    include_source_children: bool,
) -> Option<Vec<i64>> {
    let is_active_aggregate_carrier = graph.clusters.iter().any(|cluster| {
        graph.cluster_is_active(cluster)
            && cluster
                .members
                .first()
                .is_some_and(|member| graph.nodes[member.0 as usize].tala_id == parent_tala_id)
    }) || graph.sequences.iter().any(|sequence| {
        graph.sequence_is_active(sequence)
            && sequence
                .members
                .first()
                .is_some_and(|member| graph.nodes[member.0 as usize].tala_id == parent_tala_id)
    });
    if is_active_aggregate_carrier {
        // The Rust arena uses the first absorbed member as a stable identity
        // for Go's synthetic cluster/sequence vessel. AddCluster and
        // AddSequence install a fresh node with no Containers children;
        // following the alias's original source children here leaks nested
        // nodes into the vessel's owner walk before Go restores them.
        return Some(Vec::new());
    }
    let scope = if parent_tala_id == 0 {
        None
    } else {
        graph
            .nodes
            .iter()
            .position(|node| node.tala_id == parent_tala_id)
            .map(|index| NodeId(index as u32))
            .map(Some)?
    };
    let tree_children = graph
        .preprocessed_tree_children
        .get(&scope)
        .map(|children| {
            children
                .iter()
                .map(|node| graph.nodes[node.0 as usize].tala_id)
                .collect::<Vec<_>>()
        });
    let mut children = tree_children
        .clone()
        .or_else(|| trace_source_child_ids(graph, parent_tala_id))?;
    let source_children = trace_source_child_ids(graph, parent_tala_id);
    if tree_children.is_some() {
        // PlaceTrees restores peeled tree nodes to the owner only after the
        // corresponding nested scope completes. The caller supplies that
        // lifecycle decision; globally restored tree edges may belong to a
        // later scope in the Go recursion and cannot be used here.
        if let Some(source_children) = &source_children {
            if include_source_children {
                for &child in source_children {
                    if !children.contains(&child) {
                        children.push(child);
                    }
                }
            }
        }
    }

    let expected_parent = if parent_tala_id == 0 {
        None
    } else {
        Some(parent_tala_id)
    };
    for cluster in &graph.clusters {
        let Some(carrier) = cluster.members.first() else {
            continue;
        };
        if !graph.cluster_is_active(cluster) {
            continue;
        }
        let carrier_tala_id = graph.nodes[carrier.0 as usize].tala_id;
        let member_ids = cluster
            .members
            .iter()
            .map(|member| graph.nodes[member.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        let members_share_owner = member_ids.iter().all(|member_tala_id| {
            graph
                .source_parent_tala_by_tala
                .get(member_tala_id)
                .copied()
                == Some(expected_parent)
        });
        if !members_share_owner {
            continue;
        }
        children.retain(|child| !member_ids.contains(child));
        if !children.contains(&carrier_tala_id) {
            // AddCluster appends the replacement vessel after the surviving
            // direct children in the live Containers slice. The arena keeps
            // the first member as the stable carrier for that vessel.
            children.push(carrier_tala_id);
        }
    }
    for members in graph.transaction_external_aggregate_children.values() {
        let Some(carrier) = members.first() else {
            continue;
        };
        let members_share_owner = members.iter().all(|member| {
            graph
                .source_parent_tala_by_tala
                .get(&member.tala_id)
                .copied()
                == Some(expected_parent)
        });
        if members_share_owner && !children.contains(&carrier.tala_id) {
            // A completed child transaction keeps the aggregate's member
            // aliases and vessel geometry separately from the owner's live
            // child slice. The owner walk still sees the replacement vessel
            // appended after the surviving children; use the first member
            // as its stable handle so the later trace projection can resolve
            // that handle to the reconstructed vessel record.
            children.push(carrier.tala_id);
        }
    }
    if std::env::var_os("D2_RS_TRACE_OWNER_GRAPH").is_some() {
        let restored_tree_nodes = graph
            .tree_routing_nodes
            .iter()
            .filter(|(_, tree)| graph.restored_tree_edges.contains(&tree.sentinel_edge))
            .map(|(node_id, tree)| {
                let node = &graph.nodes[node_id.0 as usize];
                (
                    node.external_id.as_str(),
                    node.tala_id,
                    graph.source_parent_tala_by_tala.get(&node.tala_id),
                    tree.extraction_order,
                )
            })
            .collect::<Vec<_>>();
        eprintln!(
            "D2_RS_TRACE_OWNER_CHILD parent={} scope={:?} tree={:?} source={:?} restored={:?} live={:?}",
            parent_tala_id, scope, tree_children, source_children, restored_tree_nodes, children
        );
    }
    Some(children)
}

#[cfg(feature = "diagnostic-traces")]
fn trace_owner_container_walk_order(
    graph: &ArenaGraph,
    roots: &[NodeId],
    include_descendant_nodes: bool,
    phase: &str,
    root_tala_id: Option<i64>,
    root_is_top: bool,
) -> Vec<NodeId> {
    fn visit(
        tala_id: i64,
        graph: &ArenaGraph,
        by_tala_id: &BTreeMap<i64, NodeId>,
        visible_tala_ids: &BTreeSet<i64>,
        phase: &str,
        root_tala_id: Option<i64>,
        root_is_top: bool,
        seen: &mut BTreeSet<i64>,
        output: &mut Vec<NodeId>,
    ) {
        if !seen.insert(tala_id) {
            return;
        }
        if visible_tala_ids.contains(&tala_id)
            && let Some(&node_id) = by_tala_id.get(&tala_id)
        {
            output.push(node_id);
        }
        // The Go callback walks the selected owner's Containers map even when
        // a descendant's Node.Graph reference has moved to a recursively
        // created child graph. The child reference must not suppress this
        // traversal of the owner graph's current container map.
        let children =
            trace_owner_phase_child_ids(graph, tala_id, phase, root_tala_id, root_is_top);
        for child in children {
            visit(
                child,
                graph,
                by_tala_id,
                visible_tala_ids,
                phase,
                root_tala_id,
                root_is_top,
                seen,
                output,
            );
        }
    }

    let by_tala_id = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.tala_id, NodeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut visible_tala_ids = roots
        .iter()
        .map(|node| graph.nodes[node.0 as usize].tala_id)
        .collect::<BTreeSet<_>>();
    if include_descendant_nodes {
        let mut pending = visible_tala_ids.iter().copied().collect::<Vec<_>>();
        while let Some(parent_tala_id) = pending.pop() {
            let children = trace_owner_phase_child_ids(
                graph,
                parent_tala_id,
                phase,
                root_tala_id,
                root_is_top,
            );
            for child_tala_id in children {
                if visible_tala_ids.insert(child_tala_id) {
                    pending.push(child_tala_id);
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::with_capacity(roots.len());
    for node_id in roots {
        visit(
            graph.nodes[node_id.0 as usize].tala_id,
            graph,
            &by_tala_id,
            &visible_tala_ids,
            phase,
            root_tala_id,
            root_is_top,
            &mut seen,
            &mut ordered,
        );
    }
    for node_id in roots {
        let tala_id = graph.nodes[node_id.0 as usize].tala_id;
        if seen.insert(tala_id) {
            ordered.push(*node_id);
        }
    }
    if roots
        .iter()
        .all(|node_id| seen.contains(&graph.nodes[node_id.0 as usize].tala_id))
    {
        if std::env::var_os("D2_RS_TRACE_OWNER_GRAPH").is_some() {
            let labels = |order: &[NodeId]| {
                order
                    .iter()
                    .map(|node_id| graph.nodes[node_id.0 as usize].external_id.clone())
                    .collect::<Vec<_>>()
            };
            eprintln!(
                "D2_RS_TRACE_OWNER_WALK phase={phase} root={root_tala_id:?} input={:?} output={:?}",
                labels(roots),
                labels(&ordered)
            );
        }
        ordered
    } else {
        roots.to_vec()
    }
}

fn trace_phase_uses_live_owner_children(phase: &str) -> bool {
    matches!(
        phase,
        "initialized"
            | "placeNodes-entry"
            | "after-child-recursion"
            | "after-nears"
            | "after-herds"
            | "after-split"
            | "before-fit"
            | "after-fit-size"
            | "after-fit"
            | "after-orient"
            | "before-combine"
            | "after-combine"
    ) || [
        "before-subgraph-",
        "orthogonal-after-",
        "after-orthogonal-",
        "after-cluster-optimize-",
        "after-trees-",
        "after-restore-edges-",
        "after-direct-owner-",
        "after-subgraph-",
    ]
    .iter()
    .any(|prefix| phase.starts_with(prefix))
}

fn trace_phase_follows_child_recursion(phase: &str) -> bool {
    matches!(
        phase,
        "orthogonal-start"
            | "after-child-recursion"
            | "after-nears"
            | "after-herds"
            | "after-split"
            | "before-fit"
            | "after-fit-size"
            | "after-fit"
            | "after-orient"
            | "before-combine"
            | "after-combine"
    ) || [
        "orthogonal-after-",
        "before-subgraph-",
        "after-subgraph-",
        "after-direct-owner-",
        "after-orthogonal-",
        "after-cluster-optimize-",
        "after-trees-",
        "after-restore-edges-",
    ]
    .iter()
    .any(|prefix| phase.starts_with(prefix))
}

fn trace_owner_scope_parent_tala_id(graph: &ArenaGraph, root: &str) -> Option<Option<i64>> {
    if root.is_empty() || root == "top" {
        return Some(None);
    }
    let node = graph.nodes.iter().find(|node| node.external_id == root)?;
    Some(
        graph
            .source_parent_tala_by_tala
            .get(&node.tala_id)
            .copied()
            .unwrap_or_else(|| {
                node.container
                    .map(|container| graph.nodes[container.0 as usize].tala_id)
            }),
    )
}

fn trace_node_in_owner_scope(graph: &ArenaGraph, tala_id: i64, owner_parent: Option<i64>) -> bool {
    let mut current = tala_id;
    let mut seen = BTreeSet::new();
    while seen.insert(current) {
        let parent = graph
            .source_parent_tala_by_tala
            .get(&current)
            .copied()
            .unwrap_or_else(|| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == current)
                    .and_then(|node| node.container)
                    .map(|container| graph.nodes[container.0 as usize].tala_id)
            });
        if parent == owner_parent {
            return true;
        }
        let Some(parent) = parent else {
            return false;
        };
        current = parent;
    }
    false
}

fn trace_node_is_hidden_by_active_aggregate(graph: &ArenaGraph, node: NodeId) -> bool {
    if graph.active_aggregate_owner(node) != node {
        return true;
    }
    let mut parent = graph.nodes[node.0 as usize].container;
    while let Some(container) = parent {
        if graph.active_aggregate_owner(container) != container {
            return true;
        }
        parent = graph.nodes[container.0 as usize].container;
    }
    false
}

fn trace_child_order_is_node_ordered(graph: &ArenaGraph, parent_tala_id: i64) -> bool {
    let Some(children) = graph.trace_container_children.get(&parent_tala_id) else {
        return true;
    };
    let mut previous = None;
    for child in children {
        // Absorbed cluster members retain their source slots in this snapshot,
        // but AddCluster has removed them from the live Containers slice.
        // Their carrier's later Graph.Nodes position is not a sibling reversal.
        if graph.clusters.iter().any(|cluster| {
            graph.cluster_is_active(cluster)
                && cluster
                    .members
                    .iter()
                    .any(|member| graph.nodes[member.0 as usize].tala_id == child.tala_id)
        }) {
            continue;
        }

        let Some(rank) = graph
            .node_order
            .iter()
            .position(|node_id| graph.nodes[node_id.0 as usize].tala_id == child.tala_id)
        else {
            continue;
        };
        if previous.is_some_and(|previous| rank < previous) {
            return false;
        }
        previous = Some(rank);
    }
    true
}

// Keep the fallback lazy so ordinary graph metadata avoids locking the
// diagnostic-only remembered-directions map.
#[allow(clippy::unnecessary_lazy_evaluations)]
fn trace_direction_for_tala(graph: &ArenaGraph, tala_id: i64) -> Option<Direction> {
    graph
        .nodes
        .iter()
        .chain(
            graph
                .transaction_external_container_children
                .values()
                .flatten(),
        )
        .chain(graph.trace_container_children.values().flatten())
        .find(|node| node.tala_id == tala_id)
        .and_then(|node| graph.directions.get(&Some(node.input_id)).copied())
        .or_else(|| graph.directions_by_tala.get(&Some(tala_id)).copied())
        .or_else(|| {
            #[cfg(feature = "diagnostic-traces")]
            {
                TRACE_SOURCE_DIRECTIONS
                    .get()
                    .and_then(|directions| directions.lock().ok())
                    .and_then(|directions| directions.get(&tala_id).copied())
            }
            #[cfg(not(feature = "diagnostic-traces"))]
            {
                None
            }
        })
}

#[cfg(feature = "diagnostic-traces")]
fn trace_remember_source_directions(graph: &ArenaGraph) {
    let Ok(mut remembered) = TRACE_SOURCE_DIRECTIONS
        .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
        .lock()
    else {
        return;
    };
    for node in graph
        .nodes
        .iter()
        .chain(
            graph
                .transaction_external_container_children
                .values()
                .flatten(),
        )
        .chain(graph.trace_container_children.values().flatten())
        .chain(graph.transaction_external_containers.iter())
        .chain(
            graph
                .transaction_external_aggregate_children
                .values()
                .flatten(),
        )
    {
        if let Some(direction) = graph
            .directions
            .get(&Some(node.input_id))
            .copied()
            .or_else(|| graph.directions_by_tala.get(&Some(node.tala_id)).copied())
        {
            remembered.entry(node.tala_id).or_insert(direction);
        }
    }
}

fn trace_snapshot_child_ids(
    graph: &ArenaGraph,
    parent_tala_id: i64,
    use_live_container_order: bool,
) -> Vec<i64> {
    let current = if use_live_container_order {
        #[cfg(feature = "diagnostic-traces")]
        {
            trace_live_owner_child_ids(graph, parent_tala_id, false)
                .unwrap_or_else(|| trace_child_tala_ids(graph, parent_tala_id))
        }
        #[cfg(not(feature = "diagnostic-traces"))]
        {
            graph
                .trace_container_children
                .get(&parent_tala_id)
                .map(|children| children.iter().map(|child| child.tala_id).collect())
                .unwrap_or_else(|| trace_child_tala_ids(graph, parent_tala_id))
        }
    } else {
        trace_child_tala_ids(graph, parent_tala_id)
    };
    if use_live_container_order {
        return current;
    }
    let preserves_source_order =
        trace_direction_for_tala(graph, parent_tala_id).is_some_and(|direction| {
            matches!(
                direction,
                Direction::Right | Direction::Left | Direction::Up
            )
        });

    // Go's tracePlacement walks the owning Graph.Containers slice at every
    // boundary. Nested Rust transactions can expose the same children in
    // arena-derived order. Right-, left-, and up-directed scopes retain the
    // source order of ordinary nodes; down-directed scopes and aggregate
    // vessels preserve the live transaction order.
    if preserves_source_order {
        let source_nodes = current
            .iter()
            .map(|tala_id| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == *tala_id)
                    .or_else(|| {
                        graph
                            .transaction_external_container_children
                            .values()
                            .flatten()
                            .find(|node| node.tala_id == *tala_id)
                    })
                    .or_else(|| {
                        graph
                            .trace_container_children
                            .values()
                            .flatten()
                            .find(|node| node.tala_id == *tala_id)
                    })
            })
            .collect::<Option<Vec<_>>>();
        if let Some(source_nodes) = source_nodes {
            let input_ids = source_nodes
                .iter()
                .map(|node| node.input_id)
                .collect::<BTreeSet<_>>();
            if source_nodes.len() == current.len()
                && source_nodes
                    .iter()
                    .all(|node| !node.scoring_is_aggregate_vessel)
                && input_ids.len() == source_nodes.len()
            {
                if let Some(source_ids) = trace_source_child_ids(graph, parent_tala_id) {
                    let current_ids = current.iter().copied().collect::<BTreeSet<_>>();
                    let source_id_set = source_ids.iter().copied().collect::<BTreeSet<_>>();
                    if source_ids.len() == current.len()
                        && source_ids.len() == source_id_set.len()
                        && source_id_set == current_ids
                    {
                        return source_ids;
                    }
                }
            }
        }
    }
    current
}

// AddCluster appends its vessel before removing absorbed members from
// Containers. Reconstruct that mutation when walking retained source aliases.
fn trace_append_cluster_vessels(graph: &ArenaGraph, children: &mut [i64]) {
    children.stable_sort_by_key(|child| {
        projected_vessel_trace_id(graph, *child).is_some()
            || graph
                .transaction_external_cluster_layouts
                .keys()
                .any(|vessel| {
                    graph
                        .transaction_external_aggregate_children
                        .get(vessel)
                        .is_some_and(|members| {
                            members.iter().any(|member| member.tala_id == *child)
                        })
                })
            || graph.clusters.iter().any(|cluster| {
                graph.cluster_is_active(cluster)
                    && (cluster.vessel_tala_id == *child
                        || cluster
                            .members
                            .iter()
                            .any(|member| graph.nodes[member.0 as usize].tala_id == *child))
            })
    });
}

fn trace_child_tala_ids(graph: &ArenaGraph, parent_tala_id: i64) -> Vec<i64> {
    let mut ids = graph
        .transaction_external_container_children
        .get(&parent_tala_id)
        .or_else(|| graph.trace_container_children.get(&parent_tala_id))
        .map(|children| {
            children
                .iter()
                .map(|child| child.tala_id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(source_ids) = trace_source_child_ids(graph, parent_tala_id) {
        for tala_id in source_ids {
            if !ids.contains(&tala_id) {
                ids.push(tala_id);
            }
        }
    }
    ids
}

fn trace_root_graph_child_ids(graph: &ArenaGraph, parent_tala_id: i64) -> Vec<i64> {
    #[cfg(feature = "diagnostic-traces")]
    if graph
        .nodes
        .iter()
        .find(|node| node.tala_id == parent_tala_id)
        .is_some_and(|node| !node.is_container)
    {
        // Go's Graph.Containers has children only for container nodes. Falling
        // back to the source hierarchy for a non-container can expose nodes
        // peeled into another recursive graph to a root.Graph snapshot.
        return Vec::new();
    }
    #[cfg(feature = "diagnostic-traces")]
    let children = trace_live_owner_child_ids(graph, parent_tala_id, false)
        .unwrap_or_else(|| trace_child_tala_ids(graph, parent_tala_id));
    #[cfg(not(feature = "diagnostic-traces"))]
    let children = graph
        .transaction_external_container_children
        .get(&parent_tala_id)
        .map(|children| {
            children
                .iter()
                .map(|child| child.tala_id)
                .collect::<Vec<_>>()
        })
        .or_else(|| {
            graph
                .trace_container_children
                .get(&parent_tala_id)
                .map(|children| {
                    children
                        .iter()
                        .map(|child| child.tala_id)
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_else(|| trace_child_tala_ids(graph, parent_tala_id));
    let mut owners = BTreeMap::<i64, i64>::new();
    for cluster in &graph.clusters {
        if !graph.cluster_is_active(cluster) {
            continue;
        }
        let Some(carrier) = cluster.members.first() else {
            continue;
        };
        let carrier_tala_id = graph.nodes[carrier.0 as usize].tala_id;
        owners.insert(cluster.vessel_tala_id, carrier_tala_id);
        for member in &cluster.members {
            owners.insert(graph.nodes[member.0 as usize].tala_id, carrier_tala_id);
        }
    }
    for sequence in &graph.sequences {
        if !graph.sequence_is_active(sequence) {
            continue;
        }
        let Some(carrier) = sequence.members.first() else {
            continue;
        };
        let carrier_tala_id = graph.nodes[carrier.0 as usize].tala_id;
        for member in &sequence.members {
            owners.insert(graph.nodes[member.0 as usize].tala_id, carrier_tala_id);
        }
    }
    let mut seen = BTreeSet::new();
    children
        .into_iter()
        .filter_map(|child| {
            let owner = owners.get(&child).copied().unwrap_or(child);
            seen.insert(owner).then_some(owner)
        })
        .collect()
}

#[cfg(feature = "diagnostic-traces")]
fn trace_owner_phase_child_ids(
    graph: &ArenaGraph,
    parent_tala_id: i64,
    phase: &str,
    root_tala_id: Option<i64>,
    root_is_top: bool,
) -> Vec<i64> {
    let source_parent = |tala_id| {
        graph
            .source_parent_tala_by_tala
            .get(&tala_id)
            .copied()
            .flatten()
            .or_else(|| {
                graph
                    .source_container_child_order
                    .iter()
                    .find_map(|(&parent, children)| children.contains(&tala_id).then_some(parent))
            })
    };
    let owner_parent_tala_id = root_tala_id.and_then(source_parent);
    let parent_is_owner_sibling = matches!(
        phase,
        "placeNodes-entry"
            | "after-child-recursion"
            | "after-nears"
            | "after-herds"
            | "after-split"
            | "before-fit"
            | "after-fit-size"
            | "after-fit"
            | "after-orient"
            | "before-combine"
            | "after-combine"
    ) || phase.starts_with("before-subgraph-")
        || phase.starts_with("after-subgraph-")
        || phase.starts_with("after-direct-owner-")
        || phase.starts_with("orthogonal-after-")
        || phase.starts_with("after-orthogonal-")
        || phase.starts_with("after-cluster-optimize-")
        || phase.starts_with("after-trees-")
        || phase.starts_with("after-restore-edges-");
    let parent_is_owner_sibling = parent_is_owner_sibling
        && root_tala_id != Some(parent_tala_id)
        && owner_parent_tala_id.is_some()
        && source_parent(parent_tala_id) == owner_parent_tala_id;
    let active_root_descendant_scope = root_tala_id.is_some_and(|root| {
        let mut current = parent_tala_id;
        let mut seen = BTreeSet::new();
        while seen.insert(current) {
            let Some(parent) = source_parent(current) else {
                break;
            };
            if parent == root {
                return true;
            }
            current = parent;
        }
        false
    });
    let active_root_contains_scope =
        root_tala_id == Some(parent_tala_id) || active_root_descendant_scope;
    // Placement callbacks observe the current Containers map. A source scope
    // whose recursive placement completed before the active root has restored
    // its tree descendants. Use the recovered PlaceChildrenOrder postorder,
    // since edge-driven placement can differ from source declaration order.
    #[cfg(feature = "diagnostic-traces")]
    let prior_scope_children_are_restored = !active_root_contains_scope
        && root_tala_id
            .and_then(|root| trace_completed_scope_precedes(graph, parent_tala_id, root))
            .unwrap_or(parent_is_owner_sibling);
    let active_root_descendant_scope_is_restored =
        trace_phase_follows_child_recursion(phase) && active_root_descendant_scope;
    let top_scope_descendants_are_restored =
        root_is_top && trace_phase_follows_child_recursion(phase);
    let include_source_children = prior_scope_children_are_restored
        || active_root_descendant_scope_is_restored
        || top_scope_descendants_are_restored
        || (root_tala_id == Some(parent_tala_id)
            && (matches!(
                phase,
                "before-fit"
                    | "after-fit-size"
                    | "after-fit"
                    | "after-orient"
                    | "before-combine"
                    | "after-combine"
            ) || phase.starts_with("after-trees-")
                || phase.starts_with("after-restore-edges-")
                || phase.starts_with("after-direct-owner-")
                || phase.starts_with("after-subgraph-")));
    #[cfg(feature = "diagnostic-traces")]
    {
        trace_live_owner_child_ids(graph, parent_tala_id, include_source_children)
            .unwrap_or_else(|| trace_child_tala_ids(graph, parent_tala_id))
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    {
        trace_snapshot_child_ids(graph, parent_tala_id, true)
    }
}

#[cfg(not(feature = "diagnostic-traces"))]
fn trace_owner_phase_child_ids(
    graph: &ArenaGraph,
    parent_tala_id: i64,
    _phase: &str,
    _root_tala_id: Option<i64>,
    _root_is_top: bool,
) -> Vec<i64> {
    trace_snapshot_child_ids(graph, parent_tala_id, true)
}

/// Record the temporary vessel dimensions created by AddClusters. Go keeps
/// this vessel box independent while nested members are fitted; diagnostic
/// snapshots therefore need the pre-expansion dimensions at a member's
/// after-fit callback.
#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_record_cluster_initial_size(vessel_tala_id: i64, size: Size) {
    if let Ok(mut sizes) = TRACE_CLUSTER_INITIAL_SIZES
        .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
        .lock()
    {
        sizes.entry(vessel_tala_id).or_insert(size);
    }
}

#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_record_cluster_transition_size(vessel_tala_id: i64, size: Size) {
    if let Ok(mut sizes) = TRACE_CLUSTER_TRANSITION_SIZES
        .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
        .lock()
    {
        sizes.insert(vessel_tala_id, size);
    }
}

#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_record_cluster_pre_resize_size(vessel_tala_id: i64, size: Size) {
    if let Ok(mut sizes) = TRACE_CLUSTER_PRE_RESIZE_SIZES
        .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
        .lock()
    {
        sizes.insert(vessel_tala_id, size);
    }
}

#[derive(Serialize)]
struct NormalizedTracePoint {
    x_bits: String,
    y_bits: String,
}

#[derive(Serialize)]
struct NormalizedTraceNode {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    x_bits: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    y_bits: Option<String>,
    w_bits: String,
    h_bits: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label_position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_position: Option<String>,
}

#[derive(Serialize)]
struct NormalizedTraceEdge {
    id: String,
    index: usize,
    from: String,
    to: String,
    #[serde(rename = "curve", skip_serializing_if = "trace_bool_is_false")]
    curve: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    route: Vec<NormalizedTracePoint>,
}

fn trace_bool_is_false(value: &bool) -> bool {
    !value
}

#[derive(Serialize)]
struct NormalizedTraceEvent {
    event: &'static str,
    stage: String,
    seed: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    nodes: Vec<NormalizedTraceNode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    edges: Vec<NormalizedTraceEdge>,
}

#[derive(Clone, Serialize)]
struct PlacementTraceNode {
    id: String,
    stable_id: String,
    w_bits: String,
    h_bits: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    x_bits: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    y_bits: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fixed_x_bits: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fixed_y_bits: Option<String>,
}

#[cfg(feature = "diagnostic-traces")]
fn trace_placement_node_position(record: &PlacementTraceNode) -> Option<Point> {
    fn coordinate(bits: &str) -> Option<f64> {
        u64::from_str_radix(bits.strip_prefix("0x").unwrap_or(bits), 16)
            .ok()
            .map(f64::from_bits)
    }

    Some(Point {
        x: coordinate(record.x_bits.as_deref()?)?,
        y: coordinate(record.y_bits.as_deref()?)?,
    })
}

#[cfg(feature = "diagnostic-traces")]
fn trace_placement_target_records(
    phase: &str,
    root: &str,
    stage: &str,
    records: &[PlacementTraceNode],
) {
    if !matches!(phase, "after-herds" | "after-split") || !matches!(root, "" | "top") {
        return;
    }
    let Some(target) = crate::internal::trace_env_value("TALA_TRACE_TARGET_NODE") else {
        return;
    };
    let matches = records
        .iter()
        .filter(|record| record.id == target)
        .map(|record| (record.x_bits.as_deref(), record.y_bits.as_deref()))
        .collect::<Vec<_>>();
    eprintln!(
        "TRACE_TARGET_RECORDS_RUST phase={phase} root={root} stage={stage} node={target} records={matches:?}"
    );
}

#[cfg(feature = "diagnostic-traces")]
fn trace_placement_order_records(
    phase: &str,
    root: &str,
    stage: &str,
    records: &[PlacementTraceNode],
) {
    if (phase == "after-cluster-optimize-0"
        && root.is_empty()
        && crate::internal::trace_env_enabled("TALA_TRACE_NODE_ORDER"))
        || std::env::var_os("D2_RS_TRACE_OWNER_GRAPH").is_some()
    {
        eprintln!(
            "TRACE_RECORD_ORDER_RUST phase={phase} root={root:?} stage={stage} ids={:?}",
            records
                .iter()
                .map(|record| (record.stable_id.as_str(), record.id.as_str()))
                .collect::<Vec<_>>()
        );
    }
}

#[derive(Serialize)]
struct PlacementTraceEdge {
    from: String,
    to: String,
}

#[derive(Serialize)]
struct PlacementTraceEvent {
    event: &'static str,
    phase: String,
    root: String,
    cell_bits: String,
    // Keep the topology available to the Rust-side debugger without putting
    // Rust-only fields into the normalized cross-language event. The v0.9.0
    // Go trace serializes only the active node placement snapshot here.
    #[serde(skip_serializing)]
    graph_nodes: Vec<String>,
    #[serde(skip_serializing)]
    graph_edges: Vec<PlacementTraceEdge>,
    nodes: Vec<PlacementTraceNode>,
}

#[derive(Serialize)]
struct PlacementCandidatePoint {
    x_bits: String,
    y_bits: String,
}

#[derive(Serialize)]
struct PlacementCandidatesEvent {
    event: &'static str,
    phase: &'static str,
    node: String,
    median: PlacementCandidatePoint,
    distance_bits: String,
    minimizing_self: bool,
    points: Vec<PlacementCandidatePoint>,
}

#[derive(Serialize)]
struct PlacementScoreEvent {
    event: &'static str,
    phase: &'static str,
    node: String,
    x_bits: String,
    y_bits: String,
    score_bits: String,
    least_bits: String,
}

fn trace_float_bits(value: f64) -> String {
    format!("0x{:016x}", value.to_bits())
}

fn trace_candidate_point(point: Point) -> PlacementCandidatePoint {
    PlacementCandidatePoint {
        x_bits: trace_float_bits(point.x),
        y_bits: trace_float_bits(point.y),
    }
}

pub(crate) fn trace_placement_candidates_graph(
    phase: &'static str,
    graph: &ArenaGraph,
    node_id: NodeId,
    median: Point,
    distance: f64,
    minimizing_self: bool,
    points: &[Point],
) {
    if !crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT") {
        return;
    }
    let event = PlacementCandidatesEvent {
        event: "placement-candidates",
        phase,
        node: trace_node_external_id(graph, node_id),
        median: trace_candidate_point(median),
        distance_bits: trace_float_bits(distance),
        minimizing_self,
        points: points.iter().copied().map(trace_candidate_point).collect(),
    };
    if let Ok(encoded) = serde_json::to_string(&event) {
        eprintln!("{encoded}");
    }
}

pub(crate) fn trace_placement_score_graph(
    phase: &'static str,
    graph: &ArenaGraph,
    node_id: NodeId,
    position: Point,
    score: f64,
    least: f64,
) {
    if !crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT") {
        return;
    }
    let event = PlacementScoreEvent {
        event: "placement-score",
        phase,
        node: trace_node_external_id(graph, node_id),
        x_bits: trace_float_bits(position.x),
        y_bits: trace_float_bits(position.y),
        score_bits: trace_float_bits(score),
        least_bits: trace_float_bits(least),
    };
    if let Ok(encoded) = serde_json::to_string(&event) {
        eprintln!("{encoded}");
    }
}

fn trace_optimizer_state(phase: std::fmt::Arguments<'_>, graph: &ArenaGraph) {
    let Some(target) = crate::internal::trace_env_value("TALA_TRACE_OPTIMIZER_MEMBER")
        .and_then(|value| value.parse::<i64>().ok())
    else {
        return;
    };
    if !graph.nodes.iter().any(|node| node.tala_id == target) {
        return;
    }
    if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_ONLY") {
        const ROOT_IDS: [i64; 4] = [
            1639028324, // feed mixer
            322706392,  // ingress
            727256374,  // social frontend
            308016937,  // source pack
        ];
        if graph.nodes.len() != 15
            || ROOT_IDS
                .iter()
                .any(|id| !graph.nodes.iter().any(|node| node.tala_id == *id))
        {
            return;
        }
    }
    eprint!("OPTIMIZER_STATE_RUST {phase} len={}", graph.nodes.len());
    for node in &graph.nodes {
        eprint!(" {}=", node.tala_id);
        // Match Go's traceSizedState, which reads the materialized Graph.Nodes
        // slice directly.  The active projection can map a retired member to
        // its aggregate vessel and therefore is useful for placement itself,
        // but it is the wrong view for comparing this optimizer's state.
        if let Some(position) = node.position {
            eprint!(
                "{},{}:{},{}",
                position.x, position.y, node.rect.size.width, node.rect.size.height,
            );
        } else {
            eprint!("nil");
        }
    }
    eprintln!();
}

fn trace_sized_state_scope(graph: &ArenaGraph) -> bool {
    let Some(ids) = crate::internal::trace_env_value("TALA_TRACE_SIZED_STATE_IDS") else {
        return false;
    };
    if let Some(target) = ids
        .strip_prefix("contains:")
        .and_then(|target| target.trim().parse::<i64>().ok())
    {
        return graph.nodes.iter().any(|node| node.tala_id == target);
    }
    let wanted = ids
        .split(',')
        .filter_map(|id| id.trim().parse::<i64>().ok())
        .collect::<BTreeSet<_>>();
    wanted.len() == graph.nodes.len()
        && graph
            .nodes
            .iter()
            .all(|node| wanted.contains(&node.tala_id))
}

fn trace_sized_boundary(label: &str, graph: &ArenaGraph) {
    if !crate::internal::trace_env_enabled("TALA_TRACE_SIZED_STATE")
        || !trace_sized_state_scope(graph)
    {
        return;
    }
    eprint!("SIZED_BOUNDARY_RUST {label}");
    for node in &graph.nodes {
        eprint!(" {}@{:?}", node.tala_id, graph.position(node.input_id));
    }
    eprintln!();
}

pub(crate) fn trace_node_external_id(graph: &ArenaGraph, node_id: NodeId) -> String {
    fn cluster_trace_id(graph: &ArenaGraph, cluster_index: usize) -> String {
        let cluster = &graph.clusters[cluster_index];
        let members = cluster
            .members
            .iter()
            .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let arrangement = match cluster.arrangement {
            ClusterArrangement::Row => "Row",
            ClusterArrangement::Column => "Column",
        };
        format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}")
    }
    fn sequence_trace_id(graph: &ArenaGraph, sequence_index: usize) -> String {
        let members = graph.sequences[sequence_index]
            .members
            .iter()
            .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        format!("Sequence vessel of: [{members}]")
    }

    // `Node.DebugID` prefers a serialized D2ID over the aggregate-vessel
    // fallback. Active sequence vessels can carry such an ID in GraphJSON;
    // preserve it before synthesizing a member-based label.
    let external_id = &graph.nodes[node_id.0 as usize].external_id;
    if !external_id.is_empty()
        && !external_id.ends_with("#cluster-vessel")
        && !external_id.ends_with("#sequence-vessel")
    {
        return external_id.clone();
    }

    if let Some(cluster_index) = graph.active_cluster_index(node_id) {
        return cluster_trace_id(graph, cluster_index);
    }
    if let Some(sequence_index) = graph.active_sequence_index(node_id) {
        return sequence_trace_id(graph, sequence_index);
    }

    // Temporary transaction graphs use a source-name suffix for synthetic
    // aggregate vessels (for example `2511174837#cluster-vessel`).  Those
    // nodes are intentionally detached from the active hierarchy, but Go's
    // shared vessel pointer still reports the descriptive DebugID. Recover it
    // here for trace identity without changing the release graph.
    if let Some((base, suffix)) = external_id.rsplit_once('#') {
        if suffix == "cluster-vessel"
            && let Some(cluster_index) = graph.clusters.iter().position(|cluster| {
                cluster
                    .members
                    .iter()
                    .any(|member| graph.nodes[member.0 as usize].external_id == base)
            })
        {
            return cluster_trace_id(graph, cluster_index);
        }
        if suffix == "sequence-vessel"
            && let Some(sequence_index) = graph.sequences.iter().position(|sequence| {
                sequence
                    .members
                    .iter()
                    .any(|member| graph.nodes[member.0 as usize].external_id == base)
            })
        {
            return sequence_trace_id(graph, sequence_index);
        }
        // After a scope is flattened, the active aggregate record can be
        // gone while its transaction projection still retains the member
        // slice. Recover the Go vessel DebugID from that shared slice.
        if suffix == "cluster-vessel"
            && let Some((vessel_tala_id, members)) = graph
                .transaction_external_aggregate_children
                .iter()
                .find(|(_, children)| children.iter().any(|child| child.external_id == base))
        {
            let member_names = members
                .iter()
                .map(|member| member.external_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let arrangement = graph
                .transaction_external_cluster_layouts
                .get(vessel_tala_id)
                .map(|layout| match layout.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                })
                .unwrap_or("Column");
            return format!("Cluster vessel of: [{member_names}]; Arrangement: {arrangement}");
        }
        if suffix == "sequence-vessel"
            && let Some(members) = graph
                .transaction_external_aggregate_children
                .values()
                .find(|children| children.iter().any(|child| child.external_id == base))
        {
            let member_names = members
                .iter()
                .map(|member| member.external_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return format!("Sequence vessel of: [{member_names}]");
        }
    }
    external_id.clone()
}

fn projected_vessel_trace_id(graph: &ArenaGraph, tala_id: i64) -> Option<String> {
    let members = graph
        .transaction_external_aggregate_children
        .get(&tala_id)?;
    let names = members
        .iter()
        .map(|member| member.external_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if let Some(layout) = graph.transaction_external_cluster_layouts.get(&tala_id) {
        let arrangement = match layout.arrangement {
            ClusterArrangement::Row => "Row",
            ClusterArrangement::Column => "Column",
        };
        Some(format!(
            "Cluster vessel of: [{names}]; Arrangement: {arrangement}"
        ))
    } else {
        Some(format!("Sequence vessel of: [{names}]"))
    }
}

/// Development-only placement snapshots matching the recovered Go
/// `tracePlacement` boundaries.  This is deliberately gated by the generic
/// diagnostic environment and is never compiled into a normal build.
#[allow(clippy::collapsible_if)]
pub(crate) fn trace_placement_state(phase: &str, root: &str, graph: &ArenaGraph) {
    trace_placement_state_inner(phase, root, graph, None, None);
}

#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_placement_state_with_owner_changes(
    phase: &str,
    root: &str,
    graph: &ArenaGraph,
    changed_tala_ids: &BTreeSet<i64>,
) {
    trace_placement_state_inner(phase, root, graph, None, Some(changed_tala_ids));
}

/// Trace owner-visible geometry with the active child graph's lattice size.
/// Root-graph snapshots intentionally use a different graph for node pointers
/// than the temporary placement graph used by TALA's optimizer.
#[cfg(feature = "diagnostic-traces")]
pub(crate) fn trace_placement_state_with_cell_size(
    phase: &str,
    root: &str,
    graph: &ArenaGraph,
    cell_size: f64,
) {
    trace_placement_state_inner(phase, root, graph, Some(cell_size), None);
}

fn trace_placement_state_inner(
    phase: &str,
    root: &str,
    graph: &ArenaGraph,
    _cell_size_override: Option<f64>,
    _owner_changes: Option<&BTreeSet<i64>>,
) {
    if !crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT") {
        return;
    }
    let phase_subgraph_index = [
        "after-orthogonal-",
        "after-cluster-optimize-",
        "after-trees-",
        "after-restore-edges-",
        "after-direct-owner-",
        "after-subgraph-",
        "before-subgraph-",
    ]
    .iter()
    .find_map(|prefix| {
        phase
            .strip_prefix(prefix)
            .and_then(|index| index.parse::<usize>().ok())
    });
    let sequence_component_indices_by_tala = if phase_subgraph_index.is_some()
        && graph
            .sequences
            .iter()
            .any(|sequence| sequence.container.is_none())
    {
        graph
            .split_placement_subgraphs()
            .into_iter()
            .enumerate()
            .flat_map(|(component_index, members)| {
                members
                    .into_iter()
                    .map(move |member| (graph.nodes[member.0 as usize].tala_id, component_index))
            })
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    #[cfg(feature = "diagnostic-traces")]
    if let Some(target) = crate::internal::trace_env_value("TALA_TRACE_TARGET_NODE")
        .and_then(|value| value.parse::<i64>().ok())
        && matches!(phase, "after-herds" | "after-split")
    {
        let stable = graph
            .nodes
            .iter()
            .find(|node| node.tala_id == target)
            .map(|node| (node.position, node.rect.origin, node.container));
        let projected = graph
            .transaction_external_container_children
            .iter()
            .flat_map(|(parent, children)| {
                children
                    .iter()
                    .filter(|node| node.tala_id == target)
                    .map(|node| (*parent, node.position, node.rect.origin))
            })
            .collect::<Vec<_>>();
        let source = graph
            .trace_container_children
            .iter()
            .flat_map(|(parent, children)| {
                children
                    .iter()
                    .filter(|node| node.tala_id == target)
                    .map(|node| (*parent, node.position, node.rect.origin))
            })
            .collect::<Vec<_>>();
        eprintln!(
            "TRACE_STATE_TARGET_RUST phase={phase} root={root} node={target} stable={stable:?} active={:?} projected={projected:?} source={source:?}",
            graph
                .nodes
                .iter()
                .find(|node| node.tala_id == target)
                .map(|node| graph.active_node_position(node.input_id))
        );
    }
    let go_skips_singleton_placement_trace = matches!(
        phase,
        "after-child-recursion"
            | "after-nears"
            | "after-herds"
            | "after-split"
            | "before-combine"
            | "after-combine"
            | "after-orient"
            | "before-fit"
            | "after-fit-size"
            | "after-fit"
    ) || [
        "orthogonal-after-",
        "before-subgraph-",
        "after-orthogonal-",
        "after-cluster-optimize-",
        "after-trees-",
        "after-restore-edges-",
        "after-direct-owner-",
        "after-subgraph-",
    ]
    .iter()
    .any(|prefix| phase.starts_with(prefix));
    let visible_placement_nodes = graph
        .node_order
        .iter()
        .filter(|node| !graph.tree_routing_nodes.contains_key(node))
        .count();
    let go_takes_singleton_fast_path = graph.node_order.len() == 1
        || (!graph.tree_routing_nodes.is_empty() && visible_placement_nodes == 1);
    if go_takes_singleton_fast_path && go_skips_singleton_placement_trace {
        // D2 v0.9.0 returns from placeNodes before its first placement trace
        // when the owning graph has one node. Tree preprocessing retains the
        // removed descendants in Rust's stable arena, so count the remaining
        // placement nodes separately from those tree-routing aliases. Keep
        // these parent-scope callbacks out of the diagnostic stream while
        // retaining single-node snapshots emitted by an optimizer itself.
        return;
    }
    #[cfg(feature = "diagnostic-traces")]
    trace_remember_source_directions(graph);
    #[cfg(feature = "diagnostic-traces")]
    if phase == "initialized"
        && let Some(target) = crate::internal::trace_env_value("TALA_TRACE_PROJECTED_CHILD")
    {
        for (parent, children) in &graph.transaction_external_container_children {
            for child in children.iter().filter(|child| child.external_id == target) {
                eprintln!(
                    "PROJECTED_CHILD_IDENTITY_RUST root={root} parent={parent} child={:?} aggregates={:?} clusters={:?}",
                    (
                        child.tala_id,
                        child.scoring_is_aggregate_vessel,
                        child.rect,
                        child.position
                    ),
                    graph
                        .transaction_external_aggregate_children
                        .iter()
                        .map(|(id, members)| (
                            *id,
                            members
                                .iter()
                                .map(|n| (n.tala_id, &n.external_id))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>(),
                    graph
                        .clusters
                        .iter()
                        .map(|c| (
                            c.vessel_tala_id,
                            c.members
                                .iter()
                                .map(|id| graph.nodes[id.0 as usize].tala_id)
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    if phase == "after-fit" {
        let mut synced = graph.clone();
        synced.sync_nested_projected_container_children();
        #[cfg(feature = "diagnostic-traces")]
        {
            if let Some(root_tala_id) = synced
                .nodes
                .iter()
                .find(|node| node.external_id == root)
                .map(|node| node.tala_id)
                && let Ok(mut roots) = TRACE_SOURCE_CLUSTER_ROOTS
                    .get_or_init(|| std::sync::Mutex::new(BTreeSet::new()))
                    .lock()
            {
                roots.insert(root_tala_id);
            }
            let roots = TRACE_SOURCE_CLUSTER_ROOTS
                .get()
                .and_then(|roots| roots.lock().ok())
                .map(|roots| roots.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            for root_tala_id in roots {
                synced.sync_trace_source_cluster_children(root_tala_id);
            }
        }
        if let Ok(mut nodes) = TRACE_SYNCED_NODES
            .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
            .lock()
        {
            // `tracePlacement` starts from the owner's live Graph.Nodes
            // slice. FitToGraph has changed only this callback's root since
            // the preceding placement trace; preserve the cached shared
            // pointers for other nodes instead of replacing them with older
            // Rust value snapshots.
            let root_tala_id = graph
                .nodes
                .iter()
                .find(|node| node.external_id == root)
                .map(|node| node.tala_id);
            for node in &graph.nodes {
                if Some(node.tala_id) == root_tala_id {
                    nodes.insert(node.tala_id, node.clone());
                } else {
                    nodes.entry(node.tala_id).or_insert_with(|| node.clone());
                }
            }
            for node in synced
                .transaction_external_container_children
                .values()
                .flatten()
                .chain(synced.transaction_external_containers.iter())
                .chain(
                    synced
                        .transaction_external_aggregate_children
                        .values()
                        .flatten(),
                )
            {
                if node.position.is_some() {
                    match nodes.get_mut(&node.tala_id) {
                        // Graph.Nodes is the owner slice visited first by
                        // Go's tracePlacement. Keep its positioned geometry
                        // when a Rust transaction alias still carries an
                        // older value snapshot; use the alias only to fill a
                        // position that the stable owner has not published.
                        Some(owner) if owner.position.is_some() => {}
                        Some(owner) => *owner = node.clone(),
                        None => {
                            nodes.insert(node.tala_id, node.clone());
                        }
                    }
                }
            }
        }
    }
    if crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT_CONTAINERS") {
        eprintln!(
            "PLACEMENT_CONTAINERS_RUST {:?}",
            graph
                .containers
                .iter()
                .map(|(parent, children)| (
                    parent.map(|id| graph.nodes[id.0 as usize].tala_id),
                    children
                        .iter()
                        .map(|id| graph.nodes[id.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>()
        );
    }
    // The trace hierarchy is an immutable projection of the source graph,
    // while placement updates the value snapshots in the transaction maps.
    // Prefer the newest positioned alias when materializing a diagnostic
    // record; otherwise a child that was laid out by a recursive scope would
    // appear with its initial nil position even though Go's shared pointer has
    // already been updated by SyncNestedGeometry.
    fn trace_node_snapshot(
        graph: &ArenaGraph,
        tala_id: i64,
        fallback: &ArenaNode,
        _use_owner_frame_synced: bool,
        _prefer_live_owner: bool,
        use_synced: bool,
    ) -> ArenaNode {
        #[cfg(feature = "diagnostic-traces")]
        if _prefer_live_owner
            && let Some(node) = graph
                .nodes
                .iter()
                .find(|node| node.tala_id == tala_id && node.position.is_some())
        {
            // An owner callback can follow a child-fit operation that moved
            // the owner directly in this graph. The owner-frame cache only
            // receives nodes present in the completed child snapshot, so it
            // may lag for such ancestors. Prefer the live owner value when
            // this callback already carries it; the cache remains a fallback
            // for shared pointers omitted from the local graph view.
            return node.clone();
        }
        #[cfg(feature = "diagnostic-traces")]
        if _use_owner_frame_synced
            && let Some(nodes) = TRACE_OWNER_FRAME_NODES.get()
            && let Ok(nodes) = nodes.lock()
            && let Some(node) = nodes.get(&tala_id)
        {
            return node.clone();
        }
        if use_synced
            && let Some(nodes) = TRACE_SYNCED_NODES.get()
            && let Ok(nodes) = nodes.lock()
            && let Some(node) = nodes.get(&tala_id)
        {
            return node.clone();
        }
        graph
            .nodes
            .iter()
            .find(|node| node.tala_id == tala_id && node.position.is_some())
            .or_else(|| {
                graph
                    .transaction_external_container_children
                    .values()
                    .flatten()
                    .find(|node| node.tala_id == tala_id && node.position.is_some())
            })
            .or_else(|| {
                graph
                    .transaction_external_containers
                    .iter()
                    .find(|node| node.tala_id == tala_id && node.position.is_some())
            })
            .or_else(|| {
                graph
                    .transaction_external_aggregate_children
                    .values()
                    .flatten()
                    .find(|node| node.tala_id == tala_id && node.position.is_some())
            })
            .or_else(|| graph.nodes.iter().find(|node| node.tala_id == tala_id))
            .or_else(|| {
                graph
                    .transaction_external_container_children
                    .values()
                    .flatten()
                    .find(|node| node.tala_id == tala_id)
            })
            .or_else(|| {
                graph
                    .transaction_external_aggregate_children
                    .values()
                    .flatten()
                    .find(|node| node.tala_id == tala_id)
            })
            .cloned()
            .unwrap_or_else(|| fallback.clone())
    }
    fn trace_cluster_size(graph: &ArenaGraph, cluster_index: usize, _phase: &str) -> Size {
        let cluster = &graph.clusters[cluster_index];
        #[cfg(feature = "diagnostic-traces")]
        let refresh_after_optimization = _phase.starts_with("after-cluster-optimize-");
        #[cfg(feature = "diagnostic-traces")]
        if !refresh_after_optimization
            && let Some(size) = TRACE_CLUSTER_INITIAL_SIZES
                .get()
                .and_then(|sizes| sizes.lock().ok())
                .and_then(|sizes| sizes.get(&cluster.vessel_tala_id).copied())
        {
            return size;
        }
        let mut maximum = Size::default();
        for member in &cluster.members {
            let tala_id = graph.nodes[member.0 as usize].tala_id;
            #[cfg(feature = "diagnostic-traces")]
            let live_optimized_size =
                refresh_after_optimization.then_some(graph.nodes[member.0 as usize].rect.size);
            #[cfg(not(feature = "diagnostic-traces"))]
            let live_optimized_size = None;
            let size = live_optimized_size
                .or_else(|| {
                    graph
                        .trace_container_children
                        .values()
                        .flatten()
                        .find(|node| node.tala_id == tala_id)
                        .map(|node| node.rect.size)
                })
                .unwrap_or_else(|| graph.nodes[member.0 as usize].rect.size);
            maximum.width = maximum.width.max(size.width);
            maximum.height = maximum.height.max(size.height);
        }
        let gaps = cluster.padding * (cluster.members.len().saturating_sub(1) as f64);
        let size = match cluster.arrangement {
            ClusterArrangement::Row => Size {
                width: maximum.width * cluster.members.len() as f64 + gaps,
                height: maximum.height,
            },
            ClusterArrangement::Column => Size {
                width: maximum.width,
                height: maximum.height * cluster.members.len() as f64 + gaps,
            },
        };
        #[cfg(feature = "diagnostic-traces")]
        if refresh_after_optimization
            && let Ok(mut sizes) = TRACE_CLUSTER_INITIAL_SIZES
                .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
                .lock()
        {
            sizes.insert(cluster.vessel_tala_id, size);
        }
        size
    }
    fn trace_cluster_expanded_size(graph: &ArenaGraph, cluster_index: usize) -> Option<Size> {
        let cluster = &graph.clusters[cluster_index];
        let member_ids = cluster
            .members
            .iter()
            .map(|member| graph.nodes[member.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        let (vessel_id, children) =
            graph
                .transaction_external_aggregate_children
                .iter()
                .find(|(_, children)| {
                    children.len() == member_ids.len()
                        && children
                            .iter()
                            .all(|child| member_ids.contains(&child.tala_id))
                })?;
        let layout = graph
            .transaction_external_cluster_layouts
            .get(vessel_id)
            .copied()
            .unwrap_or(ExternalClusterLayout {
                arrangement: cluster.arrangement,
                padding: cluster.padding,
                fixed_size: cluster.fixed_size,
            });
        let gaps = layout.padding * children.len().saturating_sub(1) as f64;
        let maximum = children
            .iter()
            .fold(Size::default(), |maximum, child| Size {
                width: maximum.width.max(child.rect.size.width),
                height: maximum.height.max(child.rect.size.height),
            });
        Some(match layout.arrangement {
            ClusterArrangement::Row => Size {
                width: maximum.width * children.len() as f64 + gaps,
                height: maximum.height,
            },
            ClusterArrangement::Column => Size {
                width: maximum.width,
                height: maximum.height * children.len() as f64 + gaps,
            },
        })
    }
    // Keep the event shape identical to the additive Go trace in
    // `internal/placement/node_placement.go`. The active node/edge views are
    // used here because the arena retains extracted tree entities after Go's
    // graph slices have removed them.
    // Go's root trace iterates the current Graph.Nodes slice, whose order
    // changes as subgraphs are combined and aggregate vessels are restored.
    // The arena's node_order is that slice's stable-ID projection; falling
    // back to dense input order would replay the original source order after
    // a later packing/cleanup pass.
    let owner_snapshot = phase.starts_with("after-orthogonal-")
        || phase.starts_with("after-cluster-optimize-")
        || phase.starts_with("after-trees-")
        || phase.starts_with("after-direct-owner-")
        || phase.starts_with("after-subgraph-")
        || matches!(
            phase,
            "after-orient" | "before-combine" | "after-combine" | "before-fit" | "after-fit-size"
        );
    // These Go hooks receive root.Graph, while other placement snapshots
    // receive the owning `g` directly. Keep their source-parent projection
    // separate from owner snapshots: applying that projection to later
    // owner phases drops live nodes that Go keeps in g.
    let root_graph_snapshot = matches!(
        phase,
        "orthogonal-start"
            | "orthogonal-after-initialize"
            | "orthogonal-after-sizeless"
            | "orthogonal-after-transition-compaction"
            | "orthogonal-after-sized"
            | "orthogonal-after-optimized"
    );
    #[cfg(feature = "diagnostic-traces")]
    if root_graph_snapshot {
        let geometry = |node: &ArenaNode| {
            let position = node
                .position
                .map(|point| (point.x.to_bits(), point.y.to_bits()));
            (
                position,
                (node.rect.origin.x.to_bits(), node.rect.origin.y.to_bits()),
                (
                    node.rect.size.width.to_bits(),
                    node.rect.size.height.to_bits(),
                ),
            )
        };
        let current = graph
            .nodes
            .iter()
            .map(|node| (node.tala_id, geometry(node)))
            .collect::<BTreeMap<_, _>>();
        if phase == "orthogonal-start" {
            if let Ok(mut baselines) = TRACE_ROOT_GRAPH_BASELINES
                .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
                .lock()
            {
                baselines.insert(root.to_owned(), current);
            }
        } else if phase.starts_with("orthogonal-after-") {
            let baseline = TRACE_ROOT_GRAPH_BASELINES
                .get()
                .and_then(|baselines| baselines.lock().ok())
                .and_then(|baselines| baselines.get(root).cloned());
            if let Some(baseline) = baseline
                && let Ok(mut synced) = TRACE_SYNCED_NODES
                    .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
                    .lock()
            {
                // Go's optimizer mutates shared Node pointers. Propagate only
                // geometry changed since this root graph's orthogonal-start
                // callback; unrelated owner nodes may still be local Rust
                // snapshots from before recursive placement.
                for node in &graph.nodes {
                    if baseline.get(&node.tala_id) != Some(&geometry(node)) {
                        synced.insert(node.tala_id, node.clone());
                    }
                }
            }
        }
    }
    let mut trace_nodes = if graph.node_order.is_empty() {
        graph
            .nodes
            .iter()
            .map(|node| node.input_id)
            .collect::<Vec<_>>()
    } else {
        graph.node_order.clone()
    };
    #[cfg(feature = "diagnostic-traces")]
    if phase == "placeNodes-entry" {
        // `placeNodes` recurses with the same owning Graph and changes only
        // the root argument; every entry therefore walks that graph's one
        // pre-recursion node slice.
        let owner_tala_id = 0;
        let root_order = {
            graph
                .trace_root_owner_node_orders
                .lock()
                .ok()
                .and_then(|roots| roots.get(&owner_tala_id).cloned())
        };
        if std::env::var_os("D2_RS_TRACE_OWNER_GRAPH").is_some() {
            let saved = root_order
                .as_ref()
                .map(|order| order.iter().copied().collect::<BTreeSet<_>>())
                .unwrap_or_default();
            let later_only = graph
                .node_order
                .iter()
                .filter(|node_id| !saved.contains(&graph.nodes[node_id.0 as usize].tala_id))
                .map(|node_id| graph.nodes[node_id.0 as usize].external_id.clone())
                .collect::<Vec<_>>();
            let tree_nodes = graph
                .tree_routing_nodes
                .keys()
                .filter(|node_id| !saved.contains(&graph.nodes[node_id.0 as usize].tala_id))
                .map(|node_id| graph.nodes[node_id.0 as usize].external_id.clone())
                .collect::<Vec<_>>();
            eprintln!(
                "D2_RS_TRACE_PLACE_NODES_ENTRY root={root:?} owner_tala_id={owner_tala_id:?} saved={} live={} later_only={later_only:?} tree_nodes={tree_nodes:?}",
                root_order.as_ref().map_or(0, Vec::len),
                graph.node_order.len(),
            );
        }
        if let Some(root_order) = root_order {
            let by_tala_id = graph
                .nodes
                .iter()
                .enumerate()
                .map(|(index, node)| (node.tala_id, NodeId(index as u32)))
                .collect::<BTreeMap<_, _>>();
            if let Some(resolved) = root_order
                .iter()
                .map(|tala_id| by_tala_id.get(tala_id).copied())
                .collect::<Option<Vec<_>>>()
            {
                trace_nodes = resolved;
            }
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    let mut root_owner_snapshot_applied = false;
    #[cfg(not(feature = "diagnostic-traces"))]
    let root_owner_snapshot_applied = false;
    #[cfg(feature = "diagnostic-traces")]
    if root_graph_snapshot
        && let Some(root_tala_id) = graph
            .nodes
            .iter()
            .find(|node| node.external_id == root)
            .map(|node| node.tala_id)
    {
        let owner_order = graph
            .trace_root_owner_node_orders
            .lock()
            .ok()
            .and_then(|owners| owners.get(&root_tala_id).cloned())
            .or_else(|| {
                graph
                    .trace_owner_node_orders
                    .lock()
                    .ok()
                    .and_then(|owners| owners.get(&root_tala_id).cloned())
            });
        if let Some(owner_order) = owner_order {
            let by_tala_id = graph
                .nodes
                .iter()
                .enumerate()
                .map(|(index, node)| (node.tala_id, NodeId(index as u32)))
                .collect::<BTreeMap<_, _>>();
            let resolved = owner_order
                .iter()
                .map(|tala_id| by_tala_id.get(tala_id).copied())
                .collect::<Option<Vec<_>>>();
            if let Some(resolved) = resolved {
                trace_nodes = resolved;
                root_owner_snapshot_applied = true;
            }
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    if root_graph_snapshot && std::env::var_os("D2_RS_TRACE_OWNER_GRAPH").is_some() {
        let labels = |order: &[NodeId]| {
            order
                .iter()
                .map(|node_id| graph.nodes[node_id.0 as usize].external_id.clone())
                .collect::<Vec<_>>()
        };
        eprintln!(
            "D2_RS_TRACE_OWNER_GRAPH phase={} root={:?} selected={:?} mapped={} active={:?} logical={:?}",
            phase,
            root,
            labels(&trace_nodes),
            root_owner_snapshot_applied,
            labels(&graph.active_graph_node_order),
            labels(&graph.graph_node_order())
        );
    }
    if matches!(phase, "before-fit" | "after-fit-size" | "after-fit") {
        // PlaceTrees restores retained tree nodes to the owning Graph before
        // the fit callback. Some nested arenas keep those nodes in the tree
        // routing index rather than the owner's direct node_order, so project
        // the restored entries into these later owner snapshots.
        let owner_parent = trace_owner_scope_parent_tala_id(graph, root);
        for (&node_id, tree) in &graph.tree_routing_nodes {
            if !graph.restored_tree_edges.contains(&tree.sentinel_edge) {
                continue;
            }
            let node = &graph.nodes[node_id.0 as usize];
            let in_owner_scope = root.is_empty()
                || owner_parent
                    .is_some_and(|parent| trace_node_in_owner_scope(graph, node.tala_id, parent));
            if in_owner_scope && !trace_nodes.contains(&node_id) {
                trace_nodes.push(node_id);
            }
        }
    }
    let root_is_sequence_diagram = graph
        .nodes
        .iter()
        .find(|node| node.external_id == root)
        .is_some_and(|node| node.is_sequence_diagram);
    let root_retains_pre_aggregate_owner = graph
        .nodes
        .iter()
        .find(|node| node.external_id == root)
        .is_some_and(|node| trace_node_is_hidden_by_active_aggregate(graph, node.input_id));
    if root_graph_snapshot
        && !root_owner_snapshot_applied
        && !root_is_sequence_diagram
        && !root_retains_pre_aggregate_owner
    {
        // A sequence-diagram scope can retain the original top-level owner
        // graph for its own placement callback. Other nested callbacks use
        // the owner of their containing scope; active aggregate members and
        // descendants below those members are absent from that live graph.
        if let Some(owner_parent) = trace_owner_scope_parent_tala_id(graph, root) {
            trace_nodes.retain(|node_id| {
                trace_node_in_owner_scope(
                    graph,
                    graph.nodes[node_id.0 as usize].tala_id,
                    owner_parent,
                )
            });
        }
        trace_nodes.retain(|node_id| !trace_node_is_hidden_by_active_aggregate(graph, *node_id));
    }
    #[cfg(feature = "diagnostic-traces")]
    if matches!(phase, "before-fit" | "after-fit-size" | "after-fit")
        && let Some(owner_parent) = trace_owner_scope_parent_tala_id(graph, root)
    {
        let remembered = TRACE_OWNER_VISIBLE_NODE_IDS
            .get()
            .and_then(|nodes| nodes.lock().ok())
            .and_then(|nodes| nodes.get(&owner_parent).cloned())
            .unwrap_or_default();
        for tala_id in remembered {
            if let Some((index, _)) = graph
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| node.tala_id == tala_id)
            {
                let node_id = NodeId(index as u32);
                if !trace_nodes.contains(&node_id) {
                    trace_nodes.push(node_id);
                }
            }
        }
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    if phase == "after-fit" && !trace_nodes.is_empty() {
        // Go's after-fit callback walks the owning Graph.Nodes slice after
        // tree restoration. Reconstruct that slice as a source-hierarchy
        // DFS, preferring transaction child order where restored tree nodes
        // were reinserted (the arena node_order may still leave them at the
        // end of the dense vector).
        fn children_for(graph: &ArenaGraph, parent_tala_id: i64) -> Option<&[ArenaNode]> {
            graph
                .transaction_external_container_children
                .get(&parent_tala_id)
                .map(Vec::as_slice)
                .or_else(|| {
                    graph
                        .trace_container_children
                        .get(&parent_tala_id)
                        .map(Vec::as_slice)
                })
        }
        fn visit_source_order(
            tala_id: i64,
            graph: &ArenaGraph,
            by_tala: &BTreeMap<i64, NodeId>,
            visible_tala_ids: &BTreeSet<i64>,
            seen: &mut BTreeSet<i64>,
            output: &mut Vec<NodeId>,
        ) {
            if !seen.insert(tala_id) {
                return;
            }
            if visible_tala_ids.contains(&tala_id)
                && let Some(&node_id) = by_tala.get(&tala_id)
            {
                output.push(node_id);
            }
            if let Some(child_tala_ids) = trace_source_child_ids(graph, tala_id) {
                for child_tala_id in child_tala_ids {
                    visit_source_order(
                        child_tala_id,
                        graph,
                        by_tala,
                        visible_tala_ids,
                        seen,
                        output,
                    );
                }
                return;
            }
            if let Some(children) = children_for(graph, tala_id) {
                for child in children {
                    visit_source_order(
                        child.tala_id,
                        graph,
                        by_tala,
                        visible_tala_ids,
                        seen,
                        output,
                    );
                }
            }
        }
        let by_tala = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.tala_id, NodeId(index as u32)))
            .collect::<BTreeMap<_, _>>();
        let visible_tala_ids = trace_nodes
            .iter()
            .map(|node_id| graph.nodes[node_id.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        let mut child_ids = BTreeSet::new();
        for parent_tala_id in graph
            .transaction_external_container_children
            .keys()
            .chain(graph.trace_container_children.keys())
            .chain(graph.source_container_child_order.keys())
            .copied()
        {
            if let Some(source_children) = trace_source_child_ids(graph, parent_tala_id) {
                child_ids.extend(source_children.iter().copied());
            } else if let Some(children) = children_for(graph, parent_tala_id) {
                child_ids.extend(children.iter().map(|child| child.tala_id));
            }
        }
        let mut roots = Vec::new();
        for node_id in &trace_nodes {
            let tala_id = graph.nodes[node_id.0 as usize].tala_id;
            if !child_ids.contains(&tala_id) {
                roots.push(tala_id);
            }
        }
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::with_capacity(trace_nodes.len());
        for root_tala_id in roots {
            visit_source_order(
                root_tala_id,
                graph,
                &by_tala,
                &visible_tala_ids,
                &mut seen,
                &mut ordered,
            );
        }
        for node_id in &trace_nodes {
            let tala_id = graph.nodes[node_id.0 as usize].tala_id;
            if seen.insert(tala_id) {
                ordered.push(*node_id);
            }
        }
        if ordered.len() == trace_nodes.len() {
            trace_nodes = ordered;
        }
        // Tree-restored aliases can still be reported as synthetic roots when
        // their source parent metadata was stripped.  Keep dotted aliases
        // with the first matching top-level owner in the current Graph.Nodes
        // order (stable sort preserves child order within that group).
        let prefix_ranks = trace_nodes
            .iter()
            .enumerate()
            .filter_map(|(rank, node_id)| {
                let external_id = graph.nodes[node_id.0 as usize].external_id.as_str();
                (!external_id.contains('.')).then_some((external_id.to_owned(), rank))
            })
            .collect::<BTreeMap<_, _>>();
        trace_nodes.stable_sort_by_key(|node_id| {
            let external_id = graph.nodes[node_id.0 as usize].external_id.as_str();
            let owner = external_id.split('.').next().unwrap_or(external_id);
            prefix_ranks.get(owner).copied().unwrap_or(usize::MAX)
        });
    }
    #[cfg(feature = "diagnostic-traces")]
    let recursive_owner_walk_snapshot = matches!(
        phase,
        "after-child-recursion" | "after-nears" | "after-herds" | "after-split"
    ) || phase.starts_with("before-subgraph-");
    #[cfg(feature = "diagnostic-traces")]
    let owner_graph_walk_snapshot = owner_snapshot
        && phase != "sized-transition"
        && phase != "optimized"
        && !phase.starts_with("sized-")
        || phase == "after-fit"
        || phase.starts_with("after-restore-edges-");
    #[cfg(feature = "diagnostic-traces")]
    let owner_container_walk_applied = matches!(phase, "placeNodes-entry" | "after-fit")
        || recursive_owner_walk_snapshot
        || owner_graph_walk_snapshot
        || root_graph_snapshot
        || phase.starts_with("after-trees-")
        || phase.starts_with("after-restore-edges-");
    #[cfg(not(feature = "diagnostic-traces"))]
    let owner_container_walk_applied = false;
    #[cfg(feature = "diagnostic-traces")]
    if owner_container_walk_applied && !trace_nodes.is_empty() {
        // Go's tracePlacement walks each live Graph.Nodes entry, then its
        // current Containers children. Reproduce that callback order from
        // the post-tree child slices and active cluster vessels.
        trace_nodes = trace_owner_container_walk_order(
            graph,
            &trace_nodes,
            true,
            phase,
            graph
                .nodes
                .iter()
                .find(|node| node.external_id == root)
                .map(|node| node.tala_id),
            root.is_empty() || root == "top",
        );
    }
    if root.is_empty() && phase == "initialized" && !trace_nodes.is_empty() {
        // The global Go placement callback receives the current source graph
        // after a nested compound has been restored. Its Nodes slice is
        // followed by Containers traversal, so a restored container's
        // descendants appear immediately after that container even when the
        // arena keeps the descendants at their allocation positions. Project
        // this one lifecycle boundary through the durable source hierarchy;
        // ordinary placement and all later geometry remain untouched.
        fn append_source_order(
            tala_id: i64,
            graph: &ArenaGraph,
            by_tala: &BTreeMap<i64, NodeId>,
            seen: &mut BTreeSet<i64>,
            output: &mut Vec<NodeId>,
        ) {
            if !seen.insert(tala_id) {
                return;
            }
            if let Some(&node_id) = by_tala.get(&tala_id) {
                output.push(node_id);
            }
            if let Some(mut child_ids) = trace_source_child_ids(graph, tala_id) {
                trace_append_cluster_vessels(graph, &mut child_ids);
                for child_tala_id in child_ids {
                    append_source_order(child_tala_id, graph, by_tala, seen, output);
                }
                return;
            }
            if let Some(children) = graph
                .trace_container_children
                .get(&tala_id)
                .or_else(|| graph.transaction_external_container_children.get(&tala_id))
            {
                let child_ids = children.iter().map(|child| child.tala_id);
                for child_tala_id in child_ids {
                    append_source_order(child_tala_id, graph, by_tala, seen, output);
                }
            }
        }
        let by_tala = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.tala_id, NodeId(index as u32)))
            .collect::<BTreeMap<_, _>>();
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::with_capacity(trace_nodes.len());
        let mut root_children = trace_nodes
            .iter()
            .copied()
            .filter(|node_id| !graph.nodes[node_id.0 as usize].external_id.contains('.'))
            .collect::<Vec<_>>();
        if root_children.is_empty() {
            root_children = graph
                .containers
                .get(&None)
                .or_else(|| {
                    graph.trace_container_children.get(&0).map(|children| {
                        children
                            .iter()
                            .filter_map(|child| by_tala.get(&child.tala_id).copied())
                            .collect::<Vec<_>>()
                    })
                })
                .unwrap_or_default();
        }
        for child in root_children {
            let tala_id = graph.nodes[child.0 as usize].tala_id;
            append_source_order(tala_id, graph, &by_tala, &mut seen, &mut ordered);
        }
        // The source root's direct Nodes order is authoritative for siblings;
        // include any retained arena entries that are not represented in the
        // container map after a compound has been restored.
        if ordered.is_empty() {
            for node_id in &trace_nodes {
                let tala_id = graph.nodes[node_id.0 as usize].tala_id;
                append_source_order(tala_id, graph, &by_tala, &mut seen, &mut ordered);
            }
        }
        for node_id in &trace_nodes {
            let tala_id = graph.nodes[node_id.0 as usize].tala_id;
            if seen.insert(tala_id) {
                ordered.push(*node_id);
            }
        }
        if ordered.len() == trace_nodes.len() {
            trace_nodes = ordered;
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    if (phase.starts_with("after-direct-owner-") || phase.starts_with("after-subgraph-"))
        && let Some(owner_parent) = trace_owner_scope_parent_tala_id(graph, root)
        && let Ok(mut visible) = TRACE_OWNER_VISIBLE_NODE_IDS
            .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
            .lock()
    {
        visible.entry(owner_parent).or_default().extend(
            trace_nodes
                .iter()
                .map(|node_id| graph.nodes[node_id.0 as usize].tala_id),
        );
    }
    if crate::internal::trace_env_enabled("TALA_TRACE_NODE_ORDER") {
        eprintln!(
            "NODE_ORDER_RUST root={} phase={} ids={:?}",
            root,
            phase,
            trace_nodes
                .iter()
                .map(|id| graph.nodes[id.0 as usize].tala_id)
                .collect::<Vec<_>>()
        );
        if root == "2043751575" && phase == "initialized" {
            eprintln!(
                "TRACE_REACHABLE_RUST {:?}",
                graph.reachable_container_tala_ids
            );
        }
    }
    let mut nodes = Vec::new();
    let mut seen = BTreeSet::new();
    #[cfg(feature = "diagnostic-traces")]
    let mut visible_owners = trace_nodes
        .iter()
        .map(|node_id| graph.nodes[node_id.0 as usize].tala_id)
        .collect::<BTreeSet<_>>();
    #[cfg(not(feature = "diagnostic-traces"))]
    let visible_owners = trace_nodes
        .iter()
        .map(|node_id| graph.nodes[node_id.0 as usize].tala_id)
        .collect::<BTreeSet<_>>();
    #[cfg(feature = "diagnostic-traces")]
    if root.is_empty() && phase == "initialized" {
        // Go's root placement callback starts with the owning Graph.Nodes
        // slice, then recursively visits every node in its live Containers
        // map. The arena's root order contains only direct nodes, so include
        // exactly the currently reachable live children in the visit set;
        // otherwise nested children and cluster vessels get appended later
        // by the source fallback, after unrelated root siblings.
        let mut pending = trace_nodes
            .iter()
            .map(|node_id| graph.nodes[node_id.0 as usize].tala_id)
            .collect::<Vec<_>>();
        while let Some(parent_tala_id) = pending.pop() {
            let is_active_aggregate_carrier = graph.clusters.iter().any(|cluster| {
                graph.cluster_is_active(cluster)
                    && cluster.members.first().is_some_and(|member| {
                        graph.nodes[member.0 as usize].tala_id == parent_tala_id
                    })
            }) || graph.sequences.iter().any(|sequence| {
                graph.sequence_is_active(sequence)
                    && sequence.members.first().is_some_and(|member| {
                        graph.nodes[member.0 as usize].tala_id == parent_tala_id
                    })
            });
            if is_active_aggregate_carrier {
                continue;
            }
            let children = trace_live_owner_child_ids(graph, parent_tala_id, false)
                .unwrap_or_else(|| trace_child_tala_ids(graph, parent_tala_id));
            for child_tala_id in children {
                if visible_owners.insert(child_tala_id) {
                    pending.push(child_tala_id);
                }
            }
        }
    }
    let after_fit_root = (phase == "after-fit")
        .then(|| {
            graph
                .nodes
                .iter()
                .find(|node| node.external_id == root)
                .map(|node| node.input_id)
        })
        .flatten();
    fn trace_aggregate_ancestor_offset(
        parent_tala_id: i64,
        graph: &ArenaGraph,
        use_synced: bool,
        after_fit_root_tala: Option<i64>,
    ) -> Point {
        if !use_synced {
            return Point::default();
        }
        let mut current = Some(parent_tala_id);
        let mut seen = BTreeSet::new();
        let mut offset = Point::default();
        while let Some(tala_id) = current {
            if !seen.insert(tala_id) {
                break;
            }
            for (cluster_index, cluster) in graph.clusters.iter().enumerate() {
                if !cluster
                    .members
                    .iter()
                    .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
                {
                    continue;
                }
                let mut position = graph
                    .pending_cluster_vessel_positions
                    .get(&cluster_index)
                    .copied()
                    .or_else(|| {
                        cluster
                            .members
                            .first()
                            .and_then(|member| graph.active_node_position(*member))
                    });
                if let Some(root_tala_id) = after_fit_root_tala
                    && let Some(member_tala_id) = cluster
                        .members
                        .first()
                        .map(|member| graph.nodes[member.0 as usize].tala_id)
                    && let Some(parent_tala_id) = graph
                        .source_parent_tala_by_tala
                        .get(&member_tala_id)
                        .copied()
                        .flatten()
                    && parent_tala_id != root_tala_id
                    && graph.source_is_descendant_of_tala(parent_tala_id, root_tala_id)
                {
                    let parent_position = graph
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == parent_tala_id)
                        .and_then(|parent| {
                            TRACE_SYNCED_NODES
                                .get()
                                .and_then(|synced| synced.lock().ok())
                                .and_then(|synced| synced.get(&parent.tala_id).cloned())
                                .and_then(|parent| parent.position)
                                .or_else(|| graph.active_node_position(parent.input_id))
                        })
                        .unwrap_or_default();
                    let padding = graph.trace_source_container_padding(parent_tala_id);
                    position = position.map(|point| Point {
                        x: point.x + parent_position.x + padding.left,
                        y: point.y + parent_position.y + padding.top,
                    });
                }
                if let Some(current_position) = position {
                    if current_position == Point::default() {
                        position = TRACE_CLUSTER_TRANSITION_POSITIONS
                            .get()
                            .and_then(|positions| positions.lock().ok())
                            .and_then(|positions| positions.get(&cluster.vessel_tala_id).copied())
                            .or(Some(current_position));
                    } else if let Ok(mut positions) = TRACE_CLUSTER_TRANSITION_POSITIONS
                        .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
                        .lock()
                    {
                        positions.insert(cluster.vessel_tala_id, current_position);
                    }
                }
                if let Some(position) = position {
                    offset.x += position.x;
                    offset.y += position.y;
                }
            }
            for sequence in &graph.sequences {
                if !sequence
                    .members
                    .iter()
                    .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
                {
                    continue;
                }
                if let Some(position) = sequence
                    .members
                    .first()
                    .and_then(|member| graph.active_node_position(*member))
                {
                    offset.x += position.x;
                    offset.y += position.y;
                }
            }
            current = graph
                .source_parent_tala_by_tala
                .get(&tala_id)
                .copied()
                .flatten();
        }
        offset
    }
    fn trace_cluster_owner_offset(
        graph: &ArenaGraph,
        parent_tala_id: i64,
        snapshot_root: &str,
    ) -> Point {
        let snapshot_root_tala_id = graph
            .nodes
            .iter()
            .find(|node| node.external_id == snapshot_root)
            .map(|node| node.tala_id);
        let mut current = Some(parent_tala_id);
        let mut seen = BTreeSet::new();
        let mut offset = Point::default();
        while let Some(parent_tala_id) = current {
            if Some(parent_tala_id) == snapshot_root_tala_id || !seen.insert(parent_tala_id) {
                break;
            }
            let position = TRACE_SYNCED_NODES
                .get()
                .and_then(|synced| synced.lock().ok())
                .and_then(|synced| synced.get(&parent_tala_id).cloned())
                .and_then(|parent| parent.position)
                .or_else(|| {
                    graph
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == parent_tala_id)
                        .and_then(|parent| parent.position)
                })
                .or_else(|| {
                    graph
                        .transaction_external_containers
                        .iter()
                        .find(|parent| parent.tala_id == parent_tala_id)
                        .and_then(|parent| parent.position)
                })
                .unwrap_or_default();
            let padding = graph.trace_source_container_padding(parent_tala_id);
            offset.x += position.x + padding.left;
            offset.y += position.y + padding.top;
            current = graph
                .source_parent_tala_by_tala
                .get(&parent_tala_id)
                .copied()
                .flatten();
        }
        offset
    }
    fn trace_container_is_nested(graph: &ArenaGraph, tala_id: i64) -> bool {
        graph
            .source_parent_tala_by_tala
            .get(&tala_id)
            .copied()
            .flatten()
            .is_some()
            || graph
                .nodes
                .iter()
                .find(|node| node.tala_id == tala_id)
                .is_some_and(|node| node.container.is_some())
    }
    fn visit(
        node_id: NodeId,
        _phase: &str,
        snapshot_root: &str,
        graph: &ArenaGraph,
        phase_subgraph_index: Option<usize>,
        sequence_component_indices_by_tala: &BTreeMap<i64, usize>,
        seen: &mut BTreeSet<NodeId>,
        nodes: &mut Vec<PlacementTraceNode>,
        after_fit_root: Option<NodeId>,
        project_inactive_aggregates: bool,
        visible_owners: &BTreeSet<i64>,
        recurse_children: bool,
        owner_snapshot: bool,
        root_graph_snapshot: bool,
        _owner_changes: Option<&BTreeSet<i64>>,
    ) {
        if !seen.insert(node_id) {
            return;
        }
        let node = &graph.nodes[node_id.0 as usize];
        // Induced pre-fit scopes keep aggregate members in the arena only as
        // copyback aliases.  Go's visible Graph.Nodes slice has already
        // replaced those members with one transaction vessel; skip the
        // aliases here and let the projection pass emit that vessel record.
        if after_fit_root.is_none()
            && graph
                .transaction_external_aggregate_children
                .values()
                .any(|members| members.iter().any(|member| member.tala_id == node.tala_id))
        {
            if node.tala_id == 3946625030
                && crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_MAP")
            {
                eprintln!(
                    "VISIT_SKIP_AGG_RUST node={} txn_maps={}",
                    node.tala_id,
                    graph.transaction_external_aggregate_children.len()
                );
            }
            return;
        }
        // A Go placement graph replaces an active cluster's member slice with
        // one vessel pointer before the `after-fit` callback.  The stable
        // arena retains every member ID, so project that pointer lifecycle in
        // the diagnostic trace.  In particular, do not walk the retained
        // member container map here: those aliases are not Graph.Nodes in the
        // upstream callback and would shift the event order and geometry.
        if node.tala_id == 3946625030
            && crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_OWNER")
        {
            let parent = graph
                .source_parent_tala_by_tala
                .get(&node.tala_id)
                .copied()
                .flatten();
            eprintln!(
                "CLUSTER_OWNER_RUST node={} parent={:?} visible={:?} after_fit={:?}",
                node.tala_id, parent, visible_owners, after_fit_root
            );
        }
        if let Some((cluster_index, cluster)) =
            graph.clusters.iter().enumerate().find(|(_, cluster)| {
                cluster.members.contains(&node_id)
                    && (project_inactive_aggregates
                        || graph.cluster_is_active(cluster)
                        // Nested after-fit callbacks observe the completed
                        // aggregate through the owning graph even after its
                        // induced scope has marked the cluster inactive.
                        || after_fit_root.is_some()
                        || cluster.members.iter().any(|member| {
                            let member_tala_id = graph.nodes[member.0 as usize].tala_id;
                            graph
                                .source_parent_tala_by_tala
                                .get(&member_tala_id)
                                .copied()
                                .flatten()
                                .is_some_and(|parent| visible_owners.contains(&parent))
                        }))
            })
        {
            let Some(&carrier) = cluster.members.first() else {
                return;
            };
            if node_id != carrier {
                return;
            }
            for member in &cluster.members {
                seen.insert(*member);
            }
            // The stable arena uses the first cluster member as the vessel's
            // carrier. Its own position is not the synthetic Go vessel's
            // TopLeft; only vessel-keyed projections preserve whether that
            // pointer is positioned at this callback.
            let mut position = graph
                .transaction_external_container_children
                .values()
                .flatten()
                .find(|node| node.tala_id == cluster.vessel_tala_id)
                .and_then(|node| node.position)
                .or_else(|| {
                    graph
                        .transaction_external_aggregate_children
                        .values()
                        .flatten()
                        .find(|node| node.tala_id == cluster.vessel_tala_id)
                        .and_then(|node| node.position)
                })
                .or_else(|| {
                    graph
                        .transaction_external_containers
                        .iter()
                        .find(|node| node.tala_id == cluster.vessel_tala_id)
                        .and_then(|node| node.position)
                })
                .or_else(|| graph.projected_node_position_by_tala(cluster.vessel_tala_id))
                .or_else(|| {
                    graph
                        .pending_cluster_vessel_positions
                        .get(&cluster_index)
                        .copied()
                });
            #[cfg(feature = "diagnostic-traces")]
            if crate::internal::trace_env_value("TALA_TRACE_CLUSTER_PROJECTION")
                .is_some_and(|target| target == cluster.vessel_tala_id.to_string())
                && matches!(
                    _phase,
                    "orthogonal-after-initialize"
                        | "orthogonal-after-sizeless"
                        | "orthogonal-after-transition-compaction"
                        | "orthogonal-after-sized"
                        | "orthogonal-after-optimized"
                )
            {
                let external_positions = graph
                    .transaction_external_container_children
                    .values()
                    .flatten()
                    .chain(
                        graph
                            .transaction_external_aggregate_children
                            .values()
                            .flatten(),
                    )
                    .chain(graph.transaction_external_containers.iter())
                    .filter(|node| node.tala_id == cluster.vessel_tala_id)
                    .map(|node| node.position)
                    .collect::<Vec<_>>();
                let member_positions = cluster
                    .members
                    .iter()
                    .map(|member| {
                        let member = &graph.nodes[member.0 as usize];
                        (member.tala_id, member.position, member.rect.size)
                    })
                    .collect::<Vec<_>>();
                eprintln!(
                    "CLUSTER_TRACE_PROJECTION_RUST phase={} root={} vessel={} selected={:?} pending={:?} external={:?} projected={:?} active_carrier={:?} bounds={:?} members={:?}",
                    _phase,
                    snapshot_root,
                    cluster.vessel_tala_id,
                    position,
                    graph.pending_cluster_vessel_positions.get(&cluster_index),
                    external_positions,
                    graph.projected_node_position_by_tala(cluster.vessel_tala_id),
                    graph.active_node_position(carrier),
                    graph.cluster_bounds(&cluster.members),
                    member_positions,
                );
            }
            let root_tala_id =
                after_fit_root.map(|root_id| graph.nodes[root_id.0 as usize].tala_id);
            let carrier_tala_id = cluster
                .members
                .first()
                .map(|member| graph.nodes[member.0 as usize].tala_id);
            let snapshot_root_tala_id = graph
                .nodes
                .iter()
                .find(|node| node.external_id == snapshot_root)
                .map(|node| node.tala_id);
            #[cfg(feature = "diagnostic-traces")]
            let owner_scope_completed_before_snapshot_root =
                match (snapshot_root_tala_id, carrier_tala_id) {
                    (Some(root_tala_id), Some(carrier_tala_id)) => graph
                        .source_parent_tala_by_tala
                        .get(&carrier_tala_id)
                        .copied()
                        .flatten()
                        .is_some_and(|parent_tala_id| {
                            parent_tala_id != root_tala_id
                                && !graph.source_is_descendant_of_tala(parent_tala_id, root_tala_id)
                                && trace_completed_scope_precedes(
                                    graph,
                                    parent_tala_id,
                                    root_tala_id,
                                )
                                .unwrap_or(false)
                        }),
                    _ => false,
                };
            #[cfg(not(feature = "diagnostic-traces"))]
            let owner_scope_completed_before_snapshot_root = root_tala_id.is_some();
            let top_owner_recursion_snapshot = matches!(snapshot_root, "" | "top")
                && (matches!(
                    _phase,
                    "after-child-recursion" | "after-nears" | "after-herds" | "after-split"
                ) || _phase.starts_with("before-subgraph-"));
            // These owner callbacks serialize nested vessels in the owner
            // snapshot's coordinate frame. Recursive child-placement
            // callbacks have already published the child scope into its
            // parent, while the later fit callbacks read synced positions.
            if (top_owner_recursion_snapshot || matches!(_phase, "before-fit" | "after-fit-size"))
                && after_fit_root.is_none()
                && let Some(carrier_tala_id) = carrier_tala_id
                && let Some(parent_tala_id) = graph
                    .source_parent_tala_by_tala
                    .get(&carrier_tala_id)
                    .copied()
                    .flatten()
                    .or_else(|| {
                        graph
                            .transaction_external_container_children
                            .iter()
                            .find(|(_, children)| {
                                children
                                    .iter()
                                    .any(|child| child.tala_id == carrier_tala_id)
                            })
                            .map(|(parent, _)| *parent)
                    })
                    .or_else(|| {
                        graph
                            .nodes
                            .iter()
                            .find(|node| node.tala_id == carrier_tala_id)
                            .and_then(|node| node.container)
                            .map(|parent| graph.nodes[parent.0 as usize].tala_id)
                    })
                && (snapshot_root_tala_id.is_some_and(|root_tala_id| {
                    parent_tala_id != root_tala_id
                        && graph.source_is_descendant_of_tala(parent_tala_id, root_tala_id)
                }) || (snapshot_root_tala_id.is_none() && matches!(snapshot_root, "" | "top")))
                && trace_container_is_nested(graph, parent_tala_id)
            {
                let offset = if top_owner_recursion_snapshot {
                    // At the top-level recursive boundary the immediate
                    // container already carries its source-root position.
                    // These owner callbacks therefore add only its child
                    // inset; walking farther would count the outer
                    // container's padding a second time.
                    let parent_position = TRACE_SYNCED_NODES
                        .get()
                        .and_then(|synced| synced.lock().ok())
                        .and_then(|synced| synced.get(&parent_tala_id).cloned())
                        .and_then(|parent| parent.position)
                        .or_else(|| {
                            graph
                                .nodes
                                .iter()
                                .find(|parent| parent.tala_id == parent_tala_id)
                                .and_then(|parent| parent.position)
                        })
                        .or_else(|| {
                            graph
                                .transaction_external_containers
                                .iter()
                                .find(|parent| parent.tala_id == parent_tala_id)
                                .and_then(|parent| parent.position)
                        })
                        .unwrap_or_default();
                    let padding = graph.trace_source_container_padding(parent_tala_id);
                    Point {
                        x: parent_position.x + padding.left,
                        y: parent_position.y + padding.top,
                    }
                } else {
                    trace_cluster_owner_offset(graph, parent_tala_id, snapshot_root)
                };
                position = position.map(|point| Point {
                    x: point.x + offset.x,
                    y: point.y + offset.y,
                });
            }
            if let Some(root_tala_id) = root_tala_id
                && let Some(carrier_tala_id) = carrier_tala_id
                && graph
                    .source_parent_tala_by_tala
                    .get(&carrier_tala_id)
                    .copied()
                    .flatten()
                    .is_some_and(|parent_tala_id| {
                        parent_tala_id != root_tala_id
                            && graph.source_is_descendant_of_tala(parent_tala_id, root_tala_id)
                    })
                && let Some(parent_tala_id) = graph
                    .source_parent_tala_by_tala
                    .get(&carrier_tala_id)
                    .copied()
                    .flatten()
                && graph
                    .nodes
                    .iter()
                    .any(|node| node.tala_id == parent_tala_id)
            {
                if crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_OWNER") {
                    let padding = graph.trace_source_container_padding(parent_tala_id);
                    eprintln!(
                        "CLUSTER_VESSEL_SHIFT_RUST root={} parent={} pad={:?} pos={:?}",
                        root_tala_id, parent_tala_id, padding, position
                    );
                }
                let padding = graph.trace_source_container_padding(parent_tala_id);
                let parent_position = graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == parent_tala_id)
                    .and_then(|parent| {
                        TRACE_SYNCED_NODES
                            .get()
                            .and_then(|synced| synced.lock().ok())
                            .and_then(|synced| synced.get(&parent.tala_id).cloned())
                            .and_then(|parent| parent.position)
                            .or_else(|| graph.active_node_position(parent.input_id))
                    })
                    .unwrap_or_default();
                position = position.map(|point| Point {
                    x: point.x + parent_position.x + padding.left,
                    y: point.y + parent_position.y + padding.top,
                });
            }
            // An after-fit callback for one top-level container still traces
            // the full owner graph. Synthetic vessels from unrelated
            // branches therefore retain the last owner-frame position even
            // when this scope's transaction alias is at its local origin.
            // Recover that already-observed pointer value for the snapshot;
            // this path is diagnostic-only and does not feed layout state.
            if let Some(root_tala_id) = snapshot_root_tala_id
                && let Some(carrier_tala_id) = carrier_tala_id
                && let Some(parent_tala_id) = graph
                    .source_parent_tala_by_tala
                    .get(&carrier_tala_id)
                    .copied()
                    .flatten()
                && parent_tala_id != root_tala_id
                && !graph.source_is_descendant_of_tala(parent_tala_id, root_tala_id)
                && owner_scope_completed_before_snapshot_root
                && position.is_none_or(|point| point == Point::default())
                && let Some(owner_position) = TRACE_CLUSTER_TRANSITION_POSITIONS
                    .get()
                    .and_then(|positions| positions.lock().ok())
                    .and_then(|positions| positions.get(&cluster.vessel_tala_id).copied())
            {
                position = Some(owner_position);
            }
            // The top-level after-trees callback observes Go's shared vessel
            // pointer after tree placement. Rust's induced owner graph can
            // omit that synthetic pointer from its transaction aliases even
            // though an earlier recursive callback already observed its
            // position. Preserve that last observed pointer value for this
            // specific owner boundary.
            if matches!(snapshot_root, "" | "top")
                && (owner_snapshot
                    || _phase.starts_with("after-restore-edges-")
                    || _phase.starts_with("before-subgraph-"))
                && position.is_none()
                && let Some(owner_position) = TRACE_CLUSTER_TRANSITION_POSITIONS
                    .get()
                    .and_then(|positions| positions.lock().ok())
                    .and_then(|positions| positions.get(&cluster.vessel_tala_id).copied())
            {
                position = Some(owner_position);
            }
            if let Some(current_position) = position {
                if current_position == Point::default() {
                    if _phase == "after-combine"
                        && let Ok(mut positions) = TRACE_CLUSTER_TRANSITION_POSITIONS
                            .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
                            .lock()
                    {
                        // Component packing may place the shared vessel at an
                        // explicit origin. Preserve that pointer state for
                        // later nested owner callbacks that still need the
                        // transition cache as their geometry source.
                        positions.insert(cluster.vessel_tala_id, current_position);
                    }
                    // The enclosing owner graph can contain a cluster vessel
                    // whose transaction alias is still in the nested scope's
                    // local frame. `after-orient` has already observed the
                    // shared vessel in the owner frame; retain that pointer
                    // position across the read-only fit snapshots instead of
                    // replacing it with the stale local alias at the origin.
                    if after_fit_root.is_none() && matches!(_phase, "before-fit" | "after-fit-size")
                    {
                        position = TRACE_CLUSTER_TRANSITION_POSITIONS
                            .get()
                            .and_then(|positions| positions.lock().ok())
                            .and_then(|positions| positions.get(&cluster.vessel_tala_id).copied())
                            .or(Some(current_position));
                    }
                } else if let Ok(mut positions) = TRACE_CLUSTER_TRANSITION_POSITIONS
                    .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
                    .lock()
                {
                    positions.insert(cluster.vessel_tala_id, current_position);
                }
            }
            let current_size = graph.cluster_vessel_size(cluster_index);
            if carrier_tala_id == root_tala_id
                && let Ok(mut sizes) = TRACE_CLUSTER_TRANSITION_SIZES
                    .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
                    .lock()
            {
                sizes.entry(cluster.vessel_tala_id).or_insert(current_size);
            }
            let in_first_member_subtree = root_tala_id.is_some_and(|root_tala_id| {
                carrier_tala_id == Some(root_tala_id)
                    || carrier_tala_id.is_some_and(|carrier| {
                        graph.source_is_descendant_of_tala(root_tala_id, carrier)
                    })
            });
            let in_member_subtree = root_tala_id.is_some_and(|root_tala_id| {
                cluster.members.iter().any(|member| {
                    let member_tala_id = graph.nodes[member.0 as usize].tala_id;
                    root_tala_id == member_tala_id
                        || graph.source_is_descendant_of_tala(root_tala_id, member_tala_id)
                })
            });
            let is_direct_member = root_tala_id.is_some_and(|root_tala_id| {
                cluster
                    .members
                    .iter()
                    .any(|member| graph.nodes[member.0 as usize].tala_id == root_tala_id)
            });
            let optimized_cluster_snapshot = _phase.starts_with("after-cluster-optimize-")
                || _phase.starts_with("after-trees-")
                || _phase.starts_with("after-restore-edges-")
                || _phase.starts_with("after-direct-owner-")
                || _phase.starts_with("after-subgraph-")
                || matches!(
                    _phase,
                    "after-orient" | "before-combine" | "after-combine" | "before-fit"
                );
            let size = if optimized_cluster_snapshot {
                // Cluster optimization mutates the aggregate layout and its
                // shared member geometry before the owner graph's vessel-size
                // cache is refreshed. The Go snapshot reads the live vessel
                // after SyncGeometry, so project its dimensions from the
                // optimized aggregate members at this diagnostic boundary.
                trace_cluster_expanded_size(graph, cluster_index)
                    .unwrap_or_else(|| trace_cluster_size(graph, cluster_index, _phase))
            } else if in_first_member_subtree {
                trace_cluster_size(graph, cluster_index, _phase)
            } else if in_member_subtree && is_direct_member {
                #[cfg(feature = "diagnostic-traces")]
                let pre_resize = TRACE_CLUSTER_PRE_RESIZE_SIZES
                    .get()
                    .and_then(|sizes| sizes.lock().ok())
                    .and_then(|sizes| sizes.get(&cluster.vessel_tala_id).copied());
                #[cfg(not(feature = "diagnostic-traces"))]
                let pre_resize = None;
                pre_resize
                    .or_else(|| {
                        TRACE_CLUSTER_TRANSITION_SIZES
                            .get()
                            .and_then(|sizes| sizes.lock().ok())
                            .and_then(|sizes| sizes.get(&cluster.vessel_tala_id).copied())
                    })
                    .or_else(|| trace_cluster_expanded_size(graph, cluster_index))
                    .unwrap_or(current_size)
            } else if in_member_subtree {
                // A nested descendant's after-fit callback runs after an
                // earlier sibling has resized the shared outer vessel. Go's
                // pointer therefore exposes the transition size; the
                // pre-resize value belongs only to the direct member whose
                // own fit is about to trigger that resize.
                TRACE_CLUSTER_TRANSITION_SIZES
                    .get()
                    .and_then(|sizes| sizes.lock().ok())
                    .and_then(|sizes| sizes.get(&cluster.vessel_tala_id).copied())
                    .or_else(|| trace_cluster_expanded_size(graph, cluster_index))
                    .unwrap_or(current_size)
            } else {
                current_size
            };
            if crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_SNAPSHOT") {
                let transition = TRACE_CLUSTER_TRANSITION_SIZES
                    .get()
                    .and_then(|sizes| sizes.lock().ok())
                    .and_then(|sizes| sizes.get(&cluster.vessel_tala_id).copied());
                let root_tala_id = after_fit_root
                    .map(|root_id| graph.nodes[root_id.0 as usize].tala_id)
                    .unwrap_or_default();
                eprintln!(
                    "CLUSTER_SNAPSHOT_RUST root={} vessel={} carrier={:?} current={:?} selected={:?} first={} member={} transition={:?}",
                    root_tala_id,
                    cluster.vessel_tala_id,
                    carrier_tala_id,
                    current_size,
                    size,
                    in_first_member_subtree,
                    in_member_subtree,
                    transition
                );
            }
            let arrangement = match cluster.arrangement {
                ClusterArrangement::Row => "Row",
                ClusterArrangement::Column => "Column",
            };
            let members = cluster
                .members
                .iter()
                .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let fixed = graph.nodes[carrier.0 as usize].fixed_top_left;
            nodes.push(PlacementTraceNode {
                id: format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}"),
                stable_id: cluster.vessel_tala_id.to_string(),
                w_bits: trace_float_bits(size.width),
                h_bits: trace_float_bits(size.height),
                x_bits: position.map(|point| trace_float_bits(point.x)),
                y_bits: position.map(|point| trace_float_bits(point.y)),
                fixed_x_bits: fixed.map(|point| trace_float_bits(point.x)),
                fixed_y_bits: fixed.map(|point| trace_float_bits(point.y)),
            });
            return;
        }
        if let Some((sequence_index, sequence)) =
            graph.sequences.iter().enumerate().find(|(_, sequence)| {
                sequence.members.contains(&node_id)
                    && (project_inactive_aggregates || graph.sequence_is_active(sequence))
            })
        {
            let Some(&carrier) = sequence.members.first() else {
                return;
            };
            if node_id != carrier {
                return;
            }
            for member in &sequence.members {
                seen.insert(*member);
            }
            // Before preprocessing, Go's Sequence.PlaceVessel copies the
            // minimum top-left only when every step is positioned. Later
            // owner callbacks expose the shared step pointers captured at
            // the preceding after-fit boundary. Tree publication has a
            // separate vessel pointer, so do not infer its position from a
            // retained member that happened to be laid out independently.
            let minimum_member_position = |positions: Vec<Option<Point>>| {
                positions
                    .into_iter()
                    .collect::<Option<Vec<_>>>()
                    .and_then(|positions| {
                        positions.into_iter().reduce(|left, right| Point {
                            x: left.x.min(right.x),
                            y: left.y.min(right.y),
                        })
                    })
            };
            let synced_member_position = || {
                minimum_member_position(
                    sequence
                        .members
                        .iter()
                        .map(|member| {
                            TRACE_SYNCED_NODES
                                .get()
                                .and_then(|synced| synced.lock().ok())
                                .and_then(|synced| {
                                    synced
                                        .get(&graph.nodes[member.0 as usize].tala_id)
                                        .and_then(|node| node.position)
                                })
                        })
                        .collect(),
                )
            };
            let active_member_position = || {
                minimum_member_position(
                    sequence
                        .members
                        .iter()
                        .map(|member| graph.active_node_position(*member))
                        .collect(),
                )
            };
            let source_member_position = || {
                minimum_member_position(
                    sequence
                        .members
                        .iter()
                        .map(|member| {
                            graph
                                .source_positions_by_tala
                                .get(&graph.nodes[member.0 as usize].tala_id)
                                .copied()
                        })
                        .collect(),
                )
            };
            let projected_vessel_position = || {
                TRACE_SYNCED_NODES
                    .get()
                    .and_then(|synced| synced.lock().ok())
                    .and_then(|synced| {
                        synced
                            .get(&sequence.vessel_tala_id)
                            .and_then(|node| node.position)
                    })
                    .or_else(|| graph.projected_node_position_by_tala(sequence.vessel_tala_id))
            };
            let sequence_component_index = sequence_component_indices_by_tala
                .get(&graph.nodes[carrier.0 as usize].tala_id)
                .copied();
            let current_component_has_positioned_sequence = phase_subgraph_index
                .zip(sequence_component_index)
                .is_some_and(|(phase_index, sequence_index)| {
                    if _phase.starts_with("before-subgraph-") {
                        phase_index > sequence_index
                    } else {
                        phase_index >= sequence_index
                    }
                })
                || matches!(_phase, "before-combine" | "after-combine" | "after-orient");
            let position = if matches!(_phase, "before-fit" | "after-fit-size" | "after-fit") {
                synced_member_position()
            } else if sequence.container.is_none() {
                // Root-level sequences can remain unpositioned while their
                // retained step aliases receive local layout coordinates.
                // Those coordinates do not imply a live vessel TopLeft.
                if current_component_has_positioned_sequence {
                    synced_member_position()
                        .or_else(active_member_position)
                        .or_else(projected_vessel_position)
                } else {
                    // Replaying a post-layout GraphJSON snapshot rebuilds
                    // Go's sequence vessel from its already-positioned steps.
                    // Keep that pointer visible through owner callbacks until
                    // its own component is placed; fresh D2 input leaves the
                    // steps unpositioned, so this does not invent geometry.
                    projected_vessel_position()
                        .or_else(active_member_position)
                        .or_else(source_member_position)
                }
            } else {
                active_member_position()
            };
            // During root callbacks the sequence members may have been
            // copied back into a transaction projection with their
            // pre-expansion boxes.  Go's vessel pointer reports that live
            // snapshot, while the stable arena's member nodes can already
            // carry the later expanded dimensions.  Prefer the transaction
            // members for this diagnostic boundary when they cover the full
            // sequence; ordinary placement remains unchanged.
            let size = graph
                .transaction_external_aggregate_children
                .values()
                .find(|members| {
                    members.len() == sequence.members.len()
                        && sequence.members.iter().all(|member| {
                            let tala_id = graph.nodes[member.0 as usize].tala_id;
                            members.iter().any(|candidate| candidate.tala_id == tala_id)
                        })
                })
                .map(|members| Size {
                    width: members
                        .iter()
                        .map(|member| member.rect.size.width - 35.0)
                        .sum::<f64>()
                        + 35.0,
                    height: members
                        .first()
                        .map(|member| member.rect.size.height)
                        .unwrap_or(0.0),
                })
                .unwrap_or_else(|| graph.sequence_vessel_size(sequence_index));
            let fixed = graph.nodes[carrier.0 as usize].fixed_top_left;
            nodes.push(PlacementTraceNode {
                id: format!(
                    "Sequence vessel of: [{}]",
                    sequence
                        .members
                        .iter()
                        .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                stable_id: sequence.vessel_tala_id.to_string(),
                w_bits: trace_float_bits(size.width),
                h_bits: trace_float_bits(size.height),
                x_bits: position.map(|point| trace_float_bits(point.x)),
                y_bits: position.map(|point| trace_float_bits(point.y)),
                fixed_x_bits: fixed.map(|point| trace_float_bits(point.x)),
                fixed_y_bits: fixed.map(|point| trace_float_bits(point.y)),
            });
            return;
        }
        // Root snapshots are built from a transaction projection after the
        // aggregate records have been copied out of the temporary graph. In
        // that view the only remaining evidence is the vessel-keyed child
        // map and its layout metadata; recover the same vessel geometry for
        // the trace without changing the stable runtime graph.
        if project_inactive_aggregates
            && let Some((_, children)) = graph
                .transaction_external_aggregate_children
                .iter()
                .find(|(_, children)| children.iter().any(|child| child.tala_id == node.tala_id))
        {
            let Some(carrier) = children.first() else {
                return;
            };
            if carrier.tala_id != node.tala_id {
                return;
            }
            for child in children {
                if let Some(member) = graph
                    .nodes
                    .iter()
                    .find(|member| member.tala_id == child.tala_id)
                {
                    seen.insert(member.input_id);
                }
            }
            let vessel_id = graph
                .transaction_external_aggregate_children
                .iter()
                .find(|(_, members)| {
                    members
                        .first()
                        .is_some_and(|member| member.tala_id == carrier.tala_id)
                })
                .map(|(vessel_id, _)| *vessel_id)
                .unwrap_or_default();
            let layout = graph
                .transaction_external_cluster_layouts
                .get(&vessel_id)
                .copied()
                .unwrap_or(ExternalClusterLayout {
                    arrangement: ClusterArrangement::Column,
                    padding: 20.0,
                    fixed_size: false,
                });
            let arrangement = match layout.arrangement {
                ClusterArrangement::Row => "Row",
                ClusterArrangement::Column => "Column",
            };
            let members = children
                .iter()
                .map(|child| child.external_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            // A completed aggregate keeps its authoritative vessel box in
            // the cluster cache. The transaction child aliases can include
            // expanded descendants, so summing their rectangles here
            // overstates the vessel (and diverges from Go's pointer view).
            let cached_size = graph
                .clusters
                .iter()
                .enumerate()
                .find(|(_, cluster)| {
                    cluster.members.len() == children.len()
                        && cluster.members.iter().all(|member| {
                            let tala_id = graph.nodes[member.0 as usize].tala_id;
                            children.iter().any(|child| child.tala_id == tala_id)
                        })
                })
                .map(|(index, _)| graph.cluster_vessel_size(index));
            if crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_MAP")
                && children.iter().any(|child| child.tala_id == 3946625030)
            {
                eprintln!(
                    "TRACE_ROOT_AGG_SIZE_RUST children={:?} cached={:?} expanded={:?} clusters={:?}",
                    children
                        .iter()
                        .map(|child| child.tala_id)
                        .collect::<Vec<_>>(),
                    cached_size,
                    graph
                        .clusters
                        .iter()
                        .enumerate()
                        .map(|(index, _)| (
                            index,
                            graph.cluster_vessel_size(index),
                            trace_cluster_expanded_size(graph, index)
                        ))
                        .collect::<Vec<_>>(),
                    graph
                        .clusters
                        .iter()
                        .map(|cluster| cluster
                            .members
                            .iter()
                            .map(|member| graph.nodes[member.0 as usize].tala_id)
                            .collect::<Vec<_>>())
                        .collect::<Vec<_>>()
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_MAP")
                && children.iter().any(|child| child.tala_id == 3946625030)
            {
                eprintln!(
                    "TRACE_ROOT_AGG_SIZE_RUST children={:?} cached={:?} expanded={:?} clusters={:?}",
                    children
                        .iter()
                        .map(|child| child.tala_id)
                        .collect::<Vec<_>>(),
                    cached_size,
                    graph
                        .clusters
                        .iter()
                        .enumerate()
                        .map(|(index, _)| (
                            index,
                            graph.cluster_vessel_size(index),
                            trace_cluster_expanded_size(graph, index)
                        ))
                        .collect::<Vec<_>>(),
                    graph
                        .clusters
                        .iter()
                        .map(|cluster| cluster
                            .members
                            .iter()
                            .map(|member| graph.nodes[member.0 as usize].tala_id)
                            .collect::<Vec<_>>())
                        .collect::<Vec<_>>()
                );
            }
            let (width, height) = if let Some(size) = cached_size {
                (size.width, size.height)
            } else {
                match layout.arrangement {
                    ClusterArrangement::Row => (
                        children
                            .iter()
                            .map(|child| child.rect.size.width)
                            .sum::<f64>()
                            + layout.padding * (children.len().saturating_sub(1) as f64),
                        children
                            .iter()
                            .map(|child| child.rect.size.height)
                            .fold(0.0, f64::max),
                    ),
                    ClusterArrangement::Column => (
                        children
                            .iter()
                            .map(|child| child.rect.size.width)
                            .fold(0.0, f64::max),
                        children
                            .iter()
                            .map(|child| child.rect.size.height)
                            .sum::<f64>()
                            + layout.padding * (children.len().saturating_sub(1) as f64),
                    ),
                }
            };
            let position = graph.active_node_position(node_id).map(|point| {
                let parent_tala_id = graph
                    .source_parent_tala_by_tala
                    .get(&carrier.tala_id)
                    .copied()
                    .flatten()
                    .or_else(|| {
                        graph
                            .transaction_external_container_children
                            .iter()
                            .find(|(_, children)| {
                                children
                                    .iter()
                                    .any(|child| child.tala_id == carrier.tala_id)
                            })
                            .map(|(parent, _)| *parent)
                    })
                    .or_else(|| {
                        graph
                            .nodes
                            .iter()
                            .find(|candidate| candidate.tala_id == carrier.tala_id)
                            .and_then(|candidate| candidate.container)
                            .map(|parent| graph.nodes[parent.0 as usize].tala_id)
                    });
                let Some(parent_tala_id) = parent_tala_id else {
                    return point;
                };
                if !trace_container_is_nested(graph, parent_tala_id) {
                    return point;
                }
                let parent_position = TRACE_SYNCED_NODES
                    .get()
                    .and_then(|synced| synced.lock().ok())
                    .and_then(|synced| synced.get(&parent_tala_id).cloned())
                    .and_then(|parent| parent.position)
                    .or_else(|| {
                        graph
                            .nodes
                            .iter()
                            .find(|candidate| candidate.tala_id == parent_tala_id)
                            .and_then(|parent| parent.position)
                    })
                    .or_else(|| {
                        graph
                            .transaction_external_containers
                            .iter()
                            .find(|parent| parent.tala_id == parent_tala_id)
                            .and_then(|parent| parent.position)
                    })
                    .unwrap_or_default();
                let padding = graph.trace_source_container_padding(parent_tala_id);
                Point {
                    x: point.x + parent_position.x + padding.left,
                    y: point.y + parent_position.y + padding.top,
                }
            });
            nodes.push(PlacementTraceNode {
                id: format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}"),
                stable_id: vessel_id.to_string(),
                w_bits: trace_float_bits(width),
                h_bits: trace_float_bits(height),
                x_bits: position.map(|point| trace_float_bits(point.x)),
                y_bits: position.map(|point| trace_float_bits(point.y)),
                fixed_x_bits: None,
                fixed_y_bits: None,
            });
            return;
        }
        let fixed_direct_child = after_fit_root
            .filter(|root_id| node.container == Some(*root_id))
            .and(node.fixed_top_left);
        // `TRACE_SYNCED_NODES` is populated by an `after-fit` callback and
        // intentionally survives later scopes.  It must not override the
        // local coordinates of a subsequent `initialized` placement event;
        // Go's trace sees that scope's freshly initialized pointers.
        let synced = after_fit_root
            .is_some()
            .then(|| {
                TRACE_SYNCED_NODES
                    .get()
                    .and_then(|nodes| nodes.lock().ok())
                    .and_then(|nodes| nodes.get(&node.tala_id).cloned())
            })
            .flatten();
        let owner_graph_snapshot = root_graph_snapshot
            || _phase.starts_with("after-orthogonal-")
            || _phase.starts_with("after-cluster-optimize-")
            || _phase.starts_with("after-trees-")
            || _phase.starts_with("after-restore-edges-")
            || _phase.starts_with("after-direct-owner-")
            || _phase.starts_with("after-subgraph-")
            || matches!(
                _phase,
                "placeNodes-entry"
                    | "after-child-recursion"
                    | "after-nears"
                    | "after-herds"
                    | "after-split"
                    | "after-orient"
                    | "before-combine"
                    | "after-combine"
                    | "before-fit"
                    | "after-fit-size"
            )
            || _phase.starts_with("before-subgraph-");
        #[cfg(feature = "diagnostic-traces")]
        let use_synced_owner = matches!(
            _phase,
            "placeNodes-entry"
                | "after-child-recursion"
                | "after-nears"
                | "after-herds"
                | "after-split"
                | "before-fit"
        ) || _phase.starts_with("before-subgraph-")
            || (_phase == "after-fit-size" && node.external_id != snapshot_root);
        // Every root.Graph placement event represents the owner’s live Go
        // pointers. The Rust induced graph stores independent value snapshots;
        // callback deltas refresh this cache for nodes that actually changed,
        // while unchanged nodes keep their latest owner geometry.
        #[cfg(feature = "diagnostic-traces")]
        let shared_owner_snapshot = root_graph_snapshot
            || _phase.starts_with("after-orthogonal-")
            || _phase.starts_with("after-cluster-optimize-")
            || _phase.starts_with("after-trees-")
            || _phase.starts_with("after-restore-edges-")
            || _phase.starts_with("after-direct-owner-")
            || _phase.starts_with("after-subgraph-")
            || matches!(_phase, "before-combine" | "after-combine" | "after-orient");
        #[cfg(feature = "diagnostic-traces")]
        let use_synced_node = if shared_owner_snapshot {
            if _phase == "orthogonal-start" {
                true
            } else if snapshot_root.is_empty() || snapshot_root == "top" {
                false
            } else {
                let active_root = graph
                    .nodes
                    .iter()
                    .position(|candidate| candidate.external_id == snapshot_root)
                    .map(|index| NodeId(index as u32));
                active_root == Some(node_id)
                    || !active_root.is_some_and(|active_root| {
                        graph.is_descendant_of(node_id, active_root)
                            || graph.source_is_descendant_of_tala(
                                node.tala_id,
                                graph.nodes[active_root.0 as usize].tala_id,
                            )
                    })
            }
        } else {
            // Most root-owner callbacks need the synchronized descendant
            // positions published by recursive child placement. The first
            // top-level subgraph still uses that cache; later subgraphs read
            // the live owner after earlier direct passes have updated it.
            let later_top_level_subgraph = _phase
                .strip_prefix("before-subgraph-")
                .and_then(|index| index.parse::<usize>().ok())
                .is_some_and(|index| index > 0)
                && (snapshot_root.is_empty() || snapshot_root == "top");
            use_synced_owner && !later_top_level_subgraph
        };
        #[cfg(feature = "diagnostic-traces")]
        let later_top_orthogonal_pass = _phase
            .strip_prefix("after-orthogonal-")
            .and_then(|index| index.parse::<usize>().ok())
            .is_some_and(|index| index > 0)
            && (snapshot_root.is_empty() || snapshot_root == "top");
        #[cfg(feature = "diagnostic-traces")]
        let owner_view_is_strict_subset = {
            #[cfg(feature = "diagnostic-traces")]
            {
                let owner_order = graph
                    .trace_owner_node_orders
                    .lock()
                    .ok()
                    .and_then(|owners| owners.get(&node.tala_id).cloned());
                let root_order = graph.containers.get(&None).map(|children| {
                    children
                        .iter()
                        .map(|node_id| graph.nodes[node_id.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                });
                owner_order.is_some_and(|owner_order| {
                    root_order.is_some_and(|root_order| {
                        owner_order.len() > 1
                            && root_order.len().saturating_sub(owner_order.len()) >= 2
                            && owner_order
                                .iter()
                                .all(|tala_id| root_order.contains(tala_id))
                    })
                })
            }
            #[cfg(not(feature = "diagnostic-traces"))]
            {
                false
            }
        };
        #[cfg(feature = "diagnostic-traces")]
        let use_owner_frame_synced = (shared_owner_snapshot
            || _phase.starts_with("before-subgraph-"))
            && (snapshot_root.is_empty() || snapshot_root == "top")
            && (!later_top_orthogonal_pass || _owner_changes.is_some())
            && _phase != "after-combine"
            && owner_view_is_strict_subset
            && node.container.is_none()
            && graph
                .containers
                .get(&Some(node_id))
                .is_some_and(|children| !children.is_empty());
        #[cfg(feature = "diagnostic-traces")]
        let owner_changed_in_callback =
            _owner_changes.is_some_and(|changed_tala_ids| changed_tala_ids.contains(&node.tala_id));
        #[cfg(feature = "diagnostic-traces")]
        let prefer_live_owner =
            use_owner_frame_synced && shared_owner_snapshot && owner_changed_in_callback;
        #[cfg(feature = "diagnostic-traces")]
        let use_owner_frame_synced = use_owner_frame_synced && !owner_changed_in_callback;
        #[cfg(feature = "diagnostic-traces")]
        let owner_frame_probe_target =
            crate::internal::trace_env_value("D2RS_TRACE_OWNER_FRAME_TARGET");
        #[cfg(feature = "diagnostic-traces")]
        let owner_frame_probe_phase =
            crate::internal::trace_env_value("D2RS_TRACE_OWNER_FRAME_PHASE");
        #[cfg(feature = "diagnostic-traces")]
        let owner_frame_probe_matches = owner_frame_probe_target.as_deref().is_some_and(|target| {
            let probe_root = if snapshot_root.is_empty() {
                "top"
            } else {
                snapshot_root
            };
            target == format!("{probe_root}:{}", node.tala_id)
                && owner_frame_probe_phase
                    .as_deref()
                    .unwrap_or("orthogonal-after-sized")
                    == _phase
        });
        #[cfg(feature = "diagnostic-traces")]
        let owner_node = owner_graph_snapshot.then(|| {
            // When `use_owner_frame_synced` selected the root owner frame,
            // consult that cache before the current arena node. Recursive
            // placement owns a value snapshot, so its live coordinates can
            // belong to an older frame even though this is an owner callback.
            trace_node_snapshot(
                graph,
                node.tala_id,
                node,
                use_owner_frame_synced,
                prefer_live_owner,
                use_synced_node,
            )
        });
        #[cfg(not(feature = "diagnostic-traces"))]
        let owner_node: Option<ArenaNode> = None;
        #[cfg(feature = "diagnostic-traces")]
        if owner_frame_probe_matches {
            let position = |nodes: &std::sync::MutexGuard<'_, BTreeMap<i64, ArenaNode>>| {
                nodes.get(&node.tala_id).and_then(|node| node.position)
            };
            let synced_position = TRACE_SYNCED_NODES
                .get()
                .and_then(|nodes| nodes.lock().ok())
                .as_ref()
                .and_then(position);
            let owner_position = TRACE_OWNER_FRAME_NODES
                .get()
                .and_then(|nodes| nodes.lock().ok())
                .as_ref()
                .and_then(position);
            let live_position = graph
                .nodes
                .iter()
                .find(|candidate| candidate.tala_id == node.tala_id)
                .and_then(|candidate| candidate.position);
            eprintln!(
                "D2RS_TRACE_OWNER_FRAME root={} node={} live={live_position:?} synced={synced_position:?} owner={owner_position:?} snapshot={:?} owner_view_is_strict_subset={} shared_owner_snapshot={} use_owner_frame_synced={} changed={}",
                snapshot_root,
                node.tala_id,
                owner_node.as_ref().and_then(|snapshot| snapshot.position),
                owner_view_is_strict_subset,
                shared_owner_snapshot,
                use_owner_frame_synced,
                _owner_changes.is_some_and(|changes| changes.contains(&node.tala_id)),
            );
        }
        let position = fixed_direct_child.or_else(|| {
            owner_node
                .as_ref()
                .and_then(|snapshot| snapshot.position)
                .or_else(|| {
                    synced
                        .as_ref()
                        .and_then(|node| node.position)
                        .or_else(|| graph.active_node_position(node_id))
                })
        });
        // Root-owner callbacks expose the published source coordinates
        // directly, and the after-fit snapshot already carries synchronized
        // pointer positions. Nested owner snapshots still require the source
        // aggregate frame below to match their upstream coordinate view.
        let aggregate_offset = if ((snapshot_root.is_empty() || snapshot_root == "top")
            && owner_graph_snapshot)
            || after_fit_root.is_some()
        {
            Point::default()
        } else {
            graph
                .source_parent_tala_by_tala
                .get(&node.tala_id)
                .copied()
                .flatten()
                .map(|parent| {
                    trace_aggregate_ancestor_offset(
                        parent,
                        graph,
                        after_fit_root.is_some(),
                        after_fit_root.map(|root_id| graph.nodes[root_id.0 as usize].tala_id),
                    )
                })
                .unwrap_or_default()
        };
        if node.tala_id == 1231618029
            && crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_MAP")
        {
            eprintln!(
                "TRACE_VISIT_TARGET_RUST root={:?} after_fit={:?} parent={:?} pos={:?} offset={:?} synced={:?}",
                after_fit_root,
                after_fit_root.is_some(),
                graph.source_parent_tala_by_tala.get(&node.tala_id),
                position,
                aggregate_offset,
                synced.as_ref().and_then(|node| node.position)
            );
            eprintln!(
                "TRACE_VISIT_TARGET_STATE_RUST clusters={:?} txns={:?}",
                graph
                    .clusters
                    .iter()
                    .map(|cluster| {
                        cluster
                            .members
                            .iter()
                            .map(|member| graph.nodes[member.0 as usize].tala_id)
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>(),
                graph
                    .transaction_external_aggregate_children
                    .iter()
                    .map(|(id, members)| {
                        (
                            *id,
                            members
                                .iter()
                                .map(|member| member.tala_id)
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>()
            );
        }
        let position = position.map(|point| Point {
            x: point.x + aggregate_offset.x,
            y: point.y + aggregate_offset.y,
        });
        #[cfg(feature = "diagnostic-traces")]
        if owner_frame_probe_matches {
            let stable = graph
                .nodes
                .iter()
                .find(|candidate| candidate.tala_id == node.tala_id)
                .map(|candidate| (candidate.position, candidate.rect.origin));
            let external_containers = graph
                .transaction_external_containers
                .iter()
                .filter(|candidate| candidate.tala_id == node.tala_id)
                .map(|candidate| (candidate.position, candidate.rect.origin))
                .collect::<Vec<_>>();
            let external_children = graph
                .transaction_external_container_children
                .iter()
                .flat_map(|(parent, children)| {
                    children
                        .iter()
                        .filter(|candidate| candidate.tala_id == node.tala_id)
                        .map(|candidate| (*parent, candidate.position, candidate.rect.origin))
                })
                .collect::<Vec<_>>();
            let aggregate_children = graph
                .transaction_external_aggregate_children
                .iter()
                .flat_map(|(parent, children)| {
                    children
                        .iter()
                        .filter(|candidate| candidate.tala_id == node.tala_id)
                        .map(|candidate| (*parent, candidate.position, candidate.rect.origin))
                })
                .collect::<Vec<_>>();
            let trace_children = graph
                .trace_container_children
                .iter()
                .flat_map(|(parent, children)| {
                    children
                        .iter()
                        .filter(|candidate| candidate.tala_id == node.tala_id)
                        .map(|candidate| (*parent, candidate.position, candidate.rect.origin))
                })
                .collect::<Vec<_>>();
            eprintln!(
                "D2RS_TRACE_OWNER_FRAME_DETAIL root={} node={} final={position:?} offset={aggregate_offset:?} stable={stable:?} external_containers={external_containers:?} external_children={external_children:?} aggregate_children={aggregate_children:?} trace_children={trace_children:?}",
                snapshot_root, node.tala_id
            );
        }
        let fixed = owner_node
            .as_ref()
            .and_then(|node| node.fixed_top_left)
            .or_else(|| {
                synced
                    .as_ref()
                    .and_then(|node| node.fixed_top_left)
                    .or(node.fixed_top_left)
            });
        let size = owner_node
            .as_ref()
            .map(|node| node.rect.size)
            .or_else(|| synced.as_ref().map(|node| node.rect.size))
            .unwrap_or_else(|| graph.active_node_size(node_id));
        nodes.push(PlacementTraceNode {
            id: trace_node_external_id(graph, node_id),
            stable_id: node.tala_id.to_string(),
            w_bits: trace_float_bits(size.width),
            h_bits: trace_float_bits(size.height),
            x_bits: position.map(|point| trace_float_bits(point.x)),
            y_bits: position.map(|point| trace_float_bits(point.y)),
            fixed_x_bits: fixed.map(|point| trace_float_bits(point.x)),
            fixed_y_bits: fixed.map(|point| trace_float_bits(point.y)),
        });
        let is_active_aggregate_carrier = trace_node_external_id(graph, node_id)
            != node.external_id
            || graph.clusters.iter().any(|cluster| {
                graph.cluster_is_active(cluster) && cluster.members.first() == Some(&node_id)
            })
            || graph.sequences.iter().any(|sequence| {
                graph.sequence_is_active(sequence) && sequence.members.first() == Some(&node_id)
            });
        if recurse_children && !is_active_aggregate_carrier {
            // tracePlacement recurses through the live Containers slice,
            // independently of Graph.Nodes publication order.
            let mut ordered_children = if root_graph_snapshot {
                trace_root_graph_child_ids(graph, node.tala_id)
                    .into_iter()
                    .filter_map(|tala_id| {
                        graph
                            .nodes
                            .iter()
                            .position(|child| child.tala_id == tala_id)
                            .map(|index| NodeId(index as u32))
                    })
                    .collect::<Vec<_>>()
            } else if trace_phase_uses_live_owner_children(_phase) {
                let root_tala_id = graph
                    .nodes
                    .iter()
                    .find(|node| node.external_id == snapshot_root)
                    .map(|node| node.tala_id);
                trace_owner_phase_child_ids(
                    graph,
                    node.tala_id,
                    _phase,
                    root_tala_id,
                    snapshot_root.is_empty() || snapshot_root == "top",
                )
                .into_iter()
                .filter_map(|tala_id| {
                    graph
                        .nodes
                        .iter()
                        .position(|child| child.tala_id == tala_id)
                        .map(|index| NodeId(index as u32))
                })
                .collect::<Vec<_>>()
            } else {
                graph.containers.get(&Some(node_id)).unwrap_or_default()
            };
            // Go's CombineSubgraphs appends an aggregate vessel after the
            // ordinary children of its container. The stable arena keeps the
            // carrier in the source position, so defer active cluster and
            // sequence carriers in the diagnostic traversal while retaining
            // declaration order for all ordinary children.
            ordered_children.stable_sort_by_key(|child| {
                let aggregate_carrier = graph.clusters.iter().any(|cluster| {
                    graph.cluster_is_active(cluster) && cluster.members.first() == Some(child)
                }) || graph.sequences.iter().any(|sequence| {
                    graph.sequence_is_active(sequence) && sequence.members.first() == Some(child)
                });
                usize::from(aggregate_carrier)
            });
            for child in ordered_children {
                visit(
                    child,
                    _phase,
                    snapshot_root,
                    graph,
                    phase_subgraph_index,
                    sequence_component_indices_by_tala,
                    seen,
                    nodes,
                    after_fit_root,
                    project_inactive_aggregates,
                    visible_owners,
                    recurse_children,
                    owner_snapshot,
                    root_graph_snapshot,
                    _owner_changes,
                );
            }
        }
    }
    for node_id in &trace_nodes {
        visit(
            *node_id,
            phase,
            root,
            graph,
            phase_subgraph_index,
            &sequence_component_indices_by_tala,
            &mut seen,
            &mut nodes,
            after_fit_root,
            root.is_empty(),
            &visible_owners,
            (after_fit_root.is_some()
                || root.is_empty()
                || owner_snapshot
                || root_graph_snapshot
                || trace_phase_uses_live_owner_children(phase))
                && !owner_container_walk_applied,
            owner_snapshot,
            root_graph_snapshot,
            _owner_changes,
        );
    }
    #[cfg(feature = "diagnostic-traces")]
    if phase == "placeNodes-entry" && std::env::var_os("D2_RS_TRACE_OWNER_GRAPH").is_some() {
        eprintln!(
            "D2_RS_TRACE_BASE_PLACEMENT root={root:?} visited={:?} emitted={:?}",
            trace_nodes
                .iter()
                .map(|node_id| graph.nodes[node_id.0 as usize].tala_id)
                .collect::<Vec<_>>(),
            nodes
                .iter()
                .map(|node| (node.stable_id.as_str(), node.id.as_str()))
                .collect::<Vec<_>>(),
        );
    }
    #[cfg(feature = "diagnostic-traces")]
    trace_placement_target_records(phase, root, "base-visit", &nodes);
    #[cfg(feature = "diagnostic-traces")]
    trace_placement_order_records(phase, root, "base-visit", &nodes);
    #[cfg(feature = "diagnostic-traces")]
    if root.is_empty() {
        // Preserve the source child-order cache for later recursive callbacks
        // even when this snapshot already has an authoritative owner walk.
        // Only the record replay below is redundant for those callbacks.
        fn source_record_identity(
            graph: &ArenaGraph,
            node: &ArenaNode,
        ) -> Option<(String, String)> {
            if let Some(id) = projected_vessel_trace_id(graph, node.tala_id) {
                return Some((id, node.tala_id.to_string()));
            }
            if let Some(cluster) = graph.clusters.iter().find(|cluster| {
                cluster
                    .members
                    .iter()
                    .any(|member| graph.nodes[member.0 as usize].tala_id == node.tala_id)
            }) {
                if cluster
                    .members
                    .first()
                    .is_some_and(|member| graph.nodes[member.0 as usize].tala_id != node.tala_id)
                {
                    return None;
                }
                let members = cluster
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let arrangement = match cluster.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                };
                return Some((
                    format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}"),
                    cluster.vessel_tala_id.to_string(),
                ));
            }
            if let Some((vessel_id, members)) = graph
                .transaction_external_aggregate_children
                .iter()
                .find(|(_, members)| members.iter().any(|member| member.tala_id == node.tala_id))
            {
                if members
                    .first()
                    .is_some_and(|member| member.tala_id != node.tala_id)
                {
                    return None;
                }
                let member_names = members
                    .iter()
                    .map(|member| member.external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Some(layout) = graph.transaction_external_cluster_layouts.get(vessel_id) {
                    let arrangement = match layout.arrangement {
                        ClusterArrangement::Row => "Row",
                        ClusterArrangement::Column => "Column",
                    };
                    return Some((
                        format!("Cluster vessel of: [{member_names}]; Arrangement: {arrangement}"),
                        vessel_id.to_string(),
                    ));
                }
                return Some((
                    format!("Sequence vessel of: [{member_names}]"),
                    vessel_id.to_string(),
                ));
            }
            if let Some(sequence) = graph.sequences.iter().find(|sequence| {
                sequence
                    .members
                    .iter()
                    .any(|member| graph.nodes[member.0 as usize].tala_id == node.tala_id)
            }) {
                if sequence
                    .members
                    .first()
                    .is_some_and(|member| graph.nodes[member.0 as usize].tala_id != node.tala_id)
                {
                    return None;
                }
                let members = sequence
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some((
                    format!("Sequence vessel of: [{members}]"),
                    sequence.vessel_tala_id.to_string(),
                ));
            }
            Some((node.external_id.clone(), node.tala_id.to_string()))
        }
        let mut source_children = BTreeMap::<i64, Vec<ArenaNode>>::new();
        let mut source_child_ids = BTreeSet::new();
        for (&parent_tala_id, children) in &graph.trace_container_children {
            source_children
                .entry(parent_tala_id)
                .or_default()
                .extend(children.iter().cloned());
            source_child_ids.extend(children.iter().map(|child| child.tala_id));
        }
        // After tree restoration the owning Go graph retains the complete
        // direct-child slices in its transaction projection.  The stable
        // trace map is captured earlier and can therefore omit nodes that
        // were peeled into trees (for example resolvers.gen and
        // resolvers.adapter).  Prefer the transaction slice whenever it is
        // at least as complete; it also preserves the exact Graph.Containers
        // order used by tracePlacement.
        for (&parent_tala_id, children) in &graph.transaction_external_container_children {
            if children.is_empty() {
                continue;
            }
            let replace = match source_children.get(&parent_tala_id) {
                None => true,
                Some(existing) if children.len() > existing.len() => true,
                Some(existing) if children.len() < existing.len() => false,
                Some(_existing) => {
                    // A same-sized transaction slice can be a Rust arena
                    // ordering, while the trace slice reflects Go's source
                    // Containers traversal (or vice versa). Horizontal
                    // scopes preserve the trace slice; vertical scopes use
                    // the transaction's restored source order.
                    let direction = graph
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == parent_tala_id)
                        .and_then(|node| graph.directions.get(&Some(node.input_id)).copied())
                        .or_else(|| graph.directions_by_tala.get(&Some(parent_tala_id)).copied())
                        .unwrap_or(Direction::Down);
                    !matches!(direction, Direction::Right | Direction::Left)
                }
            };
            if replace {
                source_children.insert(parent_tala_id, children.clone());
                source_child_ids.extend(children.iter().map(|child| child.tala_id));
            }
        }
        for (&parent_tala_id, child_ids) in &graph.source_container_child_order {
            for &child_tala_id in child_ids {
                let Some(child) = graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == child_tala_id)
                else {
                    continue;
                };
                source_children
                    .entry(parent_tala_id)
                    .or_default()
                    .push(child.clone());
                source_child_ids.insert(child_tala_id);
            }
        }
        for node in &graph.nodes {
            if source_child_ids.contains(&node.tala_id) {
                continue;
            }
            let parent_tala_id = graph
                .source_parent_tala_by_tala
                .get(&node.tala_id)
                .copied()
                .flatten()
                .or_else(|| {
                    node.container
                        .map(|container| graph.nodes[container.0 as usize].tala_id)
                })
                .unwrap_or_default();
            source_children
                .entry(parent_tala_id)
                .or_default()
                .push(node.clone());
        }
        #[cfg(feature = "diagnostic-traces")]
        if let Ok(mut orders) = TRACE_SOURCE_CHILD_ORDER
            .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
            .lock()
        {
            // Capture the first complete source projection only. Later
            // callbacks may reflect tree restoration and aggregate packing,
            // which intentionally mutate the live child slices.
            for (parent_tala_id, children) in &source_children {
                if !children.is_empty() {
                    orders
                        .entry(*parent_tala_id)
                        .or_insert_with(|| children.iter().map(|child| child.tala_id).collect());
                }
            }
        }
        fn source_walk_node(
            graph: &ArenaGraph,
            source_children: &BTreeMap<i64, Vec<ArenaNode>>,
            tala_id: i64,
            seen: &mut BTreeSet<i64>,
            order: &mut Vec<(String, String)>,
        ) {
            if !seen.insert(tala_id) {
                return;
            }
            let child = graph
                .nodes
                .iter()
                .find(|node| node.tala_id == tala_id)
                .or_else(|| {
                    source_children
                        .values()
                        .flatten()
                        .find(|node| node.tala_id == tala_id)
                });
            let Some(child) = child else {
                return;
            };
            if let Some(identity) = source_record_identity(graph, child) {
                order.push(identity);
            }
            let is_aggregate = graph.clusters.iter().any(|cluster| {
                cluster
                    .members
                    .iter()
                    .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
            }) || graph.sequences.iter().any(|sequence| {
                sequence
                    .members
                    .iter()
                    .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
            }) || graph
                .transaction_external_aggregate_children
                .values()
                .any(|members| members.iter().any(|member| member.tala_id == tala_id));
            if is_aggregate {
                return;
            }
            let children = trace_live_owner_child_ids(graph, tala_id, false).unwrap_or_else(|| {
                source_children
                    .get(&tala_id)
                    .into_iter()
                    .flatten()
                    .map(|node| node.tala_id)
                    .collect()
            });
            for child_tala_id in children {
                source_walk_node(graph, source_children, child_tala_id, seen, order);
            }
        }
        // Graph.Nodes supplies root sibling order; Containers supplies the
        // recursive order below each root. Do not derive the root slice from
        // an owner-child map, whose keys can differ from source node order.
        let root_tala_ids = trace_nodes
            .iter()
            .filter_map(|node_id| {
                let node = &graph.nodes[node_id.0 as usize];
                let source_parent = graph
                    .source_parent_tala_by_tala
                    .get(&node.tala_id)
                    .copied()
                    .unwrap_or_else(|| {
                        node.container
                            .map(|parent| graph.nodes[parent.0 as usize].tala_id)
                    });
                source_parent.is_none().then_some(node.tala_id)
            })
            .collect::<Vec<_>>();
        let mut source_order = Vec::new();
        let mut source_seen = BTreeSet::new();
        for root_tala_id in root_tala_ids {
            source_walk_node(
                graph,
                &source_children,
                root_tala_id,
                &mut source_seen,
                &mut source_order,
            );
        }
        let final_source_order = source_order.clone();
        if !owner_container_walk_applied && !source_order.is_empty() {
            let mut by_stable_id = nodes
                .drain(..)
                .map(|node| (node.stable_id.clone(), node))
                .collect::<BTreeMap<_, _>>();
            let mut reordered = Vec::with_capacity(by_stable_id.len());
            for (_, stable_id) in source_order {
                if let Some(node) = by_stable_id.remove(&stable_id) {
                    reordered.push(node);
                }
            }
            reordered.extend(by_stable_id.into_values());
            nodes = reordered;
            #[cfg(feature = "diagnostic-traces")]
            trace_placement_target_records(phase, root, "source-order-before-aliases", &nodes);
            #[cfg(feature = "diagnostic-traces")]
            trace_placement_order_records(phase, root, "source-order-before-aliases", &nodes);
            // The initial placement callback walks Go's shared Containers
            // pointers, so nested ordinary aliases remain visible even when
            // their external IDs repeat direct siblings.  Reinsert those
            // aliases after their owning node while retaining the exact
            // source order used by the existing projection.
            let mut expanded_parents = BTreeSet::new();
            let mut represented_positions = nodes
                .iter()
                .map(|record| {
                    (
                        record.stable_id.clone(),
                        trace_placement_node_position(record),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            fn append_initial_aliases(
                parent_tala_id: i64,
                phase: &str,
                root: &str,
                graph: &ArenaGraph,
                expanded_parents: &mut BTreeSet<i64>,
                represented_positions: &mut BTreeMap<String, Option<Point>>,
                output: &mut Vec<PlacementTraceNode>,
                parent_position: Option<Point>,
            ) {
                if !expanded_parents.insert(parent_tala_id) {
                    return;
                }
                let Some(children) = graph
                    .transaction_external_container_children
                    .get(&parent_tala_id)
                    .or_else(|| graph.trace_container_children.get(&parent_tala_id))
                else {
                    return;
                };
                // Aggregate members are represented by their vessel in the
                // placement graph and are handled by the existing source
                // reorder.  This source walk is for ordinary nodes and also
                // materializes the direct children of a top-level container.
                let aggregate = |tala_id: i64| {
                    graph.clusters.iter().any(|cluster| {
                        cluster
                            .members
                            .iter()
                            .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
                    }) || graph.sequences.iter().any(|sequence| {
                        sequence
                            .members
                            .iter()
                            .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
                    }) || graph
                        .transaction_external_aggregate_children
                        .values()
                        .any(|members| members.iter().any(|member| member.tala_id == tala_id))
                };
                for child in children {
                    if aggregate(child.tala_id) {
                        continue;
                    }
                    let transaction_snapshot = graph
                        .transaction_external_container_children
                        .values()
                        .flatten()
                        .find(|node| node.tala_id == child.tala_id)
                        .cloned();
                    let snapshot = transaction_snapshot
                        .clone()
                        .or_else(|| {
                            TRACE_SYNCED_NODES
                                .get()
                                .and_then(|nodes| nodes.lock().ok())
                                .and_then(|nodes| nodes.get(&child.tala_id).cloned())
                        })
                        .or_else(|| {
                            graph
                                .nodes
                                .iter()
                                .find(|node| node.tala_id == child.tala_id)
                                .cloned()
                        })
                        .unwrap_or_else(|| child.clone());
                    let projected_position = snapshot.position.map(|point| {
                        if transaction_snapshot.is_some() {
                            point
                        } else {
                            parent_position
                                .map(|parent| {
                                    let padding =
                                        graph.trace_source_container_padding(parent_tala_id);
                                    Point {
                                        x: point.x + parent.x + padding.left,
                                        y: point.y + parent.y + padding.top,
                                    }
                                })
                                .unwrap_or(point)
                        }
                    });
                    let id = projected_vessel_trace_id(graph, snapshot.tala_id)
                        .unwrap_or_else(|| snapshot.external_id.clone());
                    // The recursive root walk may already have emitted this
                    // live node. Re-projecting its source alias adds the
                    // owning container frame a second time; keep the existing
                    // trace position while still traversing its children.
                    let stable_id = snapshot.tala_id.to_string();
                    let position = represented_positions
                        .get(&stable_id)
                        .copied()
                        .unwrap_or(projected_position);
                    #[cfg(feature = "diagnostic-traces")]
                    if crate::internal::trace_env_value("TALA_TRACE_TARGET_NODE")
                        .is_some_and(|target| target == child.tala_id.to_string())
                    {
                        eprintln!(
                            "TRACE_ALIAS_TARGET_RUST phase={} root={} child={} parent={} transaction={} source={:?} parent_position={:?} projected={:?}",
                            phase,
                            root,
                            child.tala_id,
                            parent_tala_id,
                            transaction_snapshot.is_some(),
                            snapshot.position,
                            parent_position,
                            position
                        );
                    }
                    // The key is still needed inside the branch, so the entry API does not fit.
                    #[allow(clippy::map_entry)]
                    if !represented_positions.contains_key(&stable_id) {
                        output.push(PlacementTraceNode {
                            id: id.clone(),
                            stable_id: stable_id.clone(),
                            w_bits: trace_float_bits(snapshot.rect.size.width),
                            h_bits: trace_float_bits(snapshot.rect.size.height),
                            x_bits: position.map(|point| trace_float_bits(point.x)),
                            y_bits: position.map(|point| trace_float_bits(point.y)),
                            fixed_x_bits: snapshot
                                .fixed_top_left
                                .map(|point| trace_float_bits(point.x)),
                            fixed_y_bits: snapshot
                                .fixed_top_left
                                .map(|point| trace_float_bits(point.y)),
                        });
                        represented_positions.insert(stable_id, position);
                    }
                    append_initial_aliases(
                        child.tala_id,
                        phase,
                        root,
                        graph,
                        expanded_parents,
                        represented_positions,
                        output,
                        position,
                    );
                }
            }
            let mut expanded = Vec::with_capacity(nodes.len());
            for record in nodes.drain(..) {
                expanded.push(record.clone());
                let parent_ids = graph
                    .trace_container_children
                    .keys()
                    .chain(graph.transaction_external_container_children.keys())
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .filter(|parent_tala_id| record.stable_id == parent_tala_id.to_string())
                    .collect::<Vec<_>>();
                for parent_tala_id in parent_ids {
                    if crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_MAP") {
                        eprintln!(
                            "TRACE_INITIAL_ALIAS_PARENT_RUST record={} parent={}",
                            record.id, parent_tala_id
                        );
                    }
                    let parent_position = graph
                        .nodes
                        .iter()
                        .find(|node| node.tala_id == parent_tala_id)
                        .and_then(|node| node.position)
                        .or_else(|| {
                            graph
                                .trace_container_children
                                .values()
                                .flatten()
                                .find(|node| node.tala_id == parent_tala_id)
                                .and_then(|node| node.position)
                        });
                    append_initial_aliases(
                        parent_tala_id,
                        phase,
                        root,
                        graph,
                        &mut expanded_parents,
                        &mut represented_positions,
                        &mut expanded,
                        parent_position,
                    );
                }
            }
            nodes = expanded;
            #[cfg(feature = "diagnostic-traces")]
            trace_placement_target_records(phase, root, "after-alias-materialization", &nodes);
            #[cfg(feature = "diagnostic-traces")]
            trace_placement_order_records(phase, root, "after-alias-materialization", &nodes);
            // Alias materialization above may append tree-restored children
            // after the dense records they were absent from.  Reapply the
            // owning Graph.Nodes order once more so those aliases occupy the
            // same slots as Go's final root snapshot (e.g. gen/adapter stay
            // immediately after resolvers.pathom).
            let mut by_stable_id = nodes
                .drain(..)
                .map(|node| (node.stable_id.clone(), node))
                .collect::<BTreeMap<_, _>>();
            let mut ordered = Vec::with_capacity(by_stable_id.len());
            for (_, stable_id) in &final_source_order {
                if let Some(node) = by_stable_id.remove(stable_id) {
                    ordered.push(node);
                }
            }
            ordered.extend(by_stable_id.into_values());
            nodes = ordered;
            #[cfg(feature = "diagnostic-traces")]
            trace_placement_target_records(phase, root, "source-alias-reorder", &nodes);
            if crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_MAP") {
                eprintln!(
                    "TRACE_INITIAL_REORDER_RESULT_RUST {:?}",
                    nodes
                        .iter()
                        .take(20)
                        .map(|node| node.id.clone())
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    let records_match_visible_owner_walk = nodes.len() == trace_nodes.len()
        && nodes.iter().zip(&trace_nodes).all(|(record, node_id)| {
            record.stable_id == graph.nodes[node_id.0 as usize].tala_id.to_string()
        });
    #[cfg(feature = "diagnostic-traces")]
    let records_cover_visible_owner_descendants = {
        let root_tala_id = graph
            .nodes
            .iter()
            .find(|node| node.external_id == root)
            .map(|node| node.tala_id);
        let root_is_top = root.is_empty() || root == "top";
        let mut visible = trace_nodes
            .iter()
            .map(|node_id| graph.nodes[node_id.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        let mut pending = visible.iter().copied().collect::<Vec<_>>();
        while let Some(parent_tala_id) = pending.pop() {
            for child_tala_id in
                trace_owner_phase_child_ids(graph, parent_tala_id, phase, root_tala_id, root_is_top)
            {
                if visible.insert(child_tala_id) {
                    pending.push(child_tala_id);
                }
            }
        }
        visible.iter().all(|tala_id| {
            nodes
                .iter()
                .any(|record| record.stable_id == tala_id.to_string())
        })
    };
    #[cfg(feature = "diagnostic-traces")]
    if !root.is_empty()
        && !(records_match_visible_owner_walk && records_cover_visible_owner_descendants)
    {
        // An induced placement graph contains only the direct children of its
        // scope, while Go's shared Containers map still exposes every nested
        // descendant to tracePlacement. Materialize those projected aliases
        // for diagnostics and order them depth-first below their owner. If
        // the placement records already match the stable owner walk exactly,
        // keep that complete snapshot intact; rebuilding it by display ID
        // would collapse distinct nested nodes whose local labels repeat.
        let mut by_id = nodes
            .drain(..)
            .map(|node| (node.id.clone(), node))
            .collect::<BTreeMap<_, _>>();
        let visible_parents = trace_nodes
            .iter()
            .map(|node_id| graph.nodes[node_id.0 as usize].tala_id)
            .collect::<BTreeSet<_>>();
        // Materialize the complete descendant tree from every visible owner.
        // A scope graph usually contains only the immediate child vessel, but
        // Go's shared Containers map makes grandchildren visible to the trace
        // as well.  The first version only inserted one map level, leaving
        // records such as `first.child` out of nested-grid snapshots.
        fn append_projected_trace_records(
            parent_tala_id: i64,
            graph: &ArenaGraph,
            by_id: &mut BTreeMap<String, PlacementTraceNode>,
            use_synced: bool,
            after_fit_root_tala: Option<i64>,
        ) {
            // A retained descendant of an active cluster/sequence member is
            // stored in the member's local coordinate system.  Go's shared
            // vessel pointer exposes that subtree in the enclosing graph's
            // coordinate system, so add the projected vessel origin while
            // materializing diagnostic aliases.  The arena keeps the
            // member and vessel separate; this translation remains confined
            // to the trace projection.
            fn aggregate_ancestor_offset(
                parent_tala_id: i64,
                graph: &ArenaGraph,
                use_synced: bool,
            ) -> Point {
                if !use_synced {
                    return Point::default();
                }
                let mut current = Some(parent_tala_id);
                let mut seen = BTreeSet::new();
                let mut offset = Point::default();
                while let Some(tala_id) = current {
                    if !seen.insert(tala_id) {
                        break;
                    }
                    for (cluster_index, cluster) in graph.clusters.iter().enumerate() {
                        let is_member = cluster
                            .members
                            .iter()
                            .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id);
                        if !is_member {
                            continue;
                        }
                        if let Some(position) = graph
                            .pending_cluster_vessel_positions
                            .get(&cluster_index)
                            .copied()
                            .or_else(|| {
                                cluster
                                    .members
                                    .first()
                                    .and_then(|member| graph.active_node_position(*member))
                            })
                        {
                            offset.x += position.x;
                            offset.y += position.y;
                        }
                    }
                    for sequence in &graph.sequences {
                        let is_member = sequence
                            .members
                            .iter()
                            .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id);
                        if !is_member {
                            continue;
                        }
                        if let Some(position) = sequence
                            .members
                            .first()
                            .and_then(|member| graph.active_node_position(*member))
                        {
                            offset.x += position.x;
                            offset.y += position.y;
                        }
                    }
                    current = graph
                        .source_parent_tala_by_tala
                        .get(&tala_id)
                        .copied()
                        .flatten();
                }
                offset
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_MAP")
                && parent_tala_id == 3677471320
            {
                eprintln!(
                    "PROJECTED_MAP_RUST parent={} after_fit={} clusters={} child_ids={:?}",
                    parent_tala_id,
                    use_synced,
                    graph.clusters.len(),
                    graph
                        .trace_container_children
                        .get(&parent_tala_id)
                        .map(|children| children
                            .iter()
                            .map(|child| child.tala_id)
                            .collect::<Vec<_>>())
                );
            }
            let Some(children) = graph
                .transaction_external_container_children
                .get(&parent_tala_id)
                .or_else(|| graph.trace_container_children.get(&parent_tala_id))
            else {
                return;
            };
            for child in children {
                // The retained child may already be the synthetic vessel,
                // rather than the first member used as an arena carrier.
                // Preserve its exact box and identify it by the aggregate key.
                if let Some(id) = projected_vessel_trace_id(graph, child.tala_id) {
                    by_id.remove(&child.external_id);
                    by_id.insert(
                        id.clone(),
                        PlacementTraceNode {
                            id,
                            stable_id: child.tala_id.to_string(),
                            w_bits: trace_float_bits(child.rect.size.width),
                            h_bits: trace_float_bits(child.rect.size.height),
                            x_bits: child.position.map(|point| trace_float_bits(point.x)),
                            y_bits: child.position.map(|point| trace_float_bits(point.y)),
                            fixed_x_bits: child
                                .fixed_top_left
                                .map(|point| trace_float_bits(point.x)),
                            fixed_y_bits: child
                                .fixed_top_left
                                .map(|point| trace_float_bits(point.y)),
                        },
                    );
                    continue;
                }

                if let Some((vessel_id, members)) = graph
                    .transaction_external_aggregate_children
                    .iter()
                    .find(|(_, members)| {
                        members
                            .first()
                            .is_some_and(|member| member.tala_id == child.tala_id)
                    })
                {
                    if crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_MAP") {
                        eprintln!(
                            "PROJECTED_TXN_BRANCH_RUST parent={} child={} after_fit={} vessel={} members={}",
                            parent_tala_id,
                            child.tala_id,
                            use_synced,
                            vessel_id,
                            members.len()
                        );
                    }
                    let layout = graph
                        .transaction_external_cluster_layouts
                        .get(vessel_id)
                        .copied();
                    let (width, height, arrangement) = if let Some(layout) = layout {
                        let maximum =
                            members
                                .iter()
                                .fold(Size::default(), |maximum, member| Size {
                                    width: maximum.width.max(member.rect.size.width),
                                    height: maximum.height.max(member.rect.size.height),
                                });
                        let gaps = layout.padding * members.len().saturating_sub(1) as f64;
                        match layout.arrangement {
                            ClusterArrangement::Row => (
                                members
                                    .iter()
                                    .map(|member| member.rect.size.width)
                                    .sum::<f64>()
                                    + gaps,
                                maximum.height,
                                "Row",
                            ),
                            ClusterArrangement::Column => (
                                maximum.width,
                                members
                                    .iter()
                                    .map(|member| member.rect.size.height)
                                    .sum::<f64>()
                                    + gaps,
                                "Column",
                            ),
                        }
                    } else {
                        (
                            members
                                .iter()
                                .map(|member| member.rect.size.width - 35.0)
                                .sum::<f64>()
                                + 35.0,
                            members
                                .iter()
                                .map(|member| member.rect.size.height)
                                .fold(0.0, f64::max),
                            "Column",
                        )
                    };
                    let member_names = members
                        .iter()
                        .map(|member| member.external_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let id = if layout.is_some() {
                        format!("Cluster vessel of: [{member_names}]; Arrangement: {arrangement}")
                    } else {
                        format!("Sequence vessel of: [{member_names}]")
                    };
                    let record_id = id.clone();
                    let mut position =
                        trace_node_snapshot(graph, child.tala_id, child, false, false, use_synced)
                            .position;
                    if use_synced
                        && after_fit_root_tala.is_some_and(|root_tala_id| {
                            parent_tala_id != root_tala_id
                                && graph.source_is_descendant_of_tala(parent_tala_id, root_tala_id)
                        })
                    {
                        if graph
                            .nodes
                            .iter()
                            .any(|node| node.tala_id == parent_tala_id)
                        {
                            let padding = graph.trace_source_container_padding(parent_tala_id);
                            let parent_position = graph
                                .nodes
                                .iter()
                                .find(|node| node.tala_id == parent_tala_id)
                                .and_then(|parent| {
                                    TRACE_SYNCED_NODES
                                        .get()
                                        .and_then(|synced| synced.lock().ok())
                                        .and_then(|synced| synced.get(&parent.tala_id).cloned())
                                        .and_then(|parent| parent.position)
                                        .or_else(|| graph.active_node_position(parent.input_id))
                                })
                                .unwrap_or_default();
                            position = position.map(|point| Point {
                                x: point.x + parent_position.x + padding.left,
                                y: point.y + parent_position.y + padding.top,
                            });
                        }
                    }
                    by_id.entry(id).or_insert_with(|| PlacementTraceNode {
                        id: record_id,
                        stable_id: vessel_id.to_string(),
                        w_bits: trace_float_bits(width),
                        h_bits: trace_float_bits(height),
                        x_bits: position.map(|point| trace_float_bits(point.x)),
                        y_bits: position.map(|point| trace_float_bits(point.y)),
                        fixed_x_bits: child.fixed_top_left.map(|point| trace_float_bits(point.x)),
                        fixed_y_bits: child.fixed_top_left.map(|point| trace_float_bits(point.y)),
                    });
                    if use_synced {
                        append_projected_trace_records(
                            child.tala_id,
                            graph,
                            by_id,
                            use_synced,
                            after_fit_root_tala,
                        );
                    }
                    continue;
                }
                // The root graph presents active cluster/sequence members as
                // one vessel. Those aliases remain in the transaction
                // projection only for later copyback and must stay out of a
                // root placement snapshot.
                let aggregate_member =
                    graph
                        .transaction_external_aggregate_children
                        .values()
                        .any(|members| {
                            members
                                .iter()
                                .skip(1)
                                .any(|member| member.tala_id == child.tala_id)
                        })
                        || graph.clusters.iter().any(|cluster| {
                            cluster.members.iter().any(|member| {
                                graph.nodes[member.0 as usize].tala_id == child.tala_id
                            }) && cluster.members.first().is_none_or(|carrier| {
                                graph.nodes[carrier.0 as usize].tala_id != child.tala_id
                            })
                        })
                        || graph.sequences.iter().any(|sequence| {
                            sequence.members.iter().any(|member| {
                                graph.nodes[member.0 as usize].tala_id == child.tala_id
                            }) && sequence.members.first().is_none_or(|carrier| {
                                graph.nodes[carrier.0 as usize].tala_id != child.tala_id
                            })
                        });
                if parent_tala_id == 3677471320
                    && child.tala_id == 3946625030
                    && crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_MAP")
                {
                    eprintln!(
                        "PROJECTED_CHILD_RUST aggregate_member={} clusters={:?}",
                        aggregate_member,
                        graph
                            .clusters
                            .iter()
                            .map(|cluster| {
                                cluster
                                    .members
                                    .iter()
                                    .map(|member| graph.nodes[member.0 as usize].tala_id)
                                    .collect::<Vec<_>>()
                            })
                            .collect::<Vec<_>>()
                    );
                    if !use_synced {
                        eprintln!(
                            "PROJECTED_TXN_RUST aggregates={:?} layouts={:?}",
                            graph
                                .transaction_external_aggregate_children
                                .iter()
                                .map(|(id, members)| (
                                    *id,
                                    members
                                        .iter()
                                        .map(|member| {
                                            (
                                                member.tala_id,
                                                member.rect.size.width,
                                                member.rect.size.height,
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                ))
                                .collect::<Vec<_>>(),
                            graph
                                .transaction_external_cluster_layouts
                                .iter()
                                .map(|(id, layout)| (*id, layout.arrangement, layout.padding))
                                .collect::<Vec<_>>()
                        );
                    }
                }
                if aggregate_member {
                    continue;
                }
                if let Some((cluster_index, cluster)) =
                    graph.clusters.iter().enumerate().find(|(_, cluster)| {
                        cluster.members.first().is_some_and(|carrier| {
                            graph.nodes[carrier.0 as usize].tala_id == child.tala_id
                        })
                    })
                {
                    if crate::internal::trace_env_enabled("TALA_TRACE_VESSEL_PROJECTION") {
                        eprintln!(
                            "VESSEL_PROJECTION_RUST parent={} child={} cluster_index={} after_fit={}",
                            parent_tala_id, child.tala_id, cluster_index, use_synced
                        );
                    }
                    let arrangement = match cluster.arrangement {
                        ClusterArrangement::Row => "Row",
                        ClusterArrangement::Column => "Column",
                    };
                    let members = cluster
                        .members
                        .iter()
                        .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let id = format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}");
                    let record_id = id.clone();
                    let size = graph.cluster_vessel_size(cluster_index);
                    let position = graph.active_node_position(child.input_id);
                    by_id.entry(id).or_insert_with(|| PlacementTraceNode {
                        id: record_id,
                        stable_id: cluster.vessel_tala_id.to_string(),
                        w_bits: trace_float_bits(size.width),
                        h_bits: trace_float_bits(size.height),
                        x_bits: position.map(|point| trace_float_bits(point.x)),
                        y_bits: position.map(|point| trace_float_bits(point.y)),
                        fixed_x_bits: child.fixed_top_left.map(|point| trace_float_bits(point.x)),
                        fixed_y_bits: child.fixed_top_left.map(|point| trace_float_bits(point.y)),
                    });
                    if use_synced {
                        append_projected_trace_records(
                            child.tala_id,
                            graph,
                            by_id,
                            use_synced,
                            after_fit_root_tala,
                        );
                    }
                    continue;
                }
                let mut child =
                    trace_node_snapshot(graph, child.tala_id, child, false, false, use_synced);
                // Go's callback prints shared node pointers as they currently
                // stand. `trace_node_snapshot` has already selected the
                // synced pointer coordinate; projecting it through an active
                // aggregate vessel would count that offset twice.
                let aggregate_offset = if after_fit_root_tala.is_some() {
                    Point::default()
                } else {
                    aggregate_ancestor_offset(parent_tala_id, graph, use_synced)
                };
                if let Some(position) = child.position {
                    child.position = Some(Point {
                        x: position.x + aggregate_offset.x,
                        y: position.y + aggregate_offset.y,
                    });
                }
                let projected = PlacementTraceNode {
                    id: child.external_id.clone(),
                    stable_id: child.tala_id.to_string(),
                    w_bits: trace_float_bits(child.rect.size.width),
                    h_bits: trace_float_bits(child.rect.size.height),
                    x_bits: child.position.map(|point| trace_float_bits(point.x)),
                    y_bits: child.position.map(|point| trace_float_bits(point.y)),
                    fixed_x_bits: child.fixed_top_left.map(|point| trace_float_bits(point.x)),
                    fixed_y_bits: child.fixed_top_left.map(|point| trace_float_bits(point.y)),
                };
                if aggregate_offset != Point::default() {
                    // `visit` may have already emitted this retained alias
                    // from the dense arena. Replace that local-coordinate
                    // record with the projected shared-pointer position.
                    by_id.insert(child.external_id.clone(), projected);
                } else {
                    by_id.entry(child.external_id.clone()).or_insert(projected);
                }
                append_projected_trace_records(
                    child.tala_id,
                    graph,
                    by_id,
                    use_synced,
                    after_fit_root_tala,
                );
            }
        }
        for parent in visible_parents {
            let parent_is_aggregate_carrier = graph.clusters.iter().any(|cluster| {
                cluster
                    .members
                    .first()
                    .is_some_and(|member| graph.nodes[member.0 as usize].tala_id == parent)
            }) || graph.sequences.iter().any(|sequence| {
                sequence
                    .members
                    .first()
                    .is_some_and(|member| graph.nodes[member.0 as usize].tala_id == parent)
            });
            if root_graph_snapshot && parent_is_aggregate_carrier {
                // In Go the vessel replaces the aggregate members in
                // Graph.Nodes, and the live Containers map has no member
                // descendants below that vessel. Rust keeps the first member
                // as its stable carrier, so don't expose the carrier's source
                // children when materializing this root.Graph view.
                continue;
            }
            // `trace_container_children` also retains the stable arena's
            // member aliases for active aggregates.  At Go's after-fit
            // boundary those members have been replaced by the vessel in
            // Graph.Nodes, so they must not be projected a second time.
            let aggregate_children =
                graph
                    .trace_container_children
                    .get(&parent)
                    .is_some_and(|children| {
                        !children.is_empty()
                            && children.iter().all(|child| {
                                graph.clusters.iter().any(|cluster| {
                                    graph.cluster_is_active(cluster)
                                        && cluster.members.iter().any(|member| {
                                            graph.nodes[member.0 as usize].tala_id == child.tala_id
                                        })
                                }) || graph.sequences.iter().any(|sequence| {
                                    graph.sequence_is_active(sequence)
                                        && sequence.members.iter().any(|member| {
                                            graph.nodes[member.0 as usize].tala_id == child.tala_id
                                        })
                                })
                            })
                    });
            if phase == "after-fit" && aggregate_children {
                continue;
            }
            append_projected_trace_records(
                parent,
                graph,
                &mut by_id,
                phase == "after-fit",
                after_fit_root.map(|root_id| graph.nodes[root_id.0 as usize].tala_id),
            );
        }
        if root == "2043751575"
            && phase == "initialized"
            && crate::internal::trace_env_enabled("TALA_TRACE_PROJECTED_MAP")
        {
            eprintln!(
                "PROJECTED_BY_ID_RUST trace_nodes={:?} {:?}",
                trace_nodes
                    .iter()
                    .map(|id| graph.nodes[id.0 as usize].tala_id)
                    .collect::<Vec<_>>(),
                by_id.keys().cloned().collect::<Vec<_>>()
            );
        }
        let mut ordered = Vec::with_capacity(by_id.len());
        let mut emitted = BTreeSet::new();
        fn projected_trace_id(graph: &ArenaGraph, tala_id: i64, fallback: &str) -> Option<String> {
            if let Some(id) = projected_vessel_trace_id(graph, tala_id) {
                return Some(id);
            }
            if let Some((vessel_id, members)) = graph
                .transaction_external_aggregate_children
                .iter()
                .find(|(_, members)| members.iter().any(|member| member.tala_id == tala_id))
            {
                let carrier = members.first()?.tala_id == tala_id;
                if !carrier {
                    return None;
                }
                let member_names = members
                    .iter()
                    .map(|member| member.external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Some(layout) = graph.transaction_external_cluster_layouts.get(vessel_id) {
                    let arrangement = match layout.arrangement {
                        ClusterArrangement::Row => "Row",
                        ClusterArrangement::Column => "Column",
                    };
                    return Some(format!(
                        "Cluster vessel of: [{member_names}]; Arrangement: {arrangement}"
                    ));
                }
                return Some(format!("Sequence vessel of: [{member_names}]"));
            }
            if let Some(cluster) = graph.clusters.iter().find(|cluster| {
                graph.cluster_is_active(cluster)
                    && cluster
                        .members
                        .iter()
                        .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
            }) {
                let &carrier = cluster.members.first()?;
                if graph.nodes[carrier.0 as usize].tala_id != tala_id {
                    return None;
                }
                let arrangement = match cluster.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                };
                let members = cluster
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some(format!(
                    "Cluster vessel of: [{members}]; Arrangement: {arrangement}"
                ));
            }
            if let Some(sequence) = graph.sequences.iter().find(|sequence| {
                graph.sequence_is_active(sequence)
                    && sequence
                        .members
                        .iter()
                        .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
            }) {
                let &carrier = sequence.members.first()?;
                if graph.nodes[carrier.0 as usize].tala_id != tala_id {
                    return None;
                }
                let members = sequence
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some(format!("Sequence vessel of: [{members}]"));
            }
            Some(fallback.to_owned())
        }
        fn append_projected_trace_node(
            tala_id: i64,
            id: &str,
            by_id: &BTreeMap<String, PlacementTraceNode>,
            graph: &ArenaGraph,
            ordered: &mut Vec<PlacementTraceNode>,
            emitted: &mut BTreeSet<String>,
            phase: &str,
            root_tala_id: Option<i64>,
            root_is_top: bool,
        ) {
            if !emitted.insert(id.to_owned()) {
                return;
            }
            if let Some(node) = by_id.get(id) {
                ordered.push(node.clone());
            }
            let ordered_children = if trace_phase_uses_live_owner_children(phase) {
                trace_owner_phase_child_ids(graph, tala_id, phase, root_tala_id, root_is_top)
            } else {
                trace_snapshot_child_ids(graph, tala_id, false)
            };
            // Carry stable node identity through the walk. Display IDs can
            // repeat across projected aliases, while Go's Containers map is
            // keyed by node pointer.
            for child_tala_id in ordered_children {
                let Some(child) = graph
                    .nodes
                    .iter()
                    .find(|child| child.tala_id == child_tala_id)
                    .or_else(|| {
                        graph
                            .transaction_external_container_children
                            .values()
                            .flatten()
                            .find(|child| child.tala_id == child_tala_id)
                    })
                    .or_else(|| {
                        graph
                            .trace_container_children
                            .values()
                            .flatten()
                            .find(|child| child.tala_id == child_tala_id)
                    })
                else {
                    continue;
                };
                if let Some(child_id) = projected_trace_id(graph, child.tala_id, &child.external_id)
                {
                    append_projected_trace_node(
                        child.tala_id,
                        &child_id,
                        by_id,
                        graph,
                        ordered,
                        emitted,
                        phase,
                        root_tala_id,
                        root_is_top,
                    );
                }
            }
        }
        let root_tala_id = graph
            .nodes
            .iter()
            .find(|node| node.external_id == root)
            .map(|node| node.tala_id);
        let mut root_trace_nodes = if phase == "after-fit" {
            graph
                .containers
                .get(&None)
                .unwrap_or_else(|| trace_nodes.clone())
        } else {
            trace_nodes.clone()
        };
        if phase == "after-fit" {
            // FitToGraph observes the combined graph's current Nodes slice.
            // The stable arena retains the original root-container order,
            // so use node_order for this diagnostic traversal as well. This
            // preserves the position at which Go appends grouping vessels.
            root_trace_nodes.stable_sort_by_key(|node_id| {
                trace_nodes
                    .iter()
                    .position(|candidate| candidate == node_id)
                    .unwrap_or(usize::MAX)
            });
        }
        for node_id in &root_trace_nodes {
            let node = &graph.nodes[node_id.0 as usize];
            let Some(id) = projected_trace_id(graph, node.tala_id, &node.external_id) else {
                continue;
            };
            append_projected_trace_node(
                node.tala_id,
                &id,
                &by_id,
                graph,
                &mut ordered,
                &mut emitted,
                phase,
                root_tala_id,
                root.is_empty() || root == "top",
            );
        }
        for (id, node) in &by_id {
            if emitted.insert(id.clone()) {
                ordered.push(node.clone());
            }
        }

        // At the owning after-fit boundary Go has one shared Containers map
        // for the whole source graph.  CombineSubgraphs replaces aggregate
        // members with a vessel in the visible child slice, but defers the
        // members' descendants until the owning top-level child has finished
        // walking.  Replaying that queue is important for operation-level
        // traces: a simple depth-first walk puts those descendants next to
        // the vessel (and a map-order fallback puts them at the end).
        if phase == "after-fit"
            && (graph
                .clusters
                .iter()
                .any(|cluster| graph.cluster_is_active(cluster))
                || graph
                    .sequences
                    .iter()
                    .any(|sequence| graph.sequence_is_active(sequence)))
            && trace_child_order_is_node_ordered(
                graph,
                graph
                    .nodes
                    .iter()
                    .find(|node| node.external_id == root)
                    .map(|node| node.tala_id)
                    .unwrap_or_default(),
            )
        {
            use std::collections::VecDeque;

            fn aggregate_spec(
                graph: &ArenaGraph,
                tala_id: i64,
            ) -> Option<(String, Vec<i64>, bool)> {
                if let Some(cluster) = graph.clusters.iter().find(|cluster| {
                    graph.cluster_is_active(cluster)
                        && cluster
                            .members
                            .iter()
                            .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
                }) {
                    let members = cluster
                        .members
                        .iter()
                        .map(|member| graph.nodes[member.0 as usize].tala_id)
                        .collect::<Vec<_>>();
                    let carrier = members.first().copied() == Some(tala_id);
                    let arrangement = match cluster.arrangement {
                        ClusterArrangement::Row => "Row",
                        ClusterArrangement::Column => "Column",
                    };
                    let member_text = members
                        .iter()
                        .filter_map(|id| {
                            graph
                                .nodes
                                .iter()
                                .find(|node| node.tala_id == *id)
                                .map(|node| node.external_id.as_str())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Some((
                        format!("Cluster vessel of: [{member_text}]; Arrangement: {arrangement}"),
                        members,
                        carrier,
                    ));
                }
                if let Some(sequence) = graph.sequences.iter().find(|sequence| {
                    graph.sequence_is_active(sequence)
                        && sequence
                            .members
                            .iter()
                            .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
                }) {
                    let members = sequence
                        .members
                        .iter()
                        .map(|member| graph.nodes[member.0 as usize].tala_id)
                        .collect::<Vec<_>>();
                    let carrier = members.first().copied() == Some(tala_id);
                    let member_text = members
                        .iter()
                        .filter_map(|id| {
                            graph
                                .nodes
                                .iter()
                                .find(|node| node.tala_id == *id)
                                .map(|node| node.external_id.as_str())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Some((
                        format!("Sequence vessel of: [{member_text}]"),
                        members,
                        carrier,
                    ));
                }
                None
            }

            fn source_record_id(graph: &ArenaGraph, tala_id: i64) -> Option<String> {
                if let Some((id, _, carrier)) = aggregate_spec(graph, tala_id) {
                    return carrier.then_some(id);
                }
                graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == tala_id)
                    .map(|node| node.external_id.clone())
                    .or_else(|| {
                        graph
                            .trace_container_children
                            .values()
                            .flatten()
                            .find(|node| node.tala_id == tala_id)
                            .map(|node| node.external_id.clone())
                    })
            }

            fn walk_after_fit(
                tala_id: i64,
                graph: &ArenaGraph,
                by_id: &BTreeMap<String, PlacementTraceNode>,
                ordered: &mut Vec<PlacementTraceNode>,
                emitted: &mut BTreeSet<String>,
                deferred: &mut VecDeque<i64>,
            ) {
                if let Some((vessel_id, members, carrier)) = aggregate_spec(graph, tala_id) {
                    if !carrier {
                        return;
                    }
                    if emitted.insert(vessel_id.clone())
                        && let Some(node) = by_id.get(&vessel_id)
                    {
                        ordered.push(node.clone());
                    }
                    // Descendants are queued in member order and are drained
                    // only after the current owning child is complete.
                    for member in members {
                        deferred.extend(trace_snapshot_child_ids(graph, member, true));
                    }
                    return;
                }
                let Some(id) = source_record_id(graph, tala_id) else {
                    return;
                };
                if emitted.insert(id.clone())
                    && let Some(node) = by_id.get(&id)
                {
                    ordered.push(node.clone());
                }
                let mut children = trace_snapshot_child_ids(graph, tala_id, true);
                trace_append_cluster_vessels(graph, &mut children);
                for child_tala_id in children {
                    walk_after_fit(child_tala_id, graph, by_id, ordered, emitted, deferred);
                }
            }

            let inferred_root_children = if graph.trace_container_children.contains_key(&0) {
                graph
                    .trace_container_children
                    .get(&0)
                    .cloned()
                    .unwrap_or_default()
            } else {
                // Induced placement scopes do not retain the nil-container
                // entry. Recover the source roots as nodes that never occur
                // as a child, preserving the owning Graph.Nodes order.
                let nested_ids = graph
                    .trace_container_children
                    .values()
                    .flatten()
                    .map(|node| node.tala_id)
                    .collect::<BTreeSet<_>>();
                graph
                    .node_order
                    .iter()
                    .map(|node_id| &graph.nodes[node_id.0 as usize])
                    .filter(|node| !nested_ids.contains(&node.tala_id))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            if !inferred_root_children.is_empty() {
                let mut source_order = Vec::with_capacity(by_id.len());
                let mut source_emitted = BTreeSet::new();
                for child in &inferred_root_children {
                    let mut deferred = VecDeque::new();
                    walk_after_fit(
                        child.tala_id,
                        graph,
                        &by_id,
                        &mut source_order,
                        &mut source_emitted,
                        &mut deferred,
                    );
                    while let Some(deferred_id) = deferred.pop_front() {
                        walk_after_fit(
                            deferred_id,
                            graph,
                            &by_id,
                            &mut source_order,
                            &mut source_emitted,
                            &mut deferred,
                        );
                    }
                }
                // Keep this projection lossless if a fixture contains an
                // alias absent from the source hierarchy.  Normal v0.9.0
                // graphs take the exact branch above.
                for (id, node) in &by_id {
                    if source_emitted.insert(id.clone()) {
                        source_order.push(node.clone());
                    }
                }
                if source_order.len() == by_id.len() {
                    ordered = source_order;
                }
            }

            // Go's placement graph keeps shared pointers for nested ordinary
            // nodes.  Their external IDs may intentionally repeat a sibling
            // in the owning graph (for example two scopes each containing
            // nodes named `0`, `1`, ...).  The stable Rust projection uses a
            // string-keyed map for the normal path, so those aliases collapse
            // to one record.  Reinsert the lossless nested pointer records at
            // the owning node's position in the after-fit snapshot.  This is
            // diagnostic-only and is limited to ordinary duplicate-ID maps;
            // aggregate vessels continue through the existing projection.
            let top_level = graph
                .nodes
                .iter()
                .filter(|node| {
                    graph
                        .source_parent_tala_by_tala
                        .get(&node.tala_id)
                        .copied()
                        .flatten()
                        .is_none()
                })
                .map(|node| node.tala_id)
                .collect::<BTreeSet<_>>();
            let mut expanded_parents = BTreeSet::new();
            fn is_aggregate_alias(graph: &ArenaGraph, tala_id: i64) -> bool {
                graph.clusters.iter().any(|cluster| {
                    cluster
                        .members
                        .iter()
                        .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
                }) || graph.sequences.iter().any(|sequence| {
                    sequence
                        .members
                        .iter()
                        .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
                }) || graph
                    .transaction_external_aggregate_children
                    .values()
                    .any(|members| members.iter().any(|member| member.tala_id == tala_id))
            }
            fn append_duplicate_aliases(
                parent_tala_id: i64,
                graph: &ArenaGraph,
                _by_id: &BTreeMap<String, PlacementTraceNode>,
                top_level: &BTreeSet<i64>,
                expanded_parents: &mut BTreeSet<i64>,
                output: &mut Vec<PlacementTraceNode>,
            ) {
                if top_level.contains(&parent_tala_id) || !expanded_parents.insert(parent_tala_id) {
                    return;
                }
                let Some(children) = graph.trace_container_children.get(&parent_tala_id) else {
                    return;
                };
                let duplicate_ids = children.iter().any(|child| {
                    !is_aggregate_alias(graph, child.tala_id)
                        && graph
                            .nodes
                            .iter()
                            .filter(|node| node.external_id == child.external_id)
                            .count()
                            > 1
                });
                if !duplicate_ids
                    || children
                        .iter()
                        .any(|child| is_aggregate_alias(graph, child.tala_id))
                {
                    return;
                }
                for child in children {
                    let snapshot =
                        trace_node_snapshot(graph, child.tala_id, child, false, false, true);
                    output.push(PlacementTraceNode {
                        id: snapshot.external_id.clone(),
                        stable_id: snapshot.tala_id.to_string(),
                        w_bits: trace_float_bits(snapshot.rect.size.width),
                        h_bits: trace_float_bits(snapshot.rect.size.height),
                        x_bits: snapshot.position.map(|point| trace_float_bits(point.x)),
                        y_bits: snapshot.position.map(|point| trace_float_bits(point.y)),
                        fixed_x_bits: snapshot
                            .fixed_top_left
                            .map(|point| trace_float_bits(point.x)),
                        fixed_y_bits: snapshot
                            .fixed_top_left
                            .map(|point| trace_float_bits(point.y)),
                    });
                    append_duplicate_aliases(
                        child.tala_id,
                        graph,
                        _by_id,
                        top_level,
                        expanded_parents,
                        output,
                    );
                }
            }
            if phase == "after-fit" {
                let mut expanded = Vec::with_capacity(ordered.len());
                for record in ordered {
                    expanded.push(record.clone());
                    let parent_ids = graph
                        .trace_container_children
                        .keys()
                        .copied()
                        .filter(|parent_tala_id| {
                            !top_level.contains(parent_tala_id)
                                && graph.nodes.iter().any(|node| {
                                    node.tala_id == *parent_tala_id && node.external_id == record.id
                                })
                        })
                        .collect::<Vec<_>>();
                    for parent_tala_id in parent_ids {
                        append_duplicate_aliases(
                            parent_tala_id,
                            graph,
                            &by_id,
                            &top_level,
                            &mut expanded_parents,
                            &mut expanded,
                        );
                    }
                }
                ordered = expanded;

                // When an aggregate member owns ordinary descendants, Go's
                // shared pointer graph visits those descendants before the
                // aggregate vessel is appended to the enclosing graph.  The
                // arena projection initially emits the vessel with the
                // visible roots, then materializes the member descendants;
                // replay that source order for this diagnostic boundary.
                if let Some(source_roots) = graph.containers.get(&None) {
                    let is_aggregate_member = |tala_id: i64| {
                        graph
                            .transaction_external_aggregate_children
                            .values()
                            .any(|members| members.iter().any(|member| member.tala_id == tala_id))
                            || graph.clusters.iter().any(|cluster| {
                                cluster
                                    .members
                                    .iter()
                                    .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
                            })
                            || graph.sequences.iter().any(|sequence| {
                                sequence
                                    .members
                                    .iter()
                                    .any(|member| graph.nodes[member.0 as usize].tala_id == tala_id)
                            })
                    };
                    let has_aggregate_descendants = source_roots.iter().any(|node_id| {
                        let tala_id = graph.nodes[node_id.0 as usize].tala_id;
                        is_aggregate_member(tala_id)
                            && graph.trace_container_children.contains_key(&tala_id)
                    });
                    let root_tala_id = graph
                        .nodes
                        .iter()
                        .find(|node| node.external_id == root)
                        .map(|node| node.tala_id)
                        .unwrap_or_default();
                    if has_aggregate_descendants
                        && trace_child_order_is_node_ordered(graph, root_tala_id)
                    {
                        let mut source_order = Vec::with_capacity(ordered.len());
                        let mut source_emitted = BTreeSet::new();
                        fn append_member_descendants(
                            parent_tala_id: i64,
                            graph: &ArenaGraph,
                            emitted: &mut BTreeSet<String>,
                            output: &mut Vec<PlacementTraceNode>,
                        ) {
                            let Some(children) =
                                graph.trace_container_children.get(&parent_tala_id)
                            else {
                                return;
                            };
                            for child in children {
                                if graph.transaction_external_aggregate_children.values().any(
                                    |members| {
                                        members.iter().any(|member| member.tala_id == child.tala_id)
                                    },
                                ) {
                                    continue;
                                }
                                let snapshot = trace_node_snapshot(
                                    graph,
                                    child.tala_id,
                                    child,
                                    false,
                                    false,
                                    true,
                                );
                                output.push(PlacementTraceNode {
                                    id: snapshot.external_id.clone(),
                                    stable_id: snapshot.tala_id.to_string(),
                                    w_bits: trace_float_bits(snapshot.rect.size.width),
                                    h_bits: trace_float_bits(snapshot.rect.size.height),
                                    x_bits: snapshot
                                        .position
                                        .map(|point| trace_float_bits(point.x)),
                                    y_bits: snapshot
                                        .position
                                        .map(|point| trace_float_bits(point.y)),
                                    fixed_x_bits: snapshot
                                        .fixed_top_left
                                        .map(|point| trace_float_bits(point.x)),
                                    fixed_y_bits: snapshot
                                        .fixed_top_left
                                        .map(|point| trace_float_bits(point.y)),
                                });
                                emitted.insert(snapshot.external_id.clone());
                                append_member_descendants(child.tala_id, graph, emitted, output);
                            }
                        }
                        for source_id in source_roots {
                            let source = &graph.nodes[source_id.0 as usize];
                            if is_aggregate_member(source.tala_id) {
                                append_member_descendants(
                                    source.tala_id,
                                    graph,
                                    &mut source_emitted,
                                    &mut source_order,
                                );
                            } else {
                                // Ordinary roots still visit their Containers
                                // subtree immediately. Special handling for
                                // an aggregate sibling must not defer them.
                                let mut deferred = VecDeque::new();
                                walk_after_fit(
                                    source.tala_id,
                                    graph,
                                    &by_id,
                                    &mut source_order,
                                    &mut source_emitted,
                                    &mut deferred,
                                );
                                while let Some(child) = deferred.pop_front() {
                                    walk_after_fit(
                                        child,
                                        graph,
                                        &by_id,
                                        &mut source_order,
                                        &mut source_emitted,
                                        &mut deferred,
                                    );
                                }
                            }
                        }
                        for record in ordered.iter().cloned() {
                            if source_emitted.insert(record.id.clone()) {
                                source_order.push(record);
                            }
                        }
                        if source_order.len() == ordered.len() {
                            ordered = source_order;
                        }
                    }
                }
            }
        }
        nodes = ordered;
        #[cfg(feature = "diagnostic-traces")]
        trace_placement_target_records(phase, root, "root-source-walk", &nodes);
    }
    #[cfg(feature = "diagnostic-traces")]
    if root.is_empty() {
        // The root graph can retain completed container children in the
        // transaction projection. Go's shared Containers map exposes those
        // children only when their parent is present in the current
        // Graph.Nodes slice; preserve that root-level projection here.
        let mut seen_external = nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<BTreeSet<_>>();
        fn append_external_trace_nodes(
            parent_tala_id: i64,
            graph: &ArenaGraph,
            seen: &mut BTreeSet<String>,
            nodes: &mut Vec<PlacementTraceNode>,
            use_synced: bool,
            use_live_container_order: bool,
        ) {
            let children =
                trace_snapshot_child_ids(graph, parent_tala_id, use_live_container_order);
            if children.is_empty() {
                return;
            }
            let mut deferred_vessels = Vec::new();
            for child_tala_id in children {
                let Some(child_node) = graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == child_tala_id)
                    .or_else(|| {
                        graph
                            .transaction_external_container_children
                            .values()
                            .flatten()
                            .find(|node| node.tala_id == child_tala_id)
                    })
                    .or_else(|| {
                        graph
                            .trace_container_children
                            .values()
                            .flatten()
                            .find(|node| node.tala_id == child_tala_id)
                    })
                    .or_else(|| {
                        graph
                            .transaction_external_aggregate_children
                            .values()
                            .flatten()
                            .find(|node| node.tala_id == child_tala_id)
                    })
                else {
                    continue;
                };
                // Active sequence members have been replaced in the owning
                // Go graph by the sequence vessel. The stable arena retains
                // those member IDs and the source child cache can still list
                // them, but they are copyback aliases rather than visible
                // children for this owner snapshot.
                if graph.sequences.iter().any(|sequence| {
                    sequence
                        .members
                        .iter()
                        .any(|member| graph.nodes[member.0 as usize].tala_id == child_tala_id)
                }) {
                    continue;
                }
                if let Some((vessel_id, members)) = graph
                    .transaction_external_aggregate_children
                    .iter()
                    .find(|(_, members)| {
                        members.iter().any(|member| member.tala_id == child_tala_id)
                    })
                {
                    let Some(carrier) = members.first() else {
                        continue;
                    };
                    if carrier.tala_id != child_tala_id {
                        continue;
                    }
                    let layout = graph
                        .transaction_external_cluster_layouts
                        .get(vessel_id)
                        .copied();
                    let id = if let Some(layout) = layout {
                        let arrangement = match layout.arrangement {
                            ClusterArrangement::Row => "Row",
                            ClusterArrangement::Column => "Column",
                        };
                        let members_text = members
                            .iter()
                            .map(|member| member.external_id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("Cluster vessel of: [{members_text}]; Arrangement: {arrangement}")
                    } else {
                        let members_text = members
                            .iter()
                            .map(|member| member.external_id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("Sequence vessel of: [{members_text}]")
                    };
                    if !seen.insert(id.clone()) {
                        continue;
                    }
                    let (width, height) = if let Some(layout) = layout {
                        match layout.arrangement {
                            ClusterArrangement::Row => (
                                members
                                    .iter()
                                    .map(|member| member.rect.size.width)
                                    .sum::<f64>()
                                    + layout.padding * (members.len().saturating_sub(1) as f64),
                                members
                                    .iter()
                                    .map(|member| member.rect.size.height)
                                    .fold(0.0, f64::max),
                            ),
                            ClusterArrangement::Column => (
                                members
                                    .iter()
                                    .map(|member| member.rect.size.width)
                                    .fold(0.0, f64::max),
                                members
                                    .iter()
                                    .map(|member| member.rect.size.height)
                                    .sum::<f64>()
                                    + layout.padding * (members.len().saturating_sub(1) as f64),
                            ),
                        }
                    } else {
                        (
                            members
                                .iter()
                                .map(|member| member.rect.size.width - 35.0)
                                .sum::<f64>()
                                + 35.0,
                            members
                                .iter()
                                .map(|member| member.rect.size.height)
                                .fold(0.0, f64::max),
                        )
                    };
                    // A cluster vessel's box encloses all arranged members.
                    // The first member is not its top-left after a mirror
                    // (column arrangements reverse member order), so recover
                    // the vessel origin from the member bounding box.
                    let position = members.iter().filter_map(|member| member.position).reduce(
                        |left, right| Point {
                            x: left.x.min(right.x),
                            y: left.y.min(right.y),
                        },
                    );
                    // The root callback recursively visits the shared
                    // Containers map. Aggregate members in the Rust
                    // transaction projection retain child-local positions,
                    // while Go's vessel pointer is already positioned in
                    // its owner's coordinate frame. Lift the reconstructed
                    // vessel by that immediate owner's position and inset.
                    let position =
                        if use_synced || !trace_container_is_nested(graph, parent_tala_id) {
                            position
                        } else {
                            position.map(|point| {
                                let parent_position = TRACE_SYNCED_NODES
                                    .get()
                                    .and_then(|synced| synced.lock().ok())
                                    .and_then(|synced| synced.get(&parent_tala_id).cloned())
                                    .and_then(|parent| parent.position)
                                    .or_else(|| {
                                        graph
                                            .nodes
                                            .iter()
                                            .find(|parent| parent.tala_id == parent_tala_id)
                                            .and_then(|parent| parent.position)
                                    })
                                    .or_else(|| {
                                        graph
                                            .transaction_external_containers
                                            .iter()
                                            .find(|parent| parent.tala_id == parent_tala_id)
                                            .and_then(|parent| parent.position)
                                    })
                                    .unwrap_or_default();
                                let padding = graph.trace_source_container_padding(parent_tala_id);
                                Point {
                                    x: point.x + parent_position.x + padding.left,
                                    y: point.y + parent_position.y + padding.top,
                                }
                            })
                        };
                    deferred_vessels.push(PlacementTraceNode {
                        id,
                        stable_id: vessel_id.to_string(),
                        w_bits: trace_float_bits(width),
                        h_bits: trace_float_bits(height),
                        x_bits: position.map(|point| trace_float_bits(point.x)),
                        y_bits: position.map(|point| trace_float_bits(point.y)),
                        fixed_x_bits: None,
                        fixed_y_bits: None,
                    });
                    continue;
                }
                let child =
                    trace_node_snapshot(graph, child_tala_id, child_node, false, false, use_synced);
                if !seen.insert(child.external_id.clone()) {
                    continue;
                }
                let is_aggregate_vessel = child.external_id.ends_with("#cluster-vessel")
                    || child.external_id.ends_with("#sequence-vessel")
                    || child.external_id.starts_with("Cluster vessel of:")
                    || child.external_id.starts_with("Sequence vessel of:");
                let position = child.position.map(|point| {
                    if use_synced
                        || !is_aggregate_vessel
                        || !trace_container_is_nested(graph, parent_tala_id)
                    {
                        point
                    } else {
                        let parent_position = TRACE_SYNCED_NODES
                            .get()
                            .and_then(|synced| synced.lock().ok())
                            .and_then(|synced| synced.get(&parent_tala_id).cloned())
                            .and_then(|parent| parent.position)
                            .or_else(|| {
                                graph
                                    .nodes
                                    .iter()
                                    .find(|parent| parent.tala_id == parent_tala_id)
                                    .and_then(|parent| parent.position)
                            })
                            .or_else(|| {
                                graph
                                    .transaction_external_containers
                                    .iter()
                                    .find(|parent| parent.tala_id == parent_tala_id)
                                    .and_then(|parent| parent.position)
                            })
                            .unwrap_or_default();
                        let padding = graph.trace_source_container_padding(parent_tala_id);
                        Point {
                            x: point.x + parent_position.x + padding.left,
                            y: point.y + parent_position.y + padding.top,
                        }
                    }
                });
                let child_record = PlacementTraceNode {
                    id: child.external_id.clone(),
                    stable_id: child.tala_id.to_string(),
                    w_bits: trace_float_bits(child.rect.size.width),
                    h_bits: trace_float_bits(child.rect.size.height),
                    x_bits: position.map(|point| trace_float_bits(point.x)),
                    y_bits: position.map(|point| trace_float_bits(point.y)),
                    fixed_x_bits: child.fixed_top_left.map(|point| trace_float_bits(point.x)),
                    fixed_y_bits: child.fixed_top_left.map(|point| trace_float_bits(point.y)),
                };
                if child.external_id.ends_with("#cluster-vessel")
                    || child.external_id.ends_with("#sequence-vessel")
                    || child.external_id.starts_with("Cluster vessel of:")
                    || child.external_id.starts_with("Sequence vessel of:")
                {
                    deferred_vessels.push(child_record);
                } else {
                    nodes.push(child_record);
                }
                append_external_trace_nodes(
                    child_tala_id,
                    graph,
                    seen,
                    nodes,
                    use_synced,
                    use_live_container_order,
                );
            }
            // CombineSubgraphs appends grouping vessels after the ordinary
            // children in the owning container. Keep that diagnostic order
            // while leaving the arena and runtime traversal untouched.
            nodes.extend(deferred_vessels);
        }
        for node_id in &trace_nodes {
            let parent = &graph.nodes[node_id.0 as usize];
            append_external_trace_nodes(
                parent.tala_id,
                graph,
                &mut seen_external,
                &mut nodes,
                phase == "after-fit" || phase == "initialized",
                phase == "after-fit" || phase == "initialized",
            );
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    let has_duplicate_trace_ids = {
        let mut ids = BTreeSet::new();
        nodes.iter().any(|node| !ids.insert(node.id.clone()))
    };
    #[cfg(not(feature = "diagnostic-traces"))]
    let has_duplicate_trace_ids = true;
    if root.is_empty() && !owner_container_walk_applied && !has_duplicate_trace_ids {
        // The owning Go graph walks its shared Containers map while visiting
        // Graph.Nodes, so children of a visible container appear immediately
        // after that container.  Rust's transaction projection stores those
        // aliases separately; reorder the already materialized diagnostic
        // records to the same depth-first order without touching runtime
        // placement state.
        let by_id = nodes
            .drain(..)
            .map(|node| (node.id.clone(), node))
            .collect::<BTreeMap<_, _>>();
        let mut ordered = Vec::with_capacity(by_id.len());
        let mut emitted = BTreeSet::new();
        fn append_trace_node(
            id: &str,
            by_id: &BTreeMap<String, PlacementTraceNode>,
            ordered: &mut Vec<PlacementTraceNode>,
            emitted: &mut BTreeSet<String>,
        ) {
            if !emitted.insert(id.to_owned()) {
                return;
            }
            if let Some(node) = by_id.get(id) {
                ordered.push(node.clone());
            }
        }
        fn projected_child_trace_id(graph: &ArenaGraph, child: &ArenaNode) -> Option<String> {
            if let Some(cluster) = graph.clusters.iter().find(|cluster| {
                graph.cluster_is_active(cluster)
                    && cluster
                        .members
                        .iter()
                        .any(|member| graph.nodes[member.0 as usize].tala_id == child.tala_id)
            }) {
                if cluster
                    .members
                    .first()
                    .is_some_and(|member| graph.nodes[member.0 as usize].tala_id != child.tala_id)
                {
                    return None;
                }
                let arrangement = match cluster.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                };
                let members = cluster
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some(format!(
                    "Cluster vessel of: [{members}]; Arrangement: {arrangement}"
                ));
            }
            if let Some((vessel_id, members)) = graph
                .transaction_external_aggregate_children
                .iter()
                .find(|(_, members)| members.iter().any(|member| member.tala_id == child.tala_id))
            {
                let carrier = members.first()?;
                if carrier.tala_id != child.tala_id {
                    return None;
                }
                let members_text = members
                    .iter()
                    .map(|member| member.external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Some(layout) = graph.transaction_external_cluster_layouts.get(vessel_id) {
                    let arrangement = match layout.arrangement {
                        ClusterArrangement::Row => "Row",
                        ClusterArrangement::Column => "Column",
                    };
                    return Some(format!(
                        "Cluster vessel of: [{members_text}]; Arrangement: {arrangement}"
                    ));
                }
                return Some(format!("Sequence vessel of: [{members_text}]"));
            }
            Some(child.external_id.clone())
        }
        fn append_trace_subtree(
            node: &ArenaNode,
            graph: &ArenaGraph,
            by_id: &BTreeMap<String, PlacementTraceNode>,
            ordered: &mut Vec<PlacementTraceNode>,
            emitted: &mut BTreeSet<String>,
            deferred_vessels: &mut Vec<String>,
            defer_vessels: bool,
            use_live_container_order: bool,
        ) {
            let Some(id) = projected_child_trace_id(graph, node) else {
                return;
            };
            if defer_vessels && id.starts_with("Cluster vessel of:") {
                // In the Go graph a grouping vessel is appended to its
                // current container slice after the ordinary siblings. The
                // stable Rust arena retains the member's original slot, so
                // defer this diagnostic-only record until the slice walk is
                // complete. Runtime node order is left untouched.
                if !emitted.contains(&id) && !deferred_vessels.contains(&id) {
                    deferred_vessels.push(id);
                }
                return;
            }
            append_trace_node(&id, by_id, ordered, emitted);
            if id.starts_with("Cluster vessel of:") || id.starts_with("Sequence vessel of:") {
                return;
            }
            for child_tala_id in
                trace_snapshot_child_ids(graph, node.tala_id, use_live_container_order)
            {
                let Some(child) = graph
                    .nodes
                    .iter()
                    .find(|child| child.tala_id == child_tala_id)
                    .or_else(|| {
                        graph
                            .transaction_external_container_children
                            .values()
                            .flatten()
                            .find(|child| child.tala_id == child_tala_id)
                    })
                    .or_else(|| {
                        graph
                            .trace_container_children
                            .values()
                            .flatten()
                            .find(|child| child.tala_id == child_tala_id)
                    })
                    .or_else(|| {
                        graph
                            .transaction_external_aggregate_children
                            .values()
                            .flatten()
                            .find(|child| child.tala_id == child_tala_id)
                    })
                else {
                    continue;
                };
                append_trace_subtree(
                    child,
                    graph,
                    by_id,
                    ordered,
                    emitted,
                    deferred_vessels,
                    defer_vessels,
                    use_live_container_order,
                );
            }
        }
        fn projected_root_trace_id(graph: &ArenaGraph, node_id: NodeId) -> Option<String> {
            // Synthetic aggregate entries already occupy the position of
            // Go's shared vessel in the owning Graph.Nodes slice. Resolve
            // their descriptive ID directly so the vessel remains
            // interleaved with ordinary siblings during initialized traces.
            let external_id = &graph.nodes[node_id.0 as usize].external_id;
            if external_id.ends_with("#cluster-vessel") || external_id.ends_with("#sequence-vessel")
            {
                return Some(trace_node_external_id(graph, node_id));
            }
            if let Some(cluster) = graph.clusters.iter().find(|cluster| {
                graph.cluster_is_active(cluster) && cluster.members.contains(&node_id)
            }) {
                if cluster.members.first() != Some(&node_id) {
                    return None;
                }
                let arrangement = match cluster.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                };
                let members = cluster
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some(format!(
                    "Cluster vessel of: [{members}]; Arrangement: {arrangement}"
                ));
            }
            if let Some(sequence) = graph
                .sequences
                .iter()
                .find(|sequence| sequence.members.contains(&node_id))
            {
                if sequence.members.first() != Some(&node_id) {
                    return None;
                }
                let members = sequence
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some(format!("Sequence vessel of: [{members}]"));
            }
            if let Some((vessel_id, children)) = graph
                .transaction_external_aggregate_children
                .iter()
                .find(|(_, children)| {
                    children
                        .iter()
                        .any(|child| graph.nodes[node_id.0 as usize].tala_id == child.tala_id)
                })
            {
                let carrier = children.first()?;
                if carrier.tala_id != graph.nodes[node_id.0 as usize].tala_id {
                    return None;
                }
                let arrangement = graph
                    .transaction_external_cluster_layouts
                    .get(vessel_id)
                    .map(|layout| match layout.arrangement {
                        ClusterArrangement::Row => "Row",
                        ClusterArrangement::Column => "Column",
                    })
                    .unwrap_or("Column");
                let members = children
                    .iter()
                    .map(|child| child.external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some(format!(
                    "Cluster vessel of: [{members}]; Arrangement: {arrangement}"
                ));
            }
            Some(graph.nodes[node_id.0 as usize].external_id.clone())
        }
        for node_id in &trace_nodes {
            // Go appends grouping vessels to each owning container's child
            // slice. Keep a per-container queue so a vessel is emitted after
            // that container's ordinary children, before the next root
            // sibling, rather than after the entire root walk.
            let mut deferred_vessels = Vec::new();
            let Some(id) = projected_root_trace_id(graph, *node_id) else {
                continue;
            };
            if id.starts_with("Cluster vessel of:") || id.starts_with("Sequence vessel of:") {
                if !emitted.contains(&id) && !deferred_vessels.contains(&id) {
                    deferred_vessels.push(id);
                }
            } else {
                append_trace_node(&id, &by_id, &mut ordered, &mut emitted);
            }
            let parent_tala_id = graph.nodes[node_id.0 as usize].tala_id;
            for child_tala_id in trace_snapshot_child_ids(
                graph,
                parent_tala_id,
                phase == "after-fit" || phase == "initialized",
            ) {
                let Some(child) = graph
                    .nodes
                    .iter()
                    .find(|child| child.tala_id == child_tala_id)
                    .or_else(|| {
                        graph
                            .transaction_external_container_children
                            .values()
                            .flatten()
                            .find(|child| child.tala_id == child_tala_id)
                    })
                    .or_else(|| {
                        graph
                            .trace_container_children
                            .values()
                            .flatten()
                            .find(|child| child.tala_id == child_tala_id)
                    })
                    .or_else(|| {
                        graph
                            .transaction_external_aggregate_children
                            .values()
                            .flatten()
                            .find(|child| child.tala_id == child_tala_id)
                    })
                else {
                    continue;
                };
                let Some(child_id) = projected_child_trace_id(graph, child) else {
                    continue;
                };
                if child_id.starts_with("Cluster vessel of:")
                    || child_id.starts_with("Sequence vessel of:")
                {
                    if !emitted.contains(&child_id) && !deferred_vessels.contains(&child_id) {
                        deferred_vessels.push(child_id);
                    }
                } else {
                    append_trace_subtree(
                        child,
                        graph,
                        &by_id,
                        &mut ordered,
                        &mut emitted,
                        &mut deferred_vessels,
                        false,
                        phase == "after-fit" || phase == "initialized",
                    );
                }
            }
            for id in deferred_vessels {
                append_trace_node(&id, &by_id, &mut ordered, &mut emitted);
            }
        }
        // Keep diagnostic-only records not reachable through the visible
        // traversal, preserving their prior order.
        for (id, node) in by_id {
            if emitted.insert(id) {
                ordered.push(node);
            }
        }
        nodes = ordered;
        #[cfg(feature = "diagnostic-traces")]
        trace_placement_target_records(phase, root, "root-child-order", &nodes);
        #[cfg(feature = "diagnostic-traces")]
        trace_placement_order_records(phase, root, "root-child-order", &nodes);
        #[cfg(feature = "diagnostic-traces")]
        if phase == "after-fit"
            && trace_child_order_is_node_ordered(
                graph,
                graph
                    .nodes
                    .iter()
                    .find(|node| node.external_id == root)
                    .map(|node| node.tala_id)
                    .unwrap_or_default(),
            )
        {
            // The final after-fit callback walks the owning root's source
            // Containers slices.  Reconstruct that depth-first order from
            // the transaction child projection as a last normalization pass;
            // this catches tree-restored direct children that were absent
            // from the earlier stable trace map (resolver.gen/adapter).
            let records = nodes
                .drain(..)
                .map(|node| (node.id.clone(), node))
                .collect::<BTreeMap<_, _>>();
            let mut final_nodes = Vec::with_capacity(records.len());
            let mut final_seen = BTreeSet::new();
            fn append_source_tree(
                parent_tala_id: i64,
                graph: &ArenaGraph,
                records: &BTreeMap<String, PlacementTraceNode>,
                seen: &mut BTreeSet<String>,
                output: &mut Vec<PlacementTraceNode>,
            ) {
                let child_ids = trace_snapshot_child_ids(graph, parent_tala_id, true);
                if child_ids.is_empty() {
                    return;
                }
                for child_tala_id in child_ids {
                    let Some(child) = graph
                        .nodes
                        .iter()
                        .find(|child| child.tala_id == child_tala_id)
                        .or_else(|| {
                            graph
                                .transaction_external_container_children
                                .values()
                                .flatten()
                                .find(|child| child.tala_id == child_tala_id)
                        })
                        .or_else(|| {
                            graph
                                .trace_container_children
                                .values()
                                .flatten()
                                .find(|child| child.tala_id == child_tala_id)
                        })
                        .or_else(|| {
                            graph
                                .transaction_external_aggregate_children
                                .values()
                                .flatten()
                                .find(|child| child.tala_id == child_tala_id)
                        })
                    else {
                        continue;
                    };
                    let id = child.external_id.clone();
                    if seen.insert(id.clone())
                        && let Some(record) = records.get(&id)
                    {
                        output.push(record.clone());
                    }
                    append_source_tree(child_tala_id, graph, records, seen, output);
                }
            }
            let root_children = graph
                .trace_container_children
                .get(&0)
                .cloned()
                .unwrap_or_else(|| {
                    graph
                        .node_order
                        .iter()
                        .filter_map(|node_id| {
                            let node = graph.nodes.get(node_id.0 as usize)?;
                            let is_source_root = graph
                                .source_parent_tala_by_tala
                                .get(&node.tala_id)
                                .copied()
                                .flatten()
                                .is_none();
                            is_source_root.then_some(node.clone())
                        })
                        .collect::<Vec<_>>()
                });
            for child in &root_children {
                let id = child.external_id.clone();
                if final_seen.insert(id.clone())
                    && let Some(record) = records.get(&id)
                {
                    final_nodes.push(record.clone());
                }
                append_source_tree(
                    child.tala_id,
                    graph,
                    &records,
                    &mut final_seen,
                    &mut final_nodes,
                );
            }
            for (id, record) in records {
                if final_seen.insert(id) {
                    final_nodes.push(record);
                }
            }
            // Reinsert restored aliases immediately after their owning
            // source node. A temporary scope can expose only one direct
            // child in `trace_container_children` while the source order
            // still names the peeled siblings (resolver.gen/adapter). Keep
            // the visible trace slice first, then append those missing
            // source children in their durable order.
            let external_id_for_tala = |tala_id: i64| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == tala_id)
                    .map(|node| node.external_id.clone())
                    .or_else(|| {
                        graph
                            .trace_container_children
                            .values()
                            .flatten()
                            .find(|node| node.tala_id == tala_id)
                            .map(|node| node.external_id.clone())
                    })
            };
            // Go's after-fit callback still walks the flat owner Graph.Nodes
            // slice after AddCluster has replaced a member with its vessel.
            // Descendants retained under that member remain independent
            // owner entries; moving them beneath the synthetic vessel puts
            // them at the end because the member ID is no longer serialized.
            let replaced_aggregate_member_tala_ids = graph
                .clusters
                .iter()
                .filter_map(|cluster| {
                    let members = cluster
                        .members
                        .iter()
                        .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let arrangement = match cluster.arrangement {
                        ClusterArrangement::Row => "Row",
                        ClusterArrangement::Column => "Column",
                    };
                    let vessel_id =
                        format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}");
                    final_nodes
                        .iter()
                        .any(|record| record.id == vessel_id)
                        .then_some(cluster)
                })
                .flat_map(|cluster| {
                    cluster
                        .members
                        .iter()
                        .map(|member| graph.nodes[member.0 as usize].tala_id)
                })
                .collect::<BTreeSet<_>>();
            for parent_tala_id in graph.source_container_child_order.keys().copied() {
                if replaced_aggregate_member_tala_ids.contains(&parent_tala_id) {
                    continue;
                }
                let Some(parent_id) = external_id_for_tala(parent_tala_id) else {
                    continue;
                };
                let child_ids =
                    trace_snapshot_child_ids(graph, parent_tala_id, phase == "after-fit");
                if child_ids.is_empty() {
                    continue;
                }
                let child_external_ids = child_ids
                    .iter()
                    .filter_map(|tala_id| external_id_for_tala(*tala_id))
                    .collect::<Vec<_>>();
                if child_external_ids.is_empty() {
                    continue;
                }
                let mut extracted = Vec::new();
                final_nodes.retain(|record| {
                    if child_external_ids.iter().any(|id| id == &record.id) {
                        extracted.push(record.clone());
                        false
                    } else {
                        true
                    }
                });
                if extracted.is_empty() {
                    continue;
                }
                let mut by_id = extracted
                    .into_iter()
                    .map(|record| (record.id.clone(), record))
                    .collect::<BTreeMap<_, _>>();
                let mut ordered_children = Vec::new();
                for child_id in child_external_ids {
                    if let Some(record) = by_id.remove(&child_id) {
                        ordered_children.push(record);
                    }
                }
                ordered_children.extend(by_id.into_values());
                let insert_at = final_nodes
                    .iter()
                    .position(|record| record.id == parent_id)
                    .map(|index| index + 1)
                    .unwrap_or(final_nodes.len());
                final_nodes.splice(insert_at..insert_at, ordered_children);
            }
            nodes = final_nodes;
        }
    }
    // Preserve the source Graph.Nodes/Containers traversal reconstructed
    // above. Display names are not ownership keys: sorting dotted labels
    // here moves synthetic aggregate vessels outside their container.
    // Go temporarily reconnects descendant edges while a placement graph is
    // being fitted, then restores the original endpoints around the
    // before/after-direct hooks. Rust keeps those endpoint rewrites in its
    // stable abduction table rather than mutating ArenaEdge. Project that
    // table's pointer-visible view here so this diagnostic records the same
    // graph lifecycle without changing release storage.
    let trace_parent_scope = if root.is_empty() {
        None
    } else {
        graph
            .nodes
            .iter()
            .find(|node| node.external_id == root)
            .and_then(|node| node.container)
    };
    let trace_scope_ancestor = |mut node_id: NodeId| {
        while graph.nodes[node_id.0 as usize].container != trace_parent_scope {
            let Some(parent) = graph.nodes[node_id.0 as usize].container else {
                break;
            };
            node_id = parent;
        }
        node_id
    };
    let trace_external_name = |tala_id: i64| {
        graph
            .nodes
            .iter()
            .find(|node| node.tala_id == tala_id)
            .map(|node| {
                if matches!(phase, "before-direct" | "after-direct") {
                    if let Some((vessel_tala_id, members)) = graph
                        .transaction_external_aggregate_children
                        .iter()
                        .find(|(_, children)| children.iter().any(|child| child.tala_id == tala_id))
                    {
                        let member_names = members
                            .iter()
                            .map(|member| member.external_id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        if let Some(layout) = graph
                            .transaction_external_cluster_layouts
                            .get(vessel_tala_id)
                        {
                            let arrangement = match layout.arrangement {
                                ClusterArrangement::Row => "Row",
                                ClusterArrangement::Column => "Column",
                            };
                            return format!(
                                "Cluster vessel of: [{member_names}]; Arrangement: {arrangement}"
                            );
                        }
                        return format!("Sequence vessel of: [{member_names}]");
                    }
                    node.external_id.clone()
                } else {
                    trace_node_external_id(graph, node.input_id)
                }
            })
            .or_else(|| {
                graph
                    .clusters
                    .iter()
                    .find(|cluster| cluster.vessel_tala_id == tala_id)
                    .and_then(|cluster| cluster.members.first().copied())
                    .map(|member| trace_node_external_id(graph, member))
            })
            .or_else(|| {
                graph
                    .sequences
                    .iter()
                    .find(|sequence| sequence.vessel_tala_id == tala_id)
                    .and_then(|sequence| sequence.members.first().copied())
                    .map(|member| trace_node_external_id(graph, member))
            })
            .or_else(|| {
                graph
                    .transaction_external_containers
                    .iter()
                    .chain(
                        graph
                            .trace_container_children
                            .values()
                            .flatten(),
                    )
                    .chain(
                        graph
                            .transaction_external_aggregate_children
                            .values()
                            .flatten(),
                    )
                    .find(|node| node.tala_id == tala_id)
                    .map(|node| {
                        // Transaction projections may carry the synthetic
                        // `member#cluster-vessel` name.  Resolve it through
                        // the same aggregate DebugID used for live nodes so
                        // pointer-visible trace identities stay aligned with
                        // Go's shared vessel object.
                        if matches!(phase, "before-direct" | "after-direct")
                            && let Some((vessel_tala_id, members)) = graph
                                .transaction_external_aggregate_children
                                .iter()
                                .find(|(_, children)| {
                                    children.iter().any(|child| child.tala_id == tala_id)
                                })
                        {
                            let member_names = members
                                .iter()
                                .map(|member| member.external_id.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            if let Some(layout) = graph
                                .transaction_external_cluster_layouts
                                .get(vessel_tala_id)
                            {
                                let arrangement = match layout.arrangement {
                                    ClusterArrangement::Row => "Row",
                                    ClusterArrangement::Column => "Column",
                                };
                                return format!(
                                    "Cluster vessel of: [{member_names}]; Arrangement: {arrangement}"
                                );
                            }
                            return format!("Sequence vessel of: [{member_names}]");
                        }
                        if let Some((base, suffix)) = node.external_id.rsplit_once('#') {
                            if suffix == "cluster-vessel"
                                && let Some(cluster) = graph.clusters.iter().find(|cluster| {
                                    cluster.members.iter().any(|member| {
                                        graph.nodes[member.0 as usize].external_id == base
                                    })
                                })
                            {
                                let members = cluster
                                    .members
                                    .iter()
                                    .map(|member| {
                                        graph.nodes[member.0 as usize].external_id.as_str()
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                let arrangement = match cluster.arrangement {
                                    ClusterArrangement::Row => "Row",
                                    ClusterArrangement::Column => "Column",
                                };
                                return format!(
                                    "Cluster vessel of: [{members}]; Arrangement: {arrangement}"
                                );
                            }
                            if suffix == "sequence-vessel"
                                && let Some(sequence) = graph.sequences.iter().find(|sequence| {
                                    sequence.members.iter().any(|member| {
                                        graph.nodes[member.0 as usize].external_id == base
                                    })
                                })
                            {
                                let members = sequence
                                    .members
                                    .iter()
                                    .map(|member| {
                                        graph.nodes[member.0 as usize].external_id.as_str()
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                return format!("Sequence vessel of: [{members}]");
                            }
                        }
                        node.external_id.clone()
                    })
            })
    };
    let trace_source_endpoint_name = |edge_id: EdgeId, from: bool| {
        let edge = graph.edge_records.get(edge_id.0 as usize)?;
        let endpoints = (edge.tala_id != 0)
            .then(|| {
                graph
                    .source_edge_endpoints_by_id
                    .get(&edge.tala_id)
                    .copied()
            })
            .flatten()
            .or_else(|| {
                graph
                    .source_edge_endpoints
                    .get(edge.input_id.0 as usize)
                    .copied()
            })?;
        let (source, target) = endpoints;
        trace_external_name(if from { source } else { target })
    };
    let trace_endpoint_name = |edge_id: EdgeId, node_id: NodeId| {
        if matches!(phase, "before-direct" | "after-direct") {
            // Those hooks run after Graph.RestoreEdgeAbductions. Resolve the
            // original serialized endpoint where it is available.
            let current = &graph.edge_records[edge_id.0 as usize];
            let tala_id = graph
                .source_edge_endpoints_by_id
                .get(&current.tala_id)
                .copied()
                .or_else(|| {
                    graph
                        .source_edge_endpoints
                        .get(current.input_id.0 as usize)
                        .copied()
                })
                .and_then(|(from, to)| {
                    if node_id == current.from {
                        Some(from)
                    } else if node_id == current.to {
                        Some(to)
                    } else {
                        None
                    }
                });
            if let Some(tala_id) = tala_id
                && let Some(name) = trace_external_name(tala_id)
            {
                return name;
            }
        }
        trace_node_external_id(graph, trace_scope_ancestor(node_id))
    };
    // `tracePlacement("after-fit", g, root)` observes the owning Go graph
    // while descendant edge abductions are still applied. An edge crossing
    // the fitted root therefore has the root as its live endpoint; an edge
    // with both endpoints inside the root keeps its concrete endpoints. The
    // arena stores original endpoints and keeps abductions in side tables, so
    // project this one diagnostic boundary explicitly.
    let after_fit_root_id = (phase == "after-fit")
        .then(|| {
            graph
                .nodes
                .iter()
                .find(|node| node.external_id == root)
                .map(|node| node.input_id)
        })
        .flatten();
    let after_fit_crossing_endpoint = |node: NodeId, other: NodeId| {
        after_fit_root_id.filter(|root_id| {
            let root_tala_id = graph.nodes[root_id.0 as usize].tala_id;
            // Temporary Rust placement scopes clear `ArenaNode.container`
            // when they flatten a child graph. Go's shared Node pointers keep
            // the original Container chain, so retain the serialized
            // ancestry as the fallback for this diagnostic boundary.
            let node_is_descendant = graph.is_descendant_of(node, *root_id)
                || graph.source_is_descendant_of_tala(
                    graph.nodes[node.0 as usize].tala_id,
                    root_tala_id,
                );
            let other_is_descendant = graph.is_descendant_of(other, *root_id)
                || graph.source_is_descendant_of_tala(
                    graph.nodes[other.0 as usize].tala_id,
                    root_tala_id,
                );
            node != *root_id && node_is_descendant && !other_is_descendant
        })
    };
    let trace_after_fit_endpoint_name = |edge_id: EdgeId, from: bool| {
        let root_id = after_fit_root_id?;
        let root_tala_id = graph.nodes[root_id.0 as usize].tala_id;
        let edge = graph.edge_records.get(edge_id.0 as usize)?;
        let endpoints = graph
            .source_edge_endpoints_by_id
            .get(&edge.tala_id)
            .copied()
            .or_else(|| {
                graph
                    .source_edge_endpoints
                    .get(edge.input_id.0 as usize)
                    .copied()
            })
            .unwrap_or_else(|| {
                (
                    graph.nodes[edge.from.0 as usize].tala_id,
                    graph.nodes[edge.to.0 as usize].tala_id,
                )
            });
        let (source, target) = endpoints;
        let source_inside =
            source == root_tala_id || graph.source_is_descendant_of_tala(source, root_tala_id);
        let target_inside =
            target == root_tala_id || graph.source_is_descendant_of_tala(target, root_tala_id);
        let tala_id = if from {
            if source_inside && !target_inside {
                root_tala_id
            } else {
                source
            }
        } else if target_inside && !source_inside {
            root_tala_id
        } else {
            target
        };
        trace_external_name(tala_id)
    };
    // A nested after-fit scope can lose peeled siblings from its live trace
    // map before the owning root callback runs. When serialized source
    // metadata names that scope, restore the visible children followed by
    // any missing source aliases in the final diagnostic list.
    if phase == "after-fit"
        && let Some(root_tala_id) = graph
            .nodes
            .iter()
            .find(|node| node.external_id == root)
            .map(|node| node.tala_id)
        && graph
            .source_container_child_order
            .contains_key(&root_tala_id)
    {
        let child_tala_ids = trace_snapshot_child_ids(graph, root_tala_id, phase == "after-fit");
        let source_child_count = trace_source_child_ids(graph, root_tala_id)
            .map(|children| children.len())
            .unwrap_or_default();
        let live_child_count = graph
            .transaction_external_container_children
            .get(&root_tala_id)
            .or_else(|| graph.trace_container_children.get(&root_tala_id))
            .map(|children| children.len())
            .unwrap_or_default();
        let missing_source_siblings = live_child_count < source_child_count;
        let child_external_ids = child_tala_ids
            .iter()
            .filter_map(|tala_id| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == *tala_id)
                    .map(|node| node.external_id.clone())
                    .or_else(|| {
                        graph
                            .trace_container_children
                            .values()
                            .flatten()
                            .find(|node| node.tala_id == *tala_id)
                            .map(|node| node.external_id.clone())
                    })
            })
            .collect::<Vec<_>>();
        // Reorder only when the current projection disagrees with the
        // recovered child sequence. Complete scopes whose order already
        // matches are left untouched, preserving any legitimate upstream
        // traversal order.
        let child_id_set = child_external_ids.iter().collect::<BTreeSet<_>>();
        let current_child_external_ids = nodes
            .iter()
            .filter_map(|record| {
                child_id_set
                    .contains(&record.id)
                    .then_some(record.id.clone())
            })
            .collect::<Vec<_>>();
        let needs_child_reorder =
            missing_source_siblings && current_child_external_ids != child_external_ids;
        if needs_child_reorder && !child_external_ids.is_empty() {
            let mut extracted = Vec::new();
            nodes.retain(|record| {
                if child_external_ids.iter().any(|id| id == &record.id) {
                    extracted.push(record.clone());
                    false
                } else {
                    true
                }
            });
            let mut by_id = extracted
                .into_iter()
                .map(|record| (record.id.clone(), record))
                .collect::<BTreeMap<_, _>>();
            let mut ordered_children = Vec::new();
            for child_id in child_external_ids {
                if let Some(record) = by_id.remove(&child_id) {
                    ordered_children.push(record);
                }
            }
            ordered_children.extend(by_id.into_values());
            let insert_at = nodes
                .iter()
                .position(|record| record.id == root)
                .map(|index| index + 1)
                .unwrap_or(nodes.len());
            nodes.splice(insert_at..insert_at, ordered_children);
        }
    }
    // Later source-order fallbacks can append retained arena members after
    // the vessel traversal. Go exposes the vessel pointer in each owning
    // graph and has removed those members from its Containers slice, so
    // suppress aliases at both root and nested owner boundaries whenever
    // their vessel is present in the final diagnostic snapshot.
    let record_ids = nodes
        .iter()
        .map(|record| record.id.clone())
        .collect::<BTreeSet<_>>();
    let mut absorbed_member_ids = BTreeSet::new();
    for cluster in &graph.clusters {
        let arrangement = match cluster.arrangement {
            ClusterArrangement::Row => "Row",
            ClusterArrangement::Column => "Column",
        };
        let members = cluster
            .members
            .iter()
            .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let vessel_id = format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}");
        if record_ids.contains(&vessel_id) {
            absorbed_member_ids.extend(
                cluster
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.clone()),
            );
        }
    }
    for sequence in &graph.sequences {
        let members = sequence
            .members
            .iter()
            .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let vessel_id = format!("Sequence vessel of: [{members}]");
        if record_ids.contains(&vessel_id) {
            absorbed_member_ids.extend(
                sequence
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.clone()),
            );
        }
    }
    for (vessel_tala_id, members) in &graph.transaction_external_aggregate_children {
        let Some(vessel_id) = projected_vessel_trace_id(graph, *vessel_tala_id) else {
            continue;
        };
        if record_ids.contains(&vessel_id) {
            absorbed_member_ids.extend(members.iter().map(|member| member.external_id.clone()));
        }
    }
    nodes.retain(|record| !absorbed_member_ids.contains(&record.id));
    #[cfg(feature = "diagnostic-traces")]
    let initialized_root_snapshot = phase == "initialized" && root.is_empty();
    #[cfg(feature = "diagnostic-traces")]
    let live_owner_snapshot = phase == "after-fit"
        || root_graph_snapshot
        || phase == "before-combine"
        || phase == "after-combine"
        || phase == "after-orient"
        || phase == "before-fit"
        || phase == "after-fit-size"
        || phase == "before-direct"
        || phase == "after-direct"
        || phase.starts_with("after-orthogonal-")
        || phase == "sized-transition"
        || phase.starts_with("sized-")
        || phase == "optimized"
        || phase.starts_with("after-trees-")
        || phase.starts_with("after-restore-edges-")
        || phase.starts_with("after-direct-owner-")
        || phase.starts_with("after-subgraph-")
        || phase.starts_with("before-subgraph-")
        || matches!(
            phase,
            "after-child-recursion" | "after-nears" | "after-herds" | "after-split"
        );
    #[cfg(feature = "diagnostic-traces")]
    if initialized_root_snapshot || live_owner_snapshot {
        // Some snapshots flatten stable arena nodes after the owner callback.
        // Rewalk live container children before serialization so nested nodes
        // and aggregate carriers remain beside their owning container.
        let output_ids = nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>();
        let aggregate_member_descendants_are_flat = live_owner_snapshot
            && !root.is_empty()
            && (graph.clusters.iter().any(|cluster| {
                if !graph.cluster_is_active(cluster) {
                    return false;
                }
                let member_names = cluster
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>();
                let arrangement = match cluster.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                };
                let vessel_record_id = format!(
                    "Cluster vessel of: [{}]; Arrangement: {arrangement}",
                    member_names.join(", ")
                );
                output_ids.contains(vessel_record_id.as_str())
                    && cluster.members.iter().any(|member| {
                        let member = &graph.nodes[member.0 as usize];
                        nodes.iter().any(|record| {
                            record
                                .id
                                .strip_prefix(&member.external_id)
                                .is_some_and(|suffix| suffix.starts_with('.'))
                        })
                    })
            }) || graph.sequences.iter().any(|sequence| {
                if !graph.sequence_is_active(sequence) {
                    return false;
                }
                let member_names = sequence
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>();
                let vessel_record_id = format!("Sequence vessel of: [{}]", member_names.join(", "));
                output_ids.contains(vessel_record_id.as_str())
                    && sequence.members.iter().any(|member| {
                        let member = &graph.nodes[member.0 as usize];
                        nodes.iter().any(|record| {
                            record
                                .id
                                .strip_prefix(&member.external_id)
                                .is_some_and(|suffix| suffix.starts_with('.'))
                        })
                    })
            }));
        let mut ranks = BTreeMap::new();
        if initialized_root_snapshot || live_owner_snapshot {
            // Go recursively visits each graph's Containers after its Nodes
            // slice. Stable arena snapshots can flatten nested children after
            // later siblings, so rebuild parent groups while preserving the
            // current order within each owner.
            let mut external_tala_by_id = graph
                .nodes
                .iter()
                .map(|node| (node.external_id.clone(), node.tala_id))
                .collect::<BTreeMap<_, _>>();
            for child in graph.trace_container_children.values().flatten() {
                external_tala_by_id
                    .entry(child.external_id.clone())
                    .or_insert(child.tala_id);
            }
            let mut tala_by_record = BTreeMap::new();
            let mut record_by_tala = BTreeMap::new();
            for record in &nodes {
                let source_tala_id = external_tala_by_id
                    .get(&record.id)
                    .copied()
                    .or_else(|| {
                        graph.clusters.iter().find_map(|cluster| {
                            let first = *cluster.members.first()?;
                            let members = cluster
                                .members
                                .iter()
                                .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            let arrangement = match cluster.arrangement {
                                ClusterArrangement::Row => "Row",
                                ClusterArrangement::Column => "Column",
                            };
                            (record.id
                                == format!(
                                    "Cluster vessel of: [{members}]; Arrangement: {arrangement}"
                                ))
                            .then_some(graph.nodes[first.0 as usize].tala_id)
                        })
                    })
                    .or_else(|| {
                        graph.sequences.iter().find_map(|sequence| {
                            let first = *sequence.members.first()?;
                            let members = sequence
                                .members
                                .iter()
                                .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            (record.id == format!("Sequence vessel of: [{members}]"))
                                .then_some(graph.nodes[first.0 as usize].tala_id)
                        })
                    })
                    .or_else(|| {
                        graph
                            .transaction_external_aggregate_children
                            .keys()
                            .find_map(|&tala_id| {
                                (projected_vessel_trace_id(graph, tala_id).as_deref()
                                    == Some(record.id.as_str()))
                                .then_some(tala_id)
                            })
                    });
                let Some(source_tala_id) = source_tala_id else {
                    continue;
                };
                tala_by_record.insert(record.id.clone(), source_tala_id);
                record_by_tala.insert(source_tala_id, record.id.clone());
            }
            // If a source child is now represented by an aggregate vessel,
            // resolve any nested parent through the vessel's emitted record.
            for cluster in &graph.clusters {
                let Some(&first) = cluster.members.first() else {
                    continue;
                };
                let members = cluster
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let arrangement = match cluster.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                };
                let vessel_id =
                    format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}");
                if output_ids.contains(vessel_id.as_str()) {
                    for member in &cluster.members {
                        record_by_tala
                            .insert(graph.nodes[member.0 as usize].tala_id, vessel_id.clone());
                    }
                    record_by_tala.insert(graph.nodes[first.0 as usize].tala_id, vessel_id);
                }
            }
            for sequence in &graph.sequences {
                let Some(&first) = sequence.members.first() else {
                    continue;
                };
                let members = sequence
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let vessel_id = format!("Sequence vessel of: [{members}]");
                if output_ids.contains(vessel_id.as_str()) {
                    for member in &sequence.members {
                        record_by_tala
                            .insert(graph.nodes[member.0 as usize].tala_id, vessel_id.clone());
                    }
                    record_by_tala.insert(graph.nodes[first.0 as usize].tala_id, vessel_id);
                }
            }
            let mut aggregate_owner_replacements =
                BTreeMap::<i64, Vec<(BTreeSet<i64>, i64)>>::new();
            for (&carrier_tala_id, members) in &graph.transaction_external_aggregate_children {
                let Some(vessel_id) = projected_vessel_trace_id(graph, carrier_tala_id) else {
                    continue;
                };
                if !output_ids.contains(vessel_id.as_str()) {
                    continue;
                }
                record_by_tala.insert(carrier_tala_id, vessel_id.clone());
                let member_tala_ids = members
                    .iter()
                    .map(|member| member.tala_id)
                    .collect::<BTreeSet<_>>();
                for member_tala_id in &member_tala_ids {
                    record_by_tala.insert(*member_tala_id, vessel_id.clone());
                }
                let parent_tala_id = graph
                    .transaction_external_container_children
                    .iter()
                    .find_map(|(&parent_tala_id, children)| {
                        member_tala_ids
                            .iter()
                            .all(|member_tala_id| {
                                children
                                    .iter()
                                    .any(|child| child.tala_id == *member_tala_id)
                            })
                            .then_some(parent_tala_id)
                    })
                    .or_else(|| {
                        graph.trace_container_children.iter().find_map(
                            |(&parent_tala_id, children)| {
                                member_tala_ids
                                    .iter()
                                    .all(|member_tala_id| {
                                        children
                                            .iter()
                                            .any(|child| child.tala_id == *member_tala_id)
                                    })
                                    .then_some(parent_tala_id)
                            },
                        )
                    })
                    .or_else(|| {
                        let parent_id = output_ids
                            .iter()
                            .copied()
                            .filter(|candidate| {
                                members.iter().all(|member| {
                                    member.external_id.starts_with(&format!("{candidate}."))
                                })
                            })
                            .max_by_key(|candidate| candidate.len())?;
                        external_tala_by_id.get(parent_id).copied()
                    })
                    .unwrap_or(0);
                aggregate_owner_replacements
                    .entry(parent_tala_id)
                    .or_default()
                    .push((member_tala_ids, carrier_tala_id));
            }
            for cluster in &graph.clusters {
                let Some(&carrier) = cluster.members.first() else {
                    continue;
                };
                let members = cluster
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>();
                let arrangement = match cluster.arrangement {
                    ClusterArrangement::Row => "Row",
                    ClusterArrangement::Column => "Column",
                };
                let vessel_id = format!(
                    "Cluster vessel of: [{}]; Arrangement: {arrangement}",
                    members.join(", ")
                );
                if !output_ids.contains(vessel_id.as_str()) {
                    continue;
                }
                let parent_id = output_ids
                    .iter()
                    .copied()
                    .filter(|candidate| {
                        members
                            .iter()
                            .all(|member| member.starts_with(&format!("{candidate}.")))
                    })
                    .max_by_key(|candidate| candidate.len());
                let parent_tala_id = parent_id
                    .and_then(|parent_id| external_tala_by_id.get(parent_id).copied())
                    .unwrap_or(0);
                aggregate_owner_replacements
                    .entry(parent_tala_id)
                    .or_default()
                    .push((
                        cluster
                            .members
                            .iter()
                            .map(|member| graph.nodes[member.0 as usize].tala_id)
                            .collect(),
                        graph.nodes[carrier.0 as usize].tala_id,
                    ));
            }
            for sequence in &graph.sequences {
                let Some(&carrier) = sequence.members.first() else {
                    continue;
                };
                let members = sequence
                    .members
                    .iter()
                    .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                    .collect::<Vec<_>>();
                let vessel_id = format!("Sequence vessel of: [{}]", members.join(", "));
                if !output_ids.contains(vessel_id.as_str()) {
                    continue;
                }
                let parent_id = output_ids
                    .iter()
                    .copied()
                    .filter(|candidate| {
                        members
                            .iter()
                            .all(|member| member.starts_with(&format!("{candidate}.")))
                    })
                    .max_by_key(|candidate| candidate.len());
                let parent_tala_id = parent_id
                    .and_then(|parent_id| external_tala_by_id.get(parent_id).copied())
                    .unwrap_or(0);
                aggregate_owner_replacements
                    .entry(parent_tala_id)
                    .or_default()
                    .push((
                        sequence
                            .members
                            .iter()
                            .map(|member| graph.nodes[member.0 as usize].tala_id)
                            .collect(),
                        graph.nodes[carrier.0 as usize].tala_id,
                    ));
            }
            let mut source_parent_by_tala = BTreeMap::new();
            for (&parent_tala_id, children) in &graph.trace_container_children {
                let parent_tala_id = (parent_tala_id != 0).then_some(parent_tala_id);
                for child in children {
                    source_parent_by_tala.insert(child.tala_id, parent_tala_id);
                }
            }
            for (&parent_tala_id, children) in &graph.transaction_external_container_children {
                let parent_tala_id = (parent_tala_id != 0).then_some(parent_tala_id);
                for child in children {
                    source_parent_by_tala
                        .entry(child.tala_id)
                        .or_insert(parent_tala_id);
                }
            }
            for (&parent_tala_id, children) in &graph.source_container_child_order {
                for child_tala_id in children {
                    source_parent_by_tala
                        .entry(*child_tala_id)
                        .or_insert(Some(parent_tala_id));
                }
            }
            for (&tala_id, parent_tala_id) in &graph.source_parent_tala_by_tala {
                source_parent_by_tala
                    .entry(tala_id)
                    .or_insert(*parent_tala_id);
            }
            for node in &graph.nodes {
                let parent_tala_id = node
                    .container
                    .map(|parent| graph.nodes[parent.0 as usize].tala_id);
                source_parent_by_tala
                    .entry(node.tala_id)
                    .or_insert(parent_tala_id);
            }
            for (&parent_tala_id, replacements) in &aggregate_owner_replacements {
                let parent_tala_id = (parent_tala_id != 0).then_some(parent_tala_id);
                for (member_tala_ids, carrier_tala_id) in replacements {
                    source_parent_by_tala.insert(*carrier_tala_id, parent_tala_id);
                    for member_tala_id in member_tala_ids {
                        source_parent_by_tala
                            .entry(*member_tala_id)
                            .or_insert(parent_tala_id);
                    }
                }
            }
            let mut children_by_parent = BTreeMap::<Option<String>, Vec<String>>::new();
            for record in &nodes {
                let Some(&source_tala_id) = tala_by_record.get(&record.id) else {
                    continue;
                };
                let mut parent_tala_id = source_parent_by_tala
                    .get(&source_tala_id)
                    .copied()
                    .flatten();
                let mut seen_ancestors = BTreeSet::new();
                let mut parent_record_id = None;
                while let Some(parent_id) = parent_tala_id {
                    if !seen_ancestors.insert(parent_id) {
                        break;
                    }
                    if let Some(candidate) = record_by_tala.get(&parent_id)
                        && candidate != &record.id
                        && output_ids.contains(candidate.as_str())
                    {
                        parent_record_id = Some(candidate.clone());
                        break;
                    }
                    parent_tala_id = source_parent_by_tala.get(&parent_id).copied().flatten();
                }
                if parent_record_id.is_none() {
                    parent_record_id = output_ids
                        .iter()
                        .copied()
                        .filter(|candidate| record.id.starts_with(&format!("{candidate}.")))
                        .max_by_key(|candidate| candidate.len())
                        .map(str::to_owned);
                }
                children_by_parent
                    .entry(parent_record_id)
                    .or_default()
                    .push(record.id.clone());
            }
            // Parent recovery can pass through shared Go pointers whose
            // recorded owner differs from the Rust arena parent. Preserve
            // the live sibling slice for each recovered container so the
            // final depth-first projection does not silently re-sort it.
            for (parent_record_id, child_records) in &mut children_by_parent {
                let Some(parent_record_id) = parent_record_id else {
                    continue;
                };
                let Some(parent_tala_id) = tala_by_record.get(parent_record_id).copied() else {
                    continue;
                };
                let child_order = if root_graph_snapshot {
                    trace_root_graph_child_ids(graph, parent_tala_id)
                } else {
                    let root_tala_id = graph
                        .nodes
                        .iter()
                        .find(|node| node.external_id == root)
                        .map(|node| node.tala_id);
                    trace_owner_phase_child_ids(
                        graph,
                        parent_tala_id,
                        phase,
                        root_tala_id,
                        root.is_empty() || root == "top",
                    )
                };
                let child_ranks = child_order
                    .into_iter()
                    .enumerate()
                    .map(|(rank, tala_id)| (tala_id, rank))
                    .collect::<BTreeMap<_, _>>();
                child_records.stable_sort_by_key(|child_record_id| {
                    tala_by_record
                        .get(child_record_id)
                        .and_then(|tala_id| child_ranks.get(tala_id))
                        .copied()
                        .unwrap_or(usize::MAX)
                });
            }
            fn visit_record(
                record_id: &str,
                children_by_parent: &BTreeMap<Option<String>, Vec<String>>,
                ranks: &mut BTreeMap<String, usize>,
                next_rank: &mut usize,
                seen_records: &mut BTreeSet<String>,
            ) {
                if !seen_records.insert(record_id.to_owned()) {
                    return;
                }
                ranks.insert(record_id.to_owned(), *next_rank);
                *next_rank += 1;
                if let Some(children) = children_by_parent.get(&Some(record_id.to_owned())) {
                    for child_id in children {
                        visit_record(child_id, children_by_parent, ranks, next_rank, seen_records);
                    }
                }
            }
            let root_ids = children_by_parent.get(&None).cloned().unwrap_or_default();
            let mut seen_records = BTreeSet::new();
            let mut next_rank = 0;
            for root_id in &root_ids {
                visit_record(
                    root_id,
                    &children_by_parent,
                    &mut ranks,
                    &mut next_rank,
                    &mut seen_records,
                );
            }
            // Retain any records whose owner is not represented in this
            // snapshot, preserving their current emitted order after the
            // reachable source hierarchy.
            for record in &nodes {
                visit_record(
                    &record.id,
                    &children_by_parent,
                    &mut ranks,
                    &mut next_rank,
                    &mut seen_records,
                );
            }
        }
        let root_uses_live_owner_walk = root.is_empty()
            && (initialized_root_snapshot || live_owner_snapshot)
            || aggregate_member_descendants_are_flat
            || recursive_owner_walk_snapshot
            || owner_graph_walk_snapshot;
        if owner_container_walk_applied {
            // `trace_nodes` is already the Go Graph.Nodes + Containers walk.
            // Later snapshot repair can move a descendant under an absorbed
            // member whose Go pointer has been replaced by a cluster vessel.
            // Reorder the records by this authoritative walk, preserving the
            // geometry computed by those repair passes.
            use std::collections::VecDeque;
            let fallback_ids = nodes.iter().map(|node| node.id.clone()).collect::<Vec<_>>();
            let mut records_by_id = BTreeMap::<String, VecDeque<PlacementTraceNode>>::new();
            for record in nodes.drain(..) {
                records_by_id
                    .entry(record.id.clone())
                    .or_default()
                    .push_back(record);
            }
            let mut ordered = Vec::with_capacity(fallback_ids.len());
            for node_id in &trace_nodes {
                let cluster_record_id = graph
                    .clusters
                    .iter()
                    .find(|cluster| cluster.members.first() == Some(node_id))
                    .map(|cluster| {
                        let members = cluster
                            .members
                            .iter()
                            .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let arrangement = match cluster.arrangement {
                            ClusterArrangement::Row => "Row",
                            ClusterArrangement::Column => "Column",
                        };
                        format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}")
                    })
                    .filter(|id| records_by_id.contains_key(id));
                let sequence_record_id = graph
                    .sequences
                    .iter()
                    .find(|sequence| sequence.members.first() == Some(node_id))
                    .map(|sequence| {
                        let members = sequence
                            .members
                            .iter()
                            .map(|member| graph.nodes[member.0 as usize].external_id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("Sequence vessel of: [{members}]")
                    })
                    .filter(|id| records_by_id.contains_key(id));
                let record_id = cluster_record_id
                    .or(sequence_record_id)
                    .unwrap_or_else(|| trace_node_external_id(graph, *node_id));
                if let Some(record) = records_by_id
                    .get_mut(&record_id)
                    .and_then(VecDeque::pop_front)
                {
                    ordered.push(record);
                }
            }
            for id in fallback_ids {
                if let Some(record) = records_by_id.get_mut(&id).and_then(VecDeque::pop_front) {
                    ordered.push(record);
                }
            }
            nodes = ordered;
            #[cfg(feature = "diagnostic-traces")]
            trace_placement_target_records(phase, root, "owner-walk", &nodes);
            #[cfg(feature = "diagnostic-traces")]
            trace_placement_order_records(phase, root, "owner-walk", &nodes);
        } else if !root_uses_live_owner_walk {
            nodes.stable_sort_by_key(|node| ranks.get(&node.id).copied().unwrap_or(usize::MAX));
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    if let Some(target) = crate::internal::trace_env_value("TALA_TRACE_TARGET_NODE")
        && matches!(phase, "after-herds" | "after-split")
    {
        eprintln!(
            "TRACE_FINAL_TARGET_RUST phase={phase} root={root} node={target} records={:?}",
            nodes
                .iter()
                .filter(|node| node.id == target)
                .map(|node| (
                    node.x_bits.as_deref(),
                    node.y_bits.as_deref(),
                    node.w_bits.as_str(),
                    node.h_bits.as_str()
                ))
                .collect::<Vec<_>>()
        );
    }
    if phase == "before-fit" {
        let positions = nodes
            .iter()
            .map(|node| {
                (
                    node.stable_id.clone(),
                    (node.x_bits.clone(), node.y_bits.clone()),
                )
            })
            .collect();
        if let Ok(mut snapshots) = TRACE_BEFORE_FIT_ROOT_POSITIONS
            .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
            .lock()
        {
            snapshots.insert(root.to_owned(), positions);
        }
    } else if matches!(phase, "after-fit-size" | "after-fit")
        && let Some(positions) = TRACE_BEFORE_FIT_ROOT_POSITIONS
            .get()
            .and_then(|snapshots| snapshots.lock().ok())
            .and_then(|snapshots| snapshots.get(root).cloned())
    {
        // Go's FitToGraph changes root dimensions but leaves every TopLeft
        // pointer in this owning graph untouched. Reuse the complete
        // before-fit position snapshot across both post-fit boundaries;
        // arena aliases can otherwise switch back to their local frame.
        for node in &mut nodes {
            if let Some((x_bits, y_bits)) = positions.get(&node.stable_id) {
                node.x_bits.clone_from(x_bits);
                node.y_bits.clone_from(y_bits);
            }
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    if phase == "after-fit-size" {
        if let Ok(mut snapshots) = TRACE_AFTER_FIT_SIZE_NODES
            .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
            .lock()
        {
            snapshots.insert(root.to_owned(), nodes.clone());
        }
    } else if phase == "after-fit"
        && let Some(snapshot) = TRACE_AFTER_FIT_SIZE_NODES
            .get()
            .and_then(|snapshots| snapshots.lock().ok())
            .and_then(|mut snapshots| snapshots.remove(root))
    {
        // Upstream emits after-fit immediately after after-fit-size with no
        // intervening mutation. Reuse that exact owner snapshot because the
        // Rust recursive copyback work that follows has a later boundary.
        nodes = snapshot;
    }
    #[cfg(feature = "diagnostic-traces")]
    if phase == "after-fit"
        && let Ok(mut synced) = TRACE_SYNCED_NODES
            .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
            .lock()
    {
        // Keep the shared-pointer cache in the same owner frame as the
        // serialized after-fit event. Later parent callbacks read this cache
        // to represent Go's already-updated Node pointers.
        for record in &nodes {
            if let Ok(tala_id) = record.stable_id.parse::<i64>()
                && let Some(node) = synced.get_mut(&tala_id)
            {
                node.position = trace_placement_node_position(record);
                if let (Ok(width_bits), Ok(height_bits)) = (
                    u64::from_str_radix(record.w_bits.trim_start_matches("0x"), 16),
                    u64::from_str_radix(record.h_bits.trim_start_matches("0x"), 16),
                ) {
                    node.rect.size = Size {
                        width: f64::from_bits(width_bits),
                        height: f64::from_bits(height_bits),
                    };
                }
                if let Some(position) = node.position {
                    node.rect.origin = position;
                }
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if let Ok(mut owner_frame_nodes) = TRACE_OWNER_FRAME_NODES
            .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
            .lock()
        {
            for record in &nodes {
                if let Ok(tala_id) = record.stable_id.parse::<i64>()
                    && let Some(node) = synced.get(&tala_id)
                {
                    owner_frame_nodes.insert(tala_id, node.clone());
                }
            }
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    let top_owner_frame_snapshot = (root.is_empty() || root == "top")
        && (phase == "optimized"
            || phase.starts_with("after-orthogonal-")
            || phase.starts_with("after-cluster-optimize-")
            || phase.starts_with("after-trees-")
            || phase.starts_with("after-restore-edges-")
            || phase.starts_with("after-direct-owner-")
            || phase.starts_with("after-subgraph-")
            || phase.starts_with("before-subgraph-")
            || phase == "after-direct"
            || matches!(phase, "before-combine" | "after-combine" | "after-orient"));
    #[cfg(feature = "diagnostic-traces")]
    if top_owner_frame_snapshot
        && let Ok(mut owner_frame_nodes) = TRACE_OWNER_FRAME_NODES
            .get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
            .lock()
    {
        // Go's nested Graph values share Node pointers with the owning graph.
        // When an orthogonal subgraph finishes, its `optimized` snapshot is
        // the latest geometry for each node in that subgraph; publish those
        // values into the owner-frame cache before the next callback snapshots
        // the wider owner graph. Nodes omitted from the subgraph retain their
        // prior owner positions.
        for record in &nodes {
            if let Ok(tala_id) = record.stable_id.parse::<i64>()
                && let Some(node) = owner_frame_nodes.get_mut(&tala_id)
            {
                node.position = trace_placement_node_position(record);
                if let (Ok(width_bits), Ok(height_bits)) = (
                    u64::from_str_radix(record.w_bits.trim_start_matches("0x"), 16),
                    u64::from_str_radix(record.h_bits.trim_start_matches("0x"), 16),
                ) {
                    node.rect.size = Size {
                        width: f64::from_bits(width_bits),
                        height: f64::from_bits(height_bits),
                    };
                }
                if let Some(position) = node.position {
                    node.rect.origin = position;
                }
            }
        }
    }
    #[cfg(feature = "diagnostic-traces")]
    trace_placement_order_records(phase, root, "serialized", &nodes);
    let event = PlacementTraceEvent {
        event: "placement",
        phase: phase.to_owned(),
        root: if root.is_empty() {
            // Go calls the top-level placement root "top" in its trace even
            // though there is no graph node representing that root.
            "top".to_owned()
        } else {
            root.to_owned()
        },
        // Go's after-fit callback receives the owning graph, not the temporary
        // child graph whose computed cell size drives placement. For a
        // GraphJSON replay, retain the serialized owner value through the
        // nested arena scopes; ordinary D2 inputs have no serialized value and
        // therefore expose the owner's initial zero here.
        cell_bits: trace_float_bits(_cell_size_override.unwrap_or_else(|| {
            if phase == "after-fit" || root_graph_snapshot || owner_snapshot {
                #[cfg(feature = "diagnostic-traces")]
                {
                    graph.source_cell_size
                }
                #[cfg(not(feature = "diagnostic-traces"))]
                {
                    0.0
                }
            } else {
                graph.cell_size
            }
        })),
        graph_nodes: graph
            .graph_node_order()
            .iter()
            .map(|node| trace_node_external_id(graph, *node))
            .collect(),
        graph_edges: graph
            .edge_order
            .read()
            .iter()
            .map(|edge_id| {
                let edge = &graph.edge_records[edge_id.0 as usize];
                // The source graph keeps the original endpoint pointers even
                // while a temporary Rust projection collapses both ends onto
                // one carrier. Recover that identity by input edge order
                // before falling back to the live arena endpoint.
                // Before/after-direct run around RestoreEdgeAbductions, so
                // the source endpoint table is the pointer-visible identity
                // there. After-fit is different: Go has already applied the
                // container fit and its live endpoint may have collapsed to
                // the fitted ancestor. Preserve that phase's live traversal.
                let use_source_endpoints = matches!(phase, "before-direct" | "after-direct");
                let after_fit_external_source = (phase == "after-fit")
                    .then(|| {
                        let root_id = graph
                            .nodes
                            .iter()
                            .find(|node| node.external_id == root)
                            .map(|node| node.input_id)?;
                        (!graph.is_descendant_of(edge.to, root_id)).then_some(())
                    })
                    .flatten()
                    .is_some();
                let from = use_source_endpoints
                    .then(|| trace_source_endpoint_name(*edge_id, true))
                    .flatten()
                    .or_else(|| {
                        (phase == "after-fit")
                            .then(|| trace_after_fit_endpoint_name(*edge_id, true))
                            .flatten()
                    })
                    .or_else(|| {
                        after_fit_crossing_endpoint(edge.from, edge.to)
                            .map(|root_id| trace_node_external_id(graph, root_id))
                    })
                    .or_else(|| {
                        after_fit_external_source.then(|| trace_endpoint_name(*edge_id, edge.from))
                    })
                    .unwrap_or_else(|| {
                        if phase == "after-fit" {
                            trace_node_external_id(graph, edge.from)
                        } else {
                            trace_endpoint_name(*edge_id, edge.from)
                        }
                    });
                let to = use_source_endpoints
                    .then(|| trace_source_endpoint_name(*edge_id, false))
                    .flatten()
                    .or_else(|| {
                        (phase == "after-fit")
                            .then(|| trace_after_fit_endpoint_name(*edge_id, false))
                            .flatten()
                    })
                    .or_else(|| {
                        after_fit_crossing_endpoint(edge.to, edge.from)
                            .map(|root_id| trace_node_external_id(graph, root_id))
                    })
                    .unwrap_or_else(|| {
                        if phase == "after-fit" {
                            trace_node_external_id(graph, edge.to)
                        } else {
                            trace_endpoint_name(*edge_id, edge.to)
                        }
                    });
                PlacementTraceEdge { from, to }
            })
            .collect(),
        nodes,
    };
    if let Ok(encoded) = serde_json::to_string(&event) {
        eprintln!("{encoded}");
    }
}

// The pinned Go darwin/arm64 math.Min/Max wrappers give the opposite infinity
// precedence over NaNs, then use FMIN/FMAX. Plan 9's operand order presents b
// before a to ARM64 ProcessNaNs: signaling values first, then quiet values.
fn stage_nan_operand(a: f64, b: f64) -> Option<f64> {
    const QUIET: u64 = 1 << 51;
    for value in [b, a] {
        if value.is_nan() && value.to_bits() & QUIET == 0 {
            return Some(f64::from_bits(value.to_bits() | QUIET));
        }
    }
    if b.is_nan() {
        Some(b)
    } else if a.is_nan() {
        Some(a)
    } else {
        None
    }
}

pub(super) fn stage_minimum(a: f64, b: f64) -> f64 {
    if a == f64::NEG_INFINITY || b == f64::NEG_INFINITY {
        f64::NEG_INFINITY
    } else if let Some(value) = stage_nan_operand(a, b) {
        value
    } else if a == b && a == 0.0 {
        f64::from_bits(a.to_bits() | b.to_bits())
    } else if a < b {
        a
    } else {
        b
    }
}

pub(super) fn stage_maximum(a: f64, b: f64) -> f64 {
    if a == f64::INFINITY || b == f64::INFINITY {
        f64::INFINITY
    } else if let Some(value) = stage_nan_operand(a, b) {
        value
    } else if a == b && a == 0.0 {
        f64::from_bits(a.to_bits() & b.to_bits())
    } else if a > b {
        a
    } else {
        b
    }
}

impl Pipeline {
    /// `Graph.BoundingBox` consumed by Go's `runAllStages`. Temporary vessels
    /// contribute their own geometry, never the retained carrier's labels,
    /// fixed position, shape modifiers, or hidden member boxes.
    pub(super) fn stage_graph_bounds(&self) -> Option<(Point, Point)> {
        let graph = &self.graph;
        let nodes = graph.graph_node_order_cow();
        // runLayout bypasses an empty input graph in the source.
        if nodes.is_empty() {
            return None;
        }
        let mut low = Point {
            x: f64::INFINITY,
            y: f64::INFINITY,
        };
        let mut high = Point {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
        };
        for &id in nodes.iter() {
            // Source Nodes.boundingBox returns nil if any current node is
            // unplaced; an already-placed subset cannot trigger this gate.
            let position = graph.active_node_position(id)?;
            let size = graph.active_node_size(id);
            let mut tl = position;
            let mut br = Point {
                x: (position.x + size.width).round(),
                y: (position.y + size.height).round(),
            };
            if !graph.active_node_is_aggregate(id) {
                let node = &graph.nodes[id.0 as usize];
                let (dx, dy) = bounds::modifier_element_adjustments(node);
                let [top, right, bottom, left] = loop_cardinal_extents(node.loop_offsets.as_ref());
                if dx != 0.0 || dy != 0.0 {
                    tl.y -= dy;
                    br.x += dx;
                }
                tl.x -= left;
                tl.y -= top;
                br.x += right;
                br.y += bottom;
                if let Some((side, alignment)) =
                    ArenaGraph::outside_side_alignment(node.label_position)
                    && let Some(label_size) = node.label_size
                {
                    let label = ExternalLabel {
                        size: label_size,
                        side,
                        alignment,
                        automatic: false,
                        reserve_space: true,
                    };
                    let label_tl = label.top_left(
                        Rect {
                            origin: position,
                            size,
                        },
                        5.0,
                    );
                    let boundary = |horizontal: bool, upper: bool| {
                        nodes.iter().all(|&other| {
                            let Some(other_position) = graph.active_node_position(other) else {
                                return true;
                            };
                            let other_size = graph.active_node_size(other);
                            match (horizontal, upper) {
                                (true, true) => other_position.x >= position.x,
                                (false, true) => other_position.y >= position.y,
                                (true, false) => {
                                    other_position.x + other_size.width <= position.x + size.width
                                }
                                (false, false) => {
                                    other_position.y + other_size.height <= position.y + size.height
                                }
                            }
                        })
                    };
                    if label_tl.x < tl.x {
                        tl.x = (label_tl.x - if boundary(true, true) { 5.0 } else { 10.0 }).floor();
                    }
                    if label_tl.y < tl.y {
                        tl.y =
                            (label_tl.y - if boundary(false, true) { 5.0 } else { 10.0 }).floor();
                    }
                    if label_tl.x > br.x {
                        br.x = (label_tl.x
                            + label_size.width
                            + if boundary(true, false) { 5.0 } else { 10.0 })
                        .ceil();
                    }
                    if label_tl.y > br.y {
                        br.y = (label_tl.y
                            + label_size.height
                            + if boundary(false, false) { 5.0 } else { 10.0 })
                        .ceil();
                    }
                }
                if node.shape != ShapeKind::Image
                    && let Some((side, alignment)) = node
                        .icon_position
                        .and_then(ArenaGraph::outside_side_alignment)
                {
                    let icon = ExternalLabel {
                        size: Size {
                            width: 64.0,
                            height: 64.0,
                        },
                        side,
                        alignment,
                        automatic: false,
                        reserve_space: true,
                    };
                    let icon_tl = icon.top_left(
                        Rect {
                            origin: position,
                            size,
                        },
                        5.0,
                    );
                    let value = (icon_tl.x - 10.0).floor();
                    if value < tl.x {
                        tl.x = value;
                    }
                    let value = (icon_tl.y - 10.0).floor();
                    if value < tl.y {
                        tl.y = value;
                    }
                    let value = (icon_tl.x + 74.0).ceil();
                    if value > br.x {
                        br.x = value;
                    }
                    let value = (icon_tl.y + 74.0).ceil();
                    if value > br.y {
                        br.y = value;
                    }
                }
            }
            low.x = stage_minimum(low.x, tl.x);
            low.y = stage_minimum(low.y, tl.y);
            high.x = stage_maximum(high.x, br.x);
            high.y = stage_maximum(high.y, br.y);
        }
        // Nodes.fixedOrigin considers only direct children of the outermost
        // container represented by this live node slice.
        let mut outer = None;
        let mut outer_depth = usize::MAX;
        for &id in nodes.iter() {
            let Some(container) = graph.active_node_container(id) else {
                outer = None;
                break;
            };
            let mut depth = 0;
            let mut current = Some(container);
            while let Some(parent) = current {
                depth += 1;
                current = graph.active_node_container(parent);
            }
            if depth < outer_depth {
                outer_depth = depth;
                outer = Some(container);
            }
        }
        for &id in nodes.iter() {
            if graph.active_node_is_aggregate(id) || graph.active_node_container(id) != outer {
                continue;
            }
            if let Some(fixed) = graph.nodes[id.0 as usize].fixed_top_left {
                let position = graph.active_node_position(id)?;
                low = Point {
                    x: position.x - fixed.x,
                    y: position.y - fixed.y,
                };
                break;
            }
        }
        for &id in &graph.edge_order.read() {
            let edge = &graph.edge_records[id.0 as usize];
            let Some(&first) = edge.route.read().first() else {
                continue;
            };
            let mut tl = first;
            let mut br = first;
            for &point in &edge.route.read() {
                tl.x = stage_minimum(tl.x, point.x);
                tl.y = stage_minimum(tl.y, point.y);
                br.x = stage_maximum(br.x, point.x);
                br.y = stage_maximum(br.y, point.y);
            }
            if let Some(label) = &edge.label
                && let Some(label_tl) = labels::edge_label_top_left(
                    edge,
                    label.position,
                    label.percentage,
                    label.size.width,
                    label.size.height,
                )
            {
                tl.x = stage_minimum(tl.x, label_tl.x);
                tl.y = stage_minimum(tl.y, label_tl.y);
                br.x = stage_maximum(br.x, label_tl.x + label.size.width);
                br.y = stage_maximum(br.y, label_tl.y + label.size.height);
            }
            for is_target in [false, true] {
                if let Some(label) =
                    labels::arrowhead_label_rect_for_route(edge, &edge.route.read(), is_target)
                {
                    tl.x = stage_minimum(tl.x, label.origin.x);
                    tl.y = stage_minimum(tl.y, label.origin.y);
                    br.x = stage_maximum(br.x, label.right());
                    br.y = stage_maximum(br.y, label.bottom());
                }
            }
            if !tl.x.is_infinite() {
                low.x = stage_minimum(low.x, tl.x.round());
                low.y = stage_minimum(low.y, tl.y.round());
                high.x = stage_maximum(high.x, br.x.round());
                high.y = stage_maximum(high.y, br.y.round());
            }
        }
        Some((
            Point {
                x: low.x.round(),
                y: low.y.round(),
            },
            Point {
                x: high.x.round(),
                y: high.y.round(),
            },
        ))
    }

    pub(super) fn check_stage_dimensions(
        &self,
        stage: &'static str,
    ) -> Result<(), crate::LayoutError> {
        // The source checks ctx.Err before checking overflow at the boundary.
        if crate::internal::cancellation_requested() {
            return Err(crate::internal::cancellation_layout_error()
                .unwrap_or(crate::LayoutError::Cancelled));
        }
        if let Some((tl, br)) = self.stage_graph_bounds() {
            let width = br.x - tl.x;
            let height = br.y - tl.y;
            if width > 30_000.0 || height > 30_000.0 {
                return Err(crate::LayoutError::StageDimensions {
                    stage,
                    width,
                    height,
                });
            }
        }
        Ok(())
    }

    /// Mirrors the pinned adapter's final selected-graph snapshot. This is a
    /// diagnostic boundary after seed racing and the compound-candidate pass,
    /// not another algorithm stage.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) fn trace_selected_result(input: &Graph, seed: i64, graph: ArenaGraph) {
        if !crate::internal::trace_env_enabled("TALA_TRACE_JSONL") {
            return;
        }
        let mut selected = Self::new_untraced(input, seed, true, false);
        selected.graph = graph;
        selected.emit_normalized_trace("SelectedResult");
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_cluster_policy_stage(&self, stage: &str) {
        if !crate::internal::trace_env_enabled("D2RS_TRACE_CLUSTER_POLICY_STAGE") {
            return;
        }
        use crate::internal::owner_groups::OwnerGroupKind;
        use crate::internal::shared_topology::OwnerGraphId;
        let graph = &self.graph;
        let node_id =
            |id: Option<NodeId>| id.map(|n| graph.nodes[n.0 as usize].tala_id.to_string());
        let records = (|| {
            let map = graph.owner_graph_group_map(OwnerGraphId(0), OwnerGroupKind::Cluster)?;
            let mut keys = graph.owner_group_map_keys(&map)?;
            keys.stable_sort_by_key(|id| id.map(|n| graph.nodes[n.0 as usize].tala_id));
            let mut records = Vec::new();
            for key in keys {
                let Some(group) = graph.owner_group_map_entry(&map, key)?.flatten() else {
                    records.push(serde_json::json!({"key":node_id(key),"cluster":null}));
                    continue;
                };
                let fields = graph.owner_group_fields(group)?;
                let policy = graph.owner_cluster_policy(group)?;
                let members = if fields.nodes.is_nil() {
                    None
                } else {
                    Some(fields.nodes.iter_slots().map(node_id).collect::<Vec<_>>())
                };
                let owner_nodes = fields
                    .graph
                    .map(|owner| graph.owner_graph_member_header(owner))
                    .transpose()?
                    .and_then(|header| {
                        if header.is_nil() {
                            None
                        } else {
                            Some(header.iter_slots().map(node_id).collect::<Vec<_>>())
                        }
                    });
                records.push(serde_json::json!({"key":node_id(key),"vessel":node_id(fields.vessel),"members":members,
                    "arrangement":policy.arrangement,"desired":policy.desired_arrangement,
                    "padding_bits":format!("0x{:016x}",policy.padding.to_bits()),"fixed":policy.fixed_size,"owner_nodes":owner_nodes}));
            }
            Ok::<_, crate::internal::shared_topology::OwnerTopologyError>(records)
        })();
        match records {
            Ok(records) => eprintln!(
                "{}",
                serde_json::json!({"event":"cluster-policy-stage","stage":stage,"clusters":records})
            ),
            Err(error) => eprintln!(
                "{}",
                serde_json::json!({"event":"cluster-policy-stage","stage":stage,"error":error.to_string()})
            ),
        }
    }

    pub(super) fn trace_node_stage(&mut self, stage: &str) {
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("D2RS_TRACE_ACTUAL_OWNER_STAGE") {
            eprintln!(
                "{}",
                serde_json::json!({
                    "diagnostic": "actual-owner-stage", "stage": stage,
                    "nodes": self.graph.nodes.iter().enumerate().map(|(index, node)| {
                        let canonical = NodeId(index as u32);
                        serde_json::json!({
                            "canonical": canonical.0, "id": node.external_id,
                            "tala_id": node.tala_id, "input": node.input_id.0,
                            "inline_position": node.position.map(|point| [point.x, point.y]),
                            "actual_position": self.graph.position(canonical).map(|point| [point.x, point.y]),
                            "size": [node.rect.size.width, node.rect.size.height],
                            "container": node.container.map(|node| node.0),
                            "graph": self.graph.topology_node_owner(canonical).map(|graph| graph.0),
                            "herd": format!("{:?}", self.graph.herd_assignment_snapshot(canonical)),
                        })
                    }).collect::<Vec<_>>(),
                })
            );
        }
        self.emit_normalized_trace(stage);
        #[cfg(feature = "diagnostic-traces")]
        self.trace_container_child_order(stage);
        if crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_STAGE") {
            for (cluster_index, cluster) in self.graph.clusters.iter().enumerate() {
                let members = cluster
                    .members
                    .iter()
                    .map(|member| {
                        let node = &self.graph.nodes[member.0 as usize];
                        (node.tala_id, node.position, node.rect.size)
                    })
                    .collect::<Vec<_>>();
                eprintln!(
                    "CLUSTER_STAGE_RUST stage={} vessel={} vessel_pos={:?} pending_vessel_pos={:?} vessel_size={:?} members={:?}",
                    stage,
                    cluster.vessel_tala_id,
                    self.graph.position(cluster.members[0]),
                    self.graph
                        .pending_cluster_vessel_positions
                        .get(&cluster_index),
                    self.graph.cluster_vessel_size(cluster_index),
                    members,
                );
                if crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_STATE") {
                    let owner = cluster.members.first().copied();
                    let owner_state = owner.map(|node| {
                        let index = node.0 as usize;
                        (
                            node.0,
                            self.graph.nodes[index].cluster,
                            self.graph
                                .active_cluster_indices
                                .get(index)
                                .copied()
                                .flatten(),
                            self.graph.active_cluster_owners.get(index).copied(),
                            self.graph.node_order_membership.get(index).copied(),
                        )
                    });
                    eprintln!(
                        "CLUSTER_STATE_RUST stage={} idx={} active_flags={:?} owner_state={:?} members={:?}",
                        stage,
                        cluster_index,
                        self.graph.active_cluster_flags.get(cluster_index),
                        owner_state,
                        cluster.members,
                    );
                }
            }
        }
        let Some(target) = crate::internal::trace_env_value("TALA_TRACE_NODE_STAGES") else {
            return;
        };
        if let Some(filter) = crate::internal::trace_env_value("TALA_TRACE_STAGE_FILTER")
            && filter != stage
        {
            return;
        }
        for node in &self.graph.nodes {
            if target != "all" && target.parse::<i64>().ok() != Some(node.tala_id) {
                continue;
            }
            eprint!(
                "NODE_STAGE_RUST stage={stage} node={} size={},{} position=",
                node.tala_id, node.rect.size.width, node.rect.size.height
            );
            if let Some(position) = node.position {
                eprintln!("{},{}", position.x, position.y);
            } else {
                eprintln!("nil");
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_NODE_META") {
                eprintln!(
                    "NODE_META_RUST stage={stage} tala={} input={} container={:?} is_container={} scoring_is_container={} hierarchy={:?} cluster={:?} sequence={:?} active_cluster={:?} active_sequence={:?} aggregate={} scoring_cluster_vessel={:?} scoring_aggregate={}",
                    node.tala_id,
                    node.input_id.0,
                    node.container.map(|id| id.0),
                    node.is_container,
                    node.scoring_is_container,
                    node.hierarchy.clone(),
                    node.cluster,
                    node.sequence,
                    self.graph.active_cluster_index(node.input_id),
                    self.graph.active_sequence_index(node.input_id),
                    node.scoring_is_aggregate_vessel,
                    node.scoring_cluster_vessel,
                    node.scoring_is_aggregate_vessel,
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_NODE_CHILDREN") {
                let children = self
                    .graph
                    .containers
                    .get(&Some(node.input_id))
                    .into_iter()
                    .flatten()
                    .map(|child| self.graph.nodes[child.0 as usize].tala_id)
                    .collect::<Vec<_>>();
                eprintln!(
                    "NODE_CHILDREN_RUST stage={stage} tala={} children={children:?} descendants={:?}",
                    node.tala_id,
                    self.graph
                        .descendants(node.input_id)
                        .iter()
                        .map(|child| self.graph.nodes[child.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_GRID_STAGE_CHILDREN")
                && (node.tala_id == 1_149_337_423 || node.tala_id == 1_782_109_120)
            {
                let children = self
                    .graph
                    .containers
                    .get(&Some(node.input_id))
                    .into_iter()
                    .flatten()
                    .map(|child| {
                        let child = &self.graph.nodes[child.0 as usize];
                        (child.tala_id, child.position, child.rect.size)
                    })
                    .collect::<Vec<_>>();
                eprintln!(
                    "GRID_STAGE_CHILDREN_RUST stage={stage} parent={} pos={:?} children={children:?}",
                    node.tala_id, node.position
                );
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ROOT_CHILDREN") {
            eprint!("ROOT_CHILDREN_RUST stage={stage}");
            for child in self.graph.containers.get(&None).into_iter().flatten() {
                eprint!(" {}", self.graph.nodes[child.0 as usize].tala_id);
            }
            eprintln!();
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_GRAPH_EDGES") {
            eprint!("GRAPH_STATE_RUST stage={stage} edges=");
            for edge in self.graph.member_edges() {
                eprint!(
                    "{}>{},",
                    self.graph.nodes[edge.from.0 as usize].tala_id,
                    self.graph.nodes[edge.to.0 as usize].tala_id
                );
            }
            eprintln!();
            eprint!("GRAPH_ORDER_EDGES_RUST stage={stage} edges=");
            for edge_id in &self.graph.edge_order.read() {
                let edge = &self.graph.edge_records[edge_id.0 as usize];
                eprint!(
                    "{}>{},",
                    self.graph.nodes[edge.from.0 as usize].tala_id,
                    self.graph.nodes[edge.to.0 as usize].tala_id
                );
            }
            eprintln!();
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ACTIVE_ORDER") {
            eprint!("ACTIVE_ORDER_RUST stage={stage}");
            for node in &self.graph.node_order {
                let tala = self.graph.nodes[node.0 as usize].tala_id;
                let position = self.graph.active_node_position(*node);
                let size = self.graph.active_node_size(*node);
                eprint!(
                    " tala={} input={} pos={:?} size={},{}",
                    tala, node.0, position, size.width, size.height
                );
            }
            eprintln!();
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_STAGE") {
            for (cluster_index, cluster) in self.graph.clusters.iter().enumerate() {
                eprint!(
                    "CLUSTER_STAGE_RUST stage={stage} vessel={} pending={:?}",
                    cluster.vessel_tala_id,
                    self.graph
                        .pending_cluster_vessel_positions
                        .get(&cluster_index)
                );
                for member in &cluster.members {
                    let node = &self.graph.nodes[member.0 as usize];
                    eprint!(
                        " member={} pos={:?} size={:?}",
                        node.tala_id, node.position, node.rect.size
                    );
                }
                eprintln!();
            }
        }
    }

    #[cfg(feature = "diagnostic-traces")]
    fn trace_container_child_order(&self, stage: &str) {
        let Some(target) = crate::internal::trace_env_value("D2RS_TRACE_CONTAINER_CHILD_ORDER")
        else {
            return;
        };
        let emit = |container: String, children: Vec<String>| {
            if target != "all" && target != container {
                return;
            }
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "container-child-order-stage",
                    "stage": stage,
                    "container": container,
                    "children": children,
                })
            );
        };
        emit(
            "0".to_owned(),
            self.graph
                .container_node_order(None)
                .into_iter()
                .map(|child| {
                    self.graph
                        .active_node_tala_id(self.graph.active_aggregate_owner(child))
                        .to_string()
                })
                .collect(),
        );
        for node in self.graph.nodes.iter().filter(|node| node.is_container) {
            emit(
                self.graph.active_node_tala_id(node.input_id).to_string(),
                self.graph
                    .container_node_order(Some(node.input_id))
                    .into_iter()
                    .map(|child| {
                        self.graph
                            .active_node_tala_id(self.graph.active_aggregate_owner(child))
                            .to_string()
                    })
                    .collect(),
            );
        }
    }

    /// Emits one deterministic JSONL stage snapshot for cross-language
    /// differential debugging. Floating-point values are represented by their
    /// exact IEEE-754 bit patterns; no timing, address, or thread data is
    /// included. This path is enabled only by the diagnostic-traces feature.
    pub(super) fn emit_normalized_trace(&mut self, stage: &str) {
        #[cfg(feature = "diagnostic-traces")]
        self.trace_cluster_policy_stage(stage);
        #[cfg(feature = "diagnostic-traces")]
        if let Some(path) = crate::internal::trace_env_value("D2RS_TRACE_FULL_STAGE_GRAPHJSON") {
            let state = (|| {
                let data = std::fs::read(path)?;
                let input: serde_json::Value = serde_json::from_slice(&data)?;
                crate::internal::graphjson::diagnostic_graphjson_state(&self.graph, &input)
            })();
            match state {
                Ok(graph) => eprintln!(
                    "{}",
                    serde_json::json!({"event":"full-graphjson-stage","stage":stage,"graph":graph})
                ),
                Err(error) => eprintln!(
                    "{}",
                    serde_json::json!({"event":"full-graphjson-stage","stage":stage,"error":error.to_string()})
                ),
            }
        }

        if !crate::internal::trace_env_enabled("TALA_TRACE_JSONL") {
            return;
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_RAW_STAGE") {
            eprintln!(
                "{}",
                serde_json::json!({
                    "diagnostic": "raw-stage", "stage": stage,
                    "node_order": self.graph.node_order.iter().map(|id| self.graph.nodes[id.0 as usize].tala_id).collect::<Vec<_>>(),
                    "edge_order": self.graph.edge_order.read().iter().map(|id| self.graph.edge_records[id.0 as usize].tala_id).collect::<Vec<_>>(),
                    "external_container_children": self.graph.transaction_external_container_children
                        .iter().map(|(parent, children)| serde_json::json!({
                            "parent_tala_id": parent,
                            "children": children.iter().map(|node| serde_json::json!({
                                "id": node.external_id, "tala_id": node.tala_id,
                                "position": node.position.map(|p| [p.x, p.y]),
                            })).collect::<Vec<_>>(),
                        })).collect::<Vec<_>>(),
                    "container_padding": self.graph.nodes.iter().filter(|node| node.is_container)
                        .map(|node| {
                            let ordinary = self.graph.shape_fit_padding_with_children(node.input_id, false);
                            let children = self.graph.shape_fit_padding_with_children(node.input_id, true);
                            serde_json::json!({
                                "id": node.external_id, "tala_id": node.tala_id,
                                "ordinary": [ordinary.left, ordinary.top],
                                "with_children": [children.left, children.top],
                            })
                        }).collect::<Vec<_>>(),
                    "nodes": self.graph.nodes.iter().map(|node| serde_json::json!({
                        "id": self.trace_node_id(node.input_id), "tala_id": node.tala_id,
                        "position": node.position.map(|p| [p.x, p.y]),
                        "width": node.rect.size.width, "height": node.rect.size.height,
                        "hierarchy": node.hierarchy.clone().map(|h| h.id),
                        "scope": node.container.map(|n| self.trace_node_id(n)),
                        "active": self.graph.node_order.contains(&node.input_id),
                    })).collect::<Vec<_>>(),
                })
            );
        }
        // Recursive copyback publishes shared descendant geometry. Observe
        // that live state without refitting hidden cluster children for tracing.
        let trace_nodes = if self.graph.node_order.is_empty() {
            self.graph
                .nodes
                .iter()
                .map(|node| node.input_id)
                .collect::<Vec<_>>()
        } else {
            self.graph.node_order.clone()
        };
        let nodes = trace_nodes
            .iter()
            .filter_map(|node_id| self.trace_node(*node_id))
            .collect();
        // TALA's trace walks Graph.Edges, which contains only the currently
        // active edge slice after tree preprocessing. The stable arena keeps
        // extracted tree edges for later routing, so use edge_order here to
        // avoid exposing inactive retained edges in the diagnostic protocol.
        let edges = self
            .graph
            .edge_order
            .read()
            .iter()
            .enumerate()
            .map(|(index, edge_id)| {
                let edge = &self.graph.edge_records[edge_id.0 as usize];
                NormalizedTraceEdge {
                    // `input_id` is the dense Rust arena slot. The Go trace
                    // records the serialized EntityID, which survives edge
                    // removal/reconnection and is the identity needed for a
                    // cross-language event comparison.
                    id: edge.tala_id.to_string(),
                    index,
                    from: self.trace_node_id(edge.from),
                    to: self.trace_node_id(edge.to),
                    curve: edge.is_curve,
                    route: edge
                        .route
                        .read()
                        .iter()
                        .map(|point| NormalizedTracePoint {
                            x_bits: trace_float_bits(point.x),
                            y_bits: trace_float_bits(point.y),
                        })
                        .collect(),
                }
            })
            .collect();
        let event = NormalizedTraceEvent {
            event: "stage",
            stage: stage.to_owned(),
            seed: self.seed,
            nodes,
            edges,
        };
        if let Ok(encoded) = serde_json::to_string(&event) {
            eprintln!("{encoded}");
        }
    }

    /// Returns the D2-visible identity and geometry for one active node.
    ///
    /// D2 temporarily replaces clustered members with a vessel node. The Rust
    /// production arena retains stable member IDs, so diagnostic
    /// traces project that view here instead of exposing an implementation
    /// detail to the cross-language comparator.
    fn trace_node(&self, node_id: NodeId) -> Option<NormalizedTraceNode> {
        let cluster = self.trace_cluster_for_node(node_id);
        if let Some((cluster_index, cluster)) = cluster {
            let members_visible = cluster
                .members
                .iter()
                .all(|member| self.graph.node_order.contains(member));
            if !members_visible {
                let first = *cluster.members.first()?;
                if node_id != first {
                    return None;
                }
                let position = self
                    .graph
                    .pending_cluster_vessel_positions
                    .get(&cluster_index)
                    .copied()
                    .or_else(|| self.graph.active_node_position(first));
                let size = self.graph.cluster_vessel_size(cluster_index);
                return Some(NormalizedTraceNode {
                    id: self.trace_cluster_id(cluster),
                    x_bits: position.map(|point| trace_float_bits(point.x)),
                    y_bits: position.map(|point| trace_float_bits(point.y)),
                    w_bits: trace_float_bits(size.width),
                    h_bits: trace_float_bits(size.height),
                    // Cluster vessels are synthetic nodes. The representative
                    // member retains its own label/icon state in the Rust arena,
                    // while D2's replacement vessel starts without either.
                    label_position: None,
                    icon_position: None,
                });
            }
            // A decoded graphjson snapshot still exposes its cluster members
            // in Graph.Nodes. Go's trace therefore reports each member's
            // serialized/live box until AddClusters removes those pointers;
            // active_node_position/size would incorrectly substitute the
            // hidden vessel as soon as the cluster metadata is loaded.
            if members_visible {
                let node = &self.graph.nodes[node_id.0 as usize];
                let (label_position, icon_position) = self.trace_node_label_positions(node_id);
                return Some(NormalizedTraceNode {
                    id: self.trace_node_id(node_id),
                    x_bits: node.position.map(|point| trace_float_bits(point.x)),
                    y_bits: node.position.map(|point| trace_float_bits(point.y)),
                    w_bits: trace_float_bits(node.rect.size.width),
                    h_bits: trace_float_bits(node.rect.size.height),
                    label_position,
                    icon_position,
                });
            }
        }
        if let Some((sequence_index, sequence)) = self.trace_sequence_for_node(node_id) {
            let members_visible = sequence
                .members
                .iter()
                .all(|member| self.graph.node_order.contains(member));
            if !members_visible {
                let first = *sequence.members.first()?;
                if node_id != first {
                    return None;
                }
                let position = self.graph.active_node_position(first);
                let size = self.graph.sequence_vessel_size(sequence_index);
                let (label_position, icon_position) = self.trace_node_label_positions(node_id);
                return Some(NormalizedTraceNode {
                    id: self.trace_sequence_id(sequence),
                    x_bits: position.map(|point| trace_float_bits(point.x)),
                    y_bits: position.map(|point| trace_float_bits(point.y)),
                    w_bits: trace_float_bits(size.width),
                    h_bits: trace_float_bits(size.height),
                    label_position,
                    icon_position,
                });
            }
            if members_visible {
                let node = &self.graph.nodes[node_id.0 as usize];
                let (label_position, icon_position) = self.trace_node_label_positions(node_id);
                return Some(NormalizedTraceNode {
                    id: self.trace_node_id(node_id),
                    x_bits: node.position.map(|point| trace_float_bits(point.x)),
                    y_bits: node.position.map(|point| trace_float_bits(point.y)),
                    w_bits: trace_float_bits(node.rect.size.width),
                    h_bits: trace_float_bits(node.rect.size.height),
                    label_position,
                    icon_position,
                });
            }
        }
        let position = self.graph.active_node_position(node_id);
        let size = self.graph.active_node_size(node_id);
        let (label_position, icon_position) = self.trace_node_label_positions(node_id);
        Some(NormalizedTraceNode {
            id: self.trace_node_id(node_id),
            x_bits: position.map(|point| trace_float_bits(point.x)),
            y_bits: position.map(|point| trace_float_bits(point.y)),
            w_bits: trace_float_bits(size.width),
            h_bits: trace_float_bits(size.height),
            label_position,
            icon_position,
        })
    }

    fn trace_node_label_positions(&self, node_id: NodeId) -> (Option<String>, Option<String>) {
        let node = &self.graph.nodes[node_id.0 as usize];
        let label_position = node.label_size.and_then(|_| {
            let name = node.label_position.d2_name();
            (!name.is_empty()).then(|| name.to_owned())
        });
        let icon_position = node
            .has_icon
            .then(|| {
                node.icon_position
                    .unwrap_or(crate::LabelPosition::Unset)
                    .d2_name()
                    .to_owned()
            })
            .filter(|name| !name.is_empty());
        (label_position, icon_position)
    }

    fn trace_node_id(&self, node_id: NodeId) -> String {
        if let Some((cluster_index, cluster)) = self.trace_cluster_for_node(node_id) {
            let members_visible = cluster
                .members
                .iter()
                .all(|member| self.graph.node_order.contains(member));
            if !members_visible {
                return self.trace_cluster_id(cluster);
            }
            let _ = cluster_index;
        }
        if let Some((_, sequence)) = self.trace_sequence_for_node(node_id) {
            let members_visible = sequence
                .members
                .iter()
                .all(|member| self.graph.node_order.contains(member));
            if !members_visible {
                return self.trace_sequence_id(sequence);
            }
        }
        // Upstream Node.DebugID prefers D2ID over the internal numeric ID.
        // Generated carriers without an input node keep their numeric fallback.
        let node = &self.graph.nodes[node_id.0 as usize];
        self.input.node(node.input_id).map_or_else(
            || node.tala_id.to_string(),
            |input| input.external_id.clone(),
        )
    }

    fn trace_cluster_for_node(&self, node_id: NodeId) -> Option<(usize, &ClusterState)> {
        self.graph
            .clusters
            .iter()
            .enumerate()
            .find(|(_, cluster)| cluster.members.contains(&node_id))
    }

    fn trace_cluster_id(&self, cluster: &ClusterState) -> String {
        let arrangement = match cluster.arrangement {
            ClusterArrangement::Row => "Row",
            ClusterArrangement::Column => "Column",
        };
        let members = cluster
            .members
            .iter()
            .map(|member| {
                let node = &self.graph.nodes[member.0 as usize];
                self.input.node(node.input_id).map_or_else(
                    || node.tala_id.to_string(),
                    |input| input.external_id.clone(),
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("Cluster vessel of: [{members}]; Arrangement: {arrangement}")
    }

    fn trace_sequence_for_node(&self, node_id: NodeId) -> Option<(usize, &SequenceState)> {
        self.graph
            .sequences
            .iter()
            .enumerate()
            .find(|(_, sequence)| sequence.members.contains(&node_id))
    }

    fn trace_sequence_id(&self, sequence: &SequenceState) -> String {
        let members = sequence
            .members
            .iter()
            .map(|member| {
                let node = &self.graph.nodes[member.0 as usize];
                self.input.node(node.input_id).map_or_else(
                    || node.tala_id.to_string(),
                    |input| input.external_id.clone(),
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("Sequence vessel of: [{members}]")
    }

    fn trace_route_stage(&mut self, stage: &str) {
        if self.suppress_route_trace {
            return;
        }
        self.emit_normalized_trace(stage);
        if !crate::internal::trace_env_enabled("TALA_TRACE_ROUTE_STAGES") {
            return;
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_GRAPH_EDGES") {
            eprint!("GRAPH_STATE_RUST stage={stage} edges=");
            for edge in self.graph.member_edges() {
                eprint!(
                    "{}>{},",
                    self.graph.nodes[edge.from.0 as usize].tala_id,
                    self.graph.nodes[edge.to.0 as usize].tala_id
                );
            }
            eprintln!();
            eprint!("GRAPH_ORDER_EDGES_RUST stage={stage} edges=");
            for edge_id in &self.graph.edge_order.read() {
                let edge = &self.graph.edge_records[edge_id.0 as usize];
                eprint!(
                    "{}>{},",
                    self.graph.nodes[edge.from.0 as usize].tala_id,
                    self.graph.nodes[edge.to.0 as usize].tala_id
                );
            }
            eprintln!();
        }
        for (edge_index, edge) in self.graph.member_edge_entries() {
            eprint!(
                "ROUTE_STAGE_RUST stage={stage} edge={edge_index} from={} to={} points=",
                self.graph.nodes[edge.from.0 as usize].tala_id,
                self.graph.nodes[edge.to.0 as usize].tala_id,
            );
            for point in &edge.route.read() {
                eprint!("{},{};", point.x, point.y);
            }
            eprintln!();
        }
    }

    pub(super) fn new(input: &Graph, seed: i64, prearranged: bool, fixed_sizes: bool) -> Self {
        Self::new_with_rng_trace(input, seed, prearranged, fixed_sizes, true)
    }

    /// Build a graph-only pipeline helper. Structural scope wrappers and
    /// final trace snapshots do not own D2 RNG streams and must not emit
    /// constructor events that look like algorithm state.
    pub(super) fn new_untraced(
        input: &Graph,
        seed: i64,
        prearranged: bool,
        fixed_sizes: bool,
    ) -> Self {
        Self::new_with_rng_trace(input, seed, prearranged, fixed_sizes, false)
    }

    fn new_with_rng_trace(
        input: &Graph,
        seed: i64,
        prearranged: bool,
        fixed_sizes: bool,
        trace_rng_init: bool,
    ) -> Self {
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_PIPELINE_NEW") {
            eprintln!(
                "PIPELINE_NEW_CALL seed={seed} prearranged={prearranged} fixed_sizes={fixed_sizes}"
            );
        }
        #[cfg(feature = "diagnostic-traces")]
        if !prearranged
            && !fixed_sizes
            && crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT")
            && let Some(sizes) = TRACE_CLUSTER_TRANSITION_SIZES.get()
            && let Ok(mut sizes) = sizes.lock()
        {
            if crate::internal::trace_env_enabled("TALA_TRACE_PIPELINE_NEW") {
                eprintln!(
                    "PIPELINE_NEW_RESET seed={seed} prearranged={prearranged} fixed_sizes={fixed_sizes}"
                );
            }
            // A placement run has one shared transition cache across all of
            // its recursive after-fit callbacks.  Reset it at the pipeline
            // boundary, rather than at every nested `initialized` event.
            sizes.clear();
        }
        if !prearranged
            && !fixed_sizes
            && crate::internal::trace_env_enabled("TALA_TRACE_PLACEMENT")
        {
            #[cfg(feature = "diagnostic-traces")]
            if let Some(sizes) = TRACE_CLUSTER_INITIAL_SIZES.get()
                && let Ok(mut sizes) = sizes.lock()
            {
                sizes.clear();
            }
            #[cfg(feature = "diagnostic-traces")]
            if let Some(sizes) = TRACE_CLUSTER_PRE_RESIZE_SIZES.get()
                && let Ok(mut sizes) = sizes.lock()
            {
                sizes.clear();
            }
            if let Some(roots) = TRACE_SOURCE_CLUSTER_ROOTS.get()
                && let Ok(mut roots) = roots.lock()
            {
                roots.clear();
            }
            if let Some(nodes) = TRACE_SYNCED_NODES.get()
                && let Ok(mut nodes) = nodes.lock()
            {
                nodes.clear();
            }
            #[cfg(feature = "diagnostic-traces")]
            if let Some(nodes) = TRACE_OWNER_FRAME_NODES.get()
                && let Ok(mut nodes) = nodes.lock()
            {
                nodes.clear();
            }
            if let Some(orders) = TRACE_SOURCE_CHILD_ORDER.get()
                && let Ok(mut orders) = orders.lock()
            {
                orders.clear();
            }
            #[cfg(feature = "diagnostic-traces")]
            if let Some(directions) = TRACE_SOURCE_DIRECTIONS.get()
                && let Ok(mut directions) = directions.lock()
            {
                directions.clear();
            }
        }
        let mut pipeline_input = input.clone();
        let mut graph = ArenaGraph::from_input(input);
        if !prearranged && !fixed_sizes {
            // A source Graph starts with CellSize == 0 in D2 v0.9.0. The Go
            // NodePlacement stage computes the root cell size only after
            // placeNodes has recursively placed its scopes; nested split
            // graphs compute their own cell size immediately before their
            // orthogonal pass. ArenaGraph::from_input also serves direct
            // geometry helpers that expect a ready lattice, so restore the
            // pipeline's stage-accurate value here. GraphJSON inputs carry
            // their serialized value, including an explicit zero.
            graph.cell_size = input.serialized_cell_size.unwrap_or(0.0);
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_PIPELINE_NEW") {
            eprintln!(
                "PIPELINE_GRAPH_RUST seed={seed} fixed_sizes={fixed_sizes} cell={} input_serialized_cell={:?} nodes={}",
                graph.cell_size,
                input.serialized_cell_size,
                graph.nodes.len()
            );
        }
        if !prearranged {
            // Go's Layout starts with ResetClusters: graphjson snapshots may
            // carry the previous run's cluster map, but those vessels are
            // detached before Prescale and rediscovered at
            // PreprocessClusters. Keep the serialized members in the live
            // node order until that stage.
            graph.reset_serialized_clusters_for_layout();
            // ResetClusters mutates the upstream Graph.Edges endpoints before
            // recursive placement scopes are built. Keep the pipeline's
            // retained source Graph at that same boundary: those scopes read
            // its endpoint IDs as the original pointer topology while
            // projecting the newly discovered aggregate vessels.
            for edge in graph.member_edges() {
                let input_edge_index = edge.input_id.0 as usize;
                let Some(input_edge) = pipeline_input.edges.get_mut(input_edge_index) else {
                    continue;
                };
                input_edge.source = graph.nodes[edge.from.0 as usize].input_id;
                input_edge.target = graph.nodes[edge.to.0 as usize].input_id;
            }
            pipeline_input
                .source_edge_endpoints
                .clone_from(&graph.source_edge_endpoints);
            pipeline_input
                .source_edge_endpoints_by_id
                .clone_from(&graph.source_edge_endpoints_by_id);
            // A fresh full layout owns all automatic routes and route-derived
            // presentation. The Go pipeline clears these at its first stage;
            // graph snapshots can contain a prior result, so retaining those
            // points would make BinPack treat the snapshot as already routed.
            // Keep this boundary in the pipeline constructor so every fresh
            // layout entry point (production and diagnostic) observes the same
            // lifecycle, while prearranged routing continues to consume the
            // supplied routes.
            for edge_index in graph.member_edge_indices() {
                let edge = &mut graph.edge_records[edge_index];
                edge.route.replace(RoutePoints::nil());
                // D2's ResetFullLayoutRouteState clears IsCurve alongside
                // automatic route points; routeedges keeps the input flag.
                edge.is_curve = false;
                if !edge.label_position_fixed
                    && let Some(label) = edge.label.as_mut()
                {
                    label.percentage = 0.0;
                    if matches!(
                        label.position,
                        LabelPosition::UnlockedTop
                            | LabelPosition::UnlockedMiddle
                            | LabelPosition::UnlockedBottom
                            | LabelPosition::Unset
                    ) {
                        label.position = LabelPosition::Unset;
                    }
                }
            }
            graph.cost_cache.reset_placement();
            // D2's layout conversion records FixedTopLeft independently of
            // TopLeft. Placement materializes it; preprocessing observes nil.
            // Serialized TopLeft remains an initial arrangement hint and is
            // visible to preprocessing just as it is in the v0.9.0 graph
            // engine. Standalone routing still receives its already-positioned
            // graph.
            for node in &mut graph.nodes {
                if node.fixed_top_left.is_some() {
                    node.position = None;
                }
            }
        }
        // Match newPipeline's source construction order. The Go implementation
        // creates hierarchyPlacementRandom before random, and init events are
        // part of the diagnostic trace even though the streams are independent.
        let hierarchy_rng = if trace_rng_init {
            go_rng::GoRng::with_stream(seed, "hierarchy-placement")
        } else {
            go_rng::GoRng::with_stream_untraced(seed, "hierarchy-placement")
        };
        let misc_rng = if trace_rng_init {
            go_rng::GoRng::with_stream(seed, "pipeline-random")
        } else {
            go_rng::GoRng::with_stream_untraced(seed, "pipeline-random")
        };
        Self {
            input: pipeline_input,
            graph,
            sized_obstacles: Vec::new(),
            seed,
            prearranged,
            fixed_sizes,
            completed_stages: Vec::new(),
            misc_rng,
            node_placement_rng: None,
            hierarchy_rng,
            hierarchy_assignments: Vec::new(),
            hierarchy_placements: Vec::new(),
            next_rng_float: None,
            run_align_axis: true,
            run_edge_route: false,
            singleton_fast_path: false,
            suppress_route_trace: false,
            trace_root: String::new(),
            route_stage_work: routing::RouteStageWork::new(
                routing::MAX_EDGE_ROUTING_STAGE_WORK_UNITS,
            ),
            source_orientation_disabled_for_loop: false,
        }
    }

    /// Actual Equidistance engine wrapper; shared with prepared cell tests.
    pub(super) fn equidistance_stage(&mut self) {
        self.graph.compute_cell_size();
        self.run_align_axis = self.graph.equidistance();
    }

    /// Actual engine stage boundary; shared with prepared wrapper contract tests.
    pub(super) fn optimize_clusters_stage(&mut self) -> Result<(), LayoutError> {
        self.graph.compute_cell_size();
        let context = gap_guard::ActiveContext::new();
        let mut guard = limits::WorkGuard::new(
            Some(&context),
            "OptimizeClustersTransactions",
            gap_guard::MAX_TRANSACTION_WORK_UNITS,
        )
        .map_err(|error| LayoutError::PlacementTransaction(error.to_string()))?;
        self.run_align_axis = self
            .graph
            .optimize_clusters_with_context(&context, &mut guard)
            .map_err(|error| LayoutError::PlacementTransaction(error.to_string()))?;
        Ok(())
    }

    pub(super) fn run_align_axes(&mut self, sync_after_commit: bool) {
        if !self.run_align_axis {
            return;
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_STAGE") {
            eprintln!("ALIGN_STAGE_RUST_BEGIN");
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_STATE") {
            eprintln!("ALIGN_STAGE_RUST begin");
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_STATE") {
            for target in [2280770807_i64, 1997504280_i64] {
                if let Some(node) = self.graph.nodes.iter().find(|node| node.tala_id == target) {
                    eprintln!(
                        "ALIGN_STAGE_RUST before id={} pos={:?} size={:?}",
                        target, node.position, node.rect.size
                    );
                }
            }
            for cluster in &self.graph.clusters {
                let vessel_pos = self.graph.active_node_position(cluster.members[0]);
                let vessel_size = self.graph.active_node_size(cluster.members[0]);
                eprint!(
                    "ALIGN_CLUSTER_STAGE_RUST vessel={} pos={:?} size={},{} active={} members=",
                    cluster.vessel_tala_id,
                    vessel_pos,
                    vessel_size.width,
                    vessel_size.height,
                    self.graph.cluster_is_active(cluster)
                );
                for member in &cluster.members {
                    let node = &self.graph.nodes[member.0 as usize];
                    eprint!(
                        "{}@{:?}: {},{};",
                        node.tala_id, node.position, node.rect.size.width, node.rect.size.height
                    );
                }
                eprintln!();
            }
        }
        self.run_align_axis = false;
        if self.graph.cell_size == 0.0 {
            self.graph.compute_cell_size();
        }
        // Graph.getNonCenterPortCost stays lazy until a scored container pair
        // appears inside AlignAxes. A candidate can create that pair, so its
        // first maxEdgeLength sample must happen after the candidate moves.
        self.graph.reset_alignment_container_cost();
        self.graph.align_axes_with_commit_sync(sync_after_commit);
        if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_STATE") {
            eprintln!("ALIGN_STAGE_RUST end");
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_STATE") {
            for target in [2280770807_i64, 1997504280_i64] {
                if let Some(node) = self.graph.nodes.iter().find(|node| node.tala_id == target) {
                    eprintln!(
                        "ALIGN_STAGE_RUST after id={} pos={:?} size={:?}",
                        target, node.position, node.rect.size
                    );
                }
            }
            for cluster in &self.graph.clusters {
                let vessel_pos = self.graph.active_node_position(cluster.members[0]);
                let vessel_size = self.graph.active_node_size(cluster.members[0]);
                eprint!(
                    "ALIGN_CLUSTER_STAGE_RUST vessel={} pos={:?} size={},{} active={} members=",
                    cluster.vessel_tala_id,
                    vessel_pos,
                    vessel_size.width,
                    vessel_size.height,
                    self.graph.cluster_is_active(cluster)
                );
                for member in &cluster.members {
                    let node = &self.graph.nodes[member.0 as usize];
                    eprint!(
                        "{}@{:?}: {},{};",
                        node.tala_id, node.position, node.rect.size.width, node.rect.size.height
                    );
                }
                eprintln!();
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_ALIGN_STAGE") {
            eprintln!("ALIGN_STAGE_RUST_END");
        }
    }

    pub(super) fn run_prescale(&mut self) {
        self.graph.prescale(self.fixed_sizes);
        self.completed_stages.push(Stage::Prescale);
    }

    pub(super) fn run_preprocess_clusters(&mut self) {
        // The v0.9.0 AddClusters stage always starts by clearing the
        // decoded cluster map and discovering groups again from the active
        // graph.  GraphJSON snapshots can contain the vessels from an
        // earlier run, but those records are input history rather than a
        // directive to preserve arrangement or vessel identity.  Re-running
        // discovery also keeps the random stream aligned: the release draws
        // a fresh vessel ID for every accepted group at this boundary.
        //
        // AssignArrangement in the release uses dimensions and its own RNG;
        // it does not inspect serialized member coordinates.  `prearranged`
        // controls route/position handling elsewhere in the pipeline and is
        // therefore intentionally not passed as a position hint here.
        self.graph
            .assign_clusters_with_rng(self.seed, false, self.fixed_sizes, &mut self.misc_rng);
        if crate::internal::trace_env_enabled("TALA_TRACE_GRAPH_ORDER") {
            eprintln!(
                "GRAPH_ORDER_RUST stage=PreprocessClusters nodes={:?}",
                self.graph
                    .node_order
                    .iter()
                    .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        self.completed_stages.push(Stage::PreprocessClusters);
    }

    pub(super) fn run_preprocess_sequences(&mut self) {
        if !self.graph.sequence_backlinks_complete || self.graph.sequences.is_empty() {
            self.graph.assign_sequences(&mut self.misc_rng);
        }
        // AddSequences disconnects the defining step edges and abducts every
        // external edge onto the new vessel. Recompute connectivity-derived
        // placement components at the same stage boundary; retaining the
        // pre-sequence cache leaves a sequence in the component of its retired
        // member graph rather than the recovered vessel graph.
        self.graph.refresh_placement_components();
        if crate::internal::trace_env_enabled("TALA_TRACE_GRAPH_ORDER") {
            eprintln!(
                "GRAPH_ORDER_RUST stage=PreprocessSequences nodes={:?}",
                self.graph
                    .node_order
                    .iter()
                    .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        self.completed_stages.push(Stage::PreprocessSequences);
    }

    /// Recovered `Pipeline.PreprocessStage`.
    ///
    /// TALA materializes each node's `LoopOffsets` map during preprocessing.
    /// Keep the same phase ownership: candidate overlap checks consume the
    /// cached values rather than rescanning incident edges for every point.
    pub(super) fn run_preprocess(&mut self) {
        // placement.Prepare (Go stages.go:110-114) runs labeling.Initialize
        // and Graph.ComputeNodeSpacing over graph.Nodes only. AddSequences
        // already removed every sequence step from graph.Nodes
        // (grouping/sequences.go addSequence), so a step's explicit label or
        // icon position is never FixPosition'ed (labeling/model.go:84-101)
        // and Node.UpdateSpacing (layoutgraph/node.go:927) never reserves its
        // label, icon or modifier spacing. The arena reserved those for every
        // node at construction; retract them for steps.
        for node in &mut self.graph.nodes {
            if node.sequence.is_some() {
                node.label_position_fixed = false;
                node.icon_position_fixed = false;
                node.layout_margins = crate::Insets::uniform(0.0);
                node.node_padding = crate::Insets::uniform(0.0);
            }
        }
        // D2 v0.9.0's placement.Prepare initializes automatic node-label
        // positions after Prescale and sequence preprocessing. Keep unset
        // labels unset through those earlier trace boundaries, then assign
        // the first shape preference before later fitting and placement.
        for node in &mut self.graph.nodes {
            if node.label_size.is_some() && node.label_position == crate::LabelPosition::Unset {
                node.label_position =
                    labels::default_node_label_position(node.shape, node.is_container);
            }
        }
        for index in 0..self.graph.nodes.len() {
            let node = self.graph.nodes[index].input_id;
            self.graph.update_loop_offsets(node);
            if crate::internal::trace_env_enabled("TALA_TRACE_LOOPS")
                && self.graph.nodes[index].edges.read().iter().any(|edge| {
                    let edge = &self.graph.edge_records[edge.0 as usize];
                    edge.from == node && edge.to == node
                })
            {
                eprintln!(
                    "LOOP_OFFSETS_RUST id={} shape={:?} size={:?} offsets={:?}",
                    self.graph.nodes[index].tala_id,
                    self.graph.nodes[index].shape,
                    self.graph.nodes[index].rect.size,
                    self.graph.nodes[index].loop_offsets
                );
            }
        }
        self.completed_stages.push(Stage::Preprocess);
    }

    pub(super) fn run_preprocess_trees(&mut self) {
        self.graph.preprocess_trees();
        if crate::internal::trace_env_enabled("TALA_TRACE_GRAPH_ORDER") {
            eprintln!(
                "GRAPH_ORDER_RUST stage=PreprocessTrees nodes={:?}",
                self.graph
                    .node_order
                    .iter()
                    .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        self.completed_stages.push(Stage::PreprocessTrees);
    }

    /// `AddContainers` is materialized by `ArenaGraph::from_input`: stable
    /// Rust IDs replace TALA's pointer-preserving remove/add transaction.
    pub(super) fn run_preprocess_containers(&mut self) {
        if crate::internal::trace_env_enabled("TALA_TRACE_GRAPH_ORDER") {
            eprintln!(
                "GRAPH_ORDER_RUST stage=PreprocessContainers nodes={:?}",
                self.graph
                    .node_order
                    .iter()
                    .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        self.completed_stages.push(Stage::PreprocessContainers);
    }

    pub(super) fn run_preprocess_hierarchies(&mut self) {
        // D2 v0.9.0 hierarchy.Assign has no RNG parameter; only hierarchy.Place
        // receives the pipeline's hierarchy-placement stream.
        self.graph.assign_forced_hierarchies();
        self.hierarchy_assignments = self.graph.assign_automatic_hierarchies_traced();
        let hierarchy_ids = self
            .graph
            .nodes
            .iter()
            .filter_map(|node| node.hierarchy.clone().map(|membership| membership.id))
            .collect::<BTreeSet<_>>();
        // Assign publishes the automatic and forced memberships before the
        // source Place traversal consumes the shared hierarchy-placement RNG.
        for hierarchy_id in hierarchy_ids {
            if let Some(trace) = self
                .graph
                .apply_hierarchy_placement_traced(hierarchy_id, &mut self.hierarchy_rng)
            {
                self.hierarchy_placements.push(trace);
            }
        }
        self.graph.remove_isolated_hierarchy_memberships();
        self.graph.remove_hierarchy_nears();
        if crate::internal::trace_env_enabled("TALA_TRACE_GRAPH_ORDER") {
            eprintln!(
                "GRAPH_ORDER_RUST stage=PreprocessHierarchies nodes={:?}",
                self.graph
                    .node_order
                    .iter()
                    .map(|node| self.graph.nodes[node.0 as usize].tala_id)
                    .collect::<Vec<_>>()
            );
        }
        self.completed_stages.push(Stage::PreprocessHierarchies);
    }

    pub(super) fn run_preprocess_hubs(&mut self) {
        self.graph.compute_hubs();
        let mut connected_child_order = self.graph.connected_container_child_order.clone();
        for node in self.graph.nodes.iter().filter(|node| node.is_container) {
            let parent_tala_id = self.graph.active_node_tala_id(node.input_id);
            let active_children = self
                .graph
                .container_node_order(Some(node.input_id))
                .into_iter()
                .map(|child| {
                    self.graph
                        .active_node_tala_id(self.graph.active_aggregate_owner(child))
                })
                .collect::<Vec<_>>();
            let source_children = connected_child_order
                .get(&parent_tala_id)
                .or_else(|| connected_child_order.get(&node.tala_id));
            // Before NodePlacement restores extracted tree children, the
            // active placement view can omit those pointers. Preserve their
            // identities in this temporary cache; after recursive placement,
            // refresh from the restored owner slice to capture its actual
            // order. Otherwise keep the fully preprocessed order, such as a
            // root containing discovered sequences.
            let ordered_children = source_children
                .filter(|source| source.len() != active_children.len())
                .cloned()
                .unwrap_or(active_children);
            connected_child_order.insert(parent_tala_id, ordered_children);
        }
        self.graph.connected_container_child_order = connected_child_order;
        #[cfg(feature = "diagnostic-traces")]
        if let Some(target) = crate::internal::trace_env_value("D2RS_TRACE_CONNECTED_ORDER") {
            for node in self.graph.nodes.iter().filter(|node| node.is_container) {
                let parent_tala_id = self.graph.active_node_tala_id(node.input_id);
                if target != "all" && target != parent_tala_id.to_string() {
                    continue;
                }
                let active_children = self
                    .graph
                    .container_node_order(Some(node.input_id))
                    .into_iter()
                    .map(|child| {
                        self.graph
                            .active_node_tala_id(self.graph.active_aggregate_owner(child))
                    })
                    .collect::<Vec<_>>();
                eprintln!(
                    "PIPELINE_CONNECTED_ORDER_RUST stage=PreprocessHubs parent={} active={:?} connected={:?} source={:?}",
                    parent_tala_id,
                    active_children,
                    self.graph
                        .connected_container_child_order
                        .get(&parent_tala_id),
                    self.graph.source_container_child_order.get(&parent_tala_id),
                );
            }
        }
        self.completed_stages.push(Stage::PreprocessHubs);
    }

    /// Refresh pointer-visible connected traversal order after recursive node
    /// placement has restored extracted tree children to their owning
    /// Containers slices. The preprocessing snapshot can precede that
    /// restoration and therefore no longer describe the owner graph seen by
    /// later movement and gap passes.
    pub(super) fn refresh_connected_container_child_order_from_live_slices(&mut self) {
        let mut connected_child_order = self.graph.connected_container_child_order.clone();
        for node in self.graph.nodes.iter().filter(|node| node.is_container) {
            let Some(children) = self.graph.containers.get(&Some(node.input_id)) else {
                continue;
            };
            let parent_tala_id = self.graph.active_node_tala_id(node.input_id);
            let ordered_children = children
                .iter()
                .map(|child| {
                    self.graph
                        .active_node_tala_id(self.graph.active_aggregate_owner(*child))
                })
                .collect();
            connected_child_order.insert(parent_tala_id, ordered_children);
        }
        self.graph.connected_container_child_order = connected_child_order;
    }

    pub(super) fn run_initialize_nodes(&mut self) -> Result<(), crate::LayoutError> {
        self.graph.initialize_nodes(self.seed)
    }

    pub(super) fn run_sizeless_optimize(&mut self, temperature: f64) {
        let mut optimizer = if let Some(rng) = self.node_placement_rng.take() {
            sizeless::SizelessOptimizer::with_trace_rng(
                &mut self.graph,
                rng,
                Some(&self.trace_root),
            )
        } else {
            sizeless::SizelessOptimizer::with_trace_root(
                &mut self.graph,
                self.seed,
                Some(&self.trace_root),
            )
        };
        optimizer.optimize(temperature);
        self.next_rng_float = Some(optimizer.next_float_probe());
    }

    pub(super) fn run_sizeless_anneal_state(&mut self) -> (go_rng::GoRng, bool) {
        let node_count = self.graph.largest_optimizable_component_size();
        #[cfg(feature = "diagnostic-traces")]
        let mut trace_observer = None;
        self.run_sizeless_anneal_state_for_node_count(
            node_count,
            #[cfg(feature = "diagnostic-traces")]
            &mut trace_observer,
        )
    }

    pub(super) fn run_sizeless_anneal_state_for_node_count(
        &mut self,
        node_count: usize,
        #[cfg(feature = "diagnostic-traces")] trace_observer: &mut Option<
            &mut dyn FnMut(&str, &ArenaGraph),
        >,
    ) -> (go_rng::GoRng, bool) {
        let (rng, horizontal, _) = self
            .run_sizeless_anneal_state_and_temperature_for_node_count(
                node_count,
                #[cfg(feature = "diagnostic-traces")]
                trace_observer,
                None,
            )
            .expect("dense sizeless annealing has no owner binding errors");
        (rng, horizontal)
    }

    fn run_sizeless_anneal_state_and_temperature_for_node_count(
        &mut self,
        node_count: usize,
        #[cfg(feature = "diagnostic-traces")] trace_observer: &mut Option<
            &mut dyn FnMut(&str, &ArenaGraph),
        >,
        mut owner_session: Option<&mut OwnerScoreSession<'_>>,
    ) -> Result<(go_rng::GoRng, bool, f64), crate::LayoutError> {
        let mut optimizer = if let Some(rng) = self.node_placement_rng.take() {
            sizeless::SizelessOptimizer::with_trace_rng(
                &mut self.graph,
                rng,
                Some(&self.trace_root),
            )
        } else {
            sizeless::SizelessOptimizer::with_trace_root(
                &mut self.graph,
                self.seed,
                Some(&self.trace_root),
            )
        };
        let iterations = (90.0 * (node_count as f64).sqrt()) as usize;
        let mut temperature = 2.0 * (node_count as f64).sqrt();
        if iterations == 0 {
            #[cfg(feature = "diagnostic-traces")]
            if let Some(observer) = trace_observer.as_deref_mut() {
                observer("orthogonal-after-sizeless", optimizer.graph());
            }
            return Ok((optimizer.into_rng(), true, temperature));
        }
        // Go's math.Pow positive, finite, non-integral path is
        // Exp(y*Log(x)). Rust's libm `powf` rounds this case one ULP away
        // from the recovered Go result, and that can change a strict
        // annealing acceptance later in the pass.
        let cooling_factor = (1.0 / iterations as f64 * (0.2 / temperature).ln()).exp();
        if crate::internal::trace_env_enabled("TALA_TRACE_COOLING") {
            eprintln!(
                "COOLING_RUST sizeless node_count={} iterations={} initial_temp={:.17e} factor={:.17e} factor_bits={:016x}",
                node_count,
                iterations,
                temperature,
                cooling_factor,
                cooling_factor.to_bits(),
            );
        }
        let mut horizontal = true;
        for iteration in 0..iterations / 2 {
            if let Some(session) = owner_session.as_deref_mut() {
                if let Some(cause) = crate::internal::cancellation_error() {
                    return Err(crate::LayoutError::PlacementTransaction(format!(
                        "PlaceNodes: {cause}"
                    )));
                }
                optimizer
                    .optimize_with_owner_session(temperature, session, &SizelessLayoutContext)
                    .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?;
            } else {
                optimizer.optimize(temperature);
            }
            if iteration % 9 == 0 {
                if crate::internal::trace_env_enabled("TALA_TRACE_SIZELESS_COMPACTION") {
                    crate::internal::pipeline::trace_placement_state(
                        &format!("sizeless-before-compact-{iteration}"),
                        &self.trace_root,
                        optimizer.graph(),
                    );
                }
                if let Some(session) = owner_session.as_deref_mut() {
                    optimizer
                        .compact_with_owner_session(horizontal, 3.0, session)
                        .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?;
                } else {
                    optimizer
                        .compact(horizontal, 3.0)
                        .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?;
                }
                if crate::internal::trace_env_enabled("TALA_TRACE_SIZELESS_COMPACTION") {
                    crate::internal::pipeline::trace_placement_state(
                        &format!("sizeless-compact-{iteration}"),
                        &self.trace_root,
                        optimizer.graph(),
                    );
                }
                horizontal = !horizontal;
            }
            trace_optimizer_state(format_args!("sizeless-{iteration}"), optimizer.graph());
            temperature *= cooling_factor;
        }
        #[cfg(feature = "diagnostic-traces")]
        if let Some(observer) = trace_observer.as_deref_mut() {
            observer("orthogonal-after-sizeless", optimizer.graph());
        }
        self.next_rng_float = Some(optimizer.next_float_probe());
        if let Some(session) = owner_session {
            optimizer.release_owner_geometry(session);
        }
        Ok((optimizer.into_rng(), horizontal, temperature))
    }

    pub(super) fn run_sizeless_anneal_with_rng(&mut self) -> go_rng::GoRng {
        self.run_sizeless_anneal_state().0
    }

    pub(super) fn run_sizeless_anneal(&mut self) {
        let _ = self.run_sizeless_anneal_with_rng();
    }

    pub(super) fn run_sized_pass(
        &mut self,
        pass_count: usize,
        first_node_only: bool,
        compact_during_passes: bool,
        zero_optimize: bool,
        edge_abduction_nodes: Option<BTreeSet<NodeId>>,
    ) {
        let node_count = self.graph.largest_optimizable_component_size();
        self.run_sized_pass_for_node_count(
            node_count,
            pass_count,
            first_node_only,
            compact_during_passes,
            zero_optimize,
            edge_abduction_nodes,
            #[cfg(feature = "diagnostic-traces")]
            None,
            None,
        )
        .expect("dense sized pass has no owner binding errors");
    }

    pub(super) fn run_split_subgraph_sized_pass(
        &mut self,
        pass_count: usize,
        edge_abduction_nodes: Option<BTreeSet<NodeId>>,
        #[cfg(feature = "diagnostic-traces")] trace_observer: Option<
            &mut dyn FnMut(&str, &ArenaGraph),
        >,
    ) {
        self.run_split_subgraph_sized_pass_with_owner(
            pass_count,
            edge_abduction_nodes,
            #[cfg(feature = "diagnostic-traces")]
            trace_observer,
            None,
        )
        .expect("dense split pass has no owner binding errors");
    }

    pub(super) fn run_split_subgraph_sized_pass_with_owner(
        &mut self,
        pass_count: usize,
        edge_abduction_nodes: Option<BTreeSet<NodeId>>,
        #[cfg(feature = "diagnostic-traces")] trace_observer: Option<
            &mut dyn FnMut(&str, &ArenaGraph),
        >,
        owner_session: Option<&mut OwnerScoreSession<'_>>,
    ) -> Result<(), crate::LayoutError> {
        // Match Graph.placeNodes' singleton fast path. A one-node graph is
        // positioned directly and returns before InitializeNodes' optimizer,
        // so it has no placement snapshots or RNG-consuming passes.
        if self.singleton_fast_path && self.graph.nodes.len() == 1 {
            let node = &mut self.graph.nodes[0];
            // The Go singleton branch unconditionally assigns TopLeft: it
            // does not preserve a serialized arrangement hint.  This matters
            // for a one-node graph whose InitializeNodes projection has
            // already materialized the default inset at (1,1).
            node.position = Some(node.fixed_top_left.unwrap_or_default());
            node.rect.origin = node.position.unwrap_or_default();
            // The owning Graph.Containers map still holds the same shared
            // pointer when this singleton is a child of a larger scope. Keep
            // every projected alias in that map synchronized with the direct
            // node assignment before the enclosing scope packs its components.
            let tala_id = node.tala_id;
            let position = node.position;
            let origin = node.rect.origin;
            let size = node.rect.size;
            for external in &mut self.graph.transaction_external_containers {
                if external.tala_id == tala_id {
                    external.position = position;
                    external.rect.origin = origin;
                    external.rect.size = size;
                }
            }
            for children in self
                .graph
                .transaction_external_container_children
                .values_mut()
            {
                for external in children {
                    if external.tala_id == tala_id {
                        external.position = position;
                        external.rect.origin = origin;
                        external.rect.size = size;
                    }
                }
            }
            self.graph.sync_nested_projected_container_children();
            return Ok(());
        }
        // Recovered placeNodesOrthogonally derives both annealing halves from
        // len(subgraph.Nodes), including fixed and singleton nodes.
        self.run_sized_pass_for_node_count(
            self.graph.nodes.len(),
            pass_count,
            false,
            true,
            true,
            edge_abduction_nodes,
            #[cfg(feature = "diagnostic-traces")]
            trace_observer,
            owner_session,
        )
    }

    fn run_sized_pass_for_node_count(
        &mut self,
        node_count: usize,
        pass_count: usize,
        first_node_only: bool,
        compact_during_passes: bool,
        zero_optimize: bool,
        edge_abduction_nodes: Option<BTreeSet<NodeId>>,
        #[cfg(feature = "diagnostic-traces")] mut trace_observer: Option<
            &mut dyn FnMut(&str, &ArenaGraph),
        >,
        mut owner_session: Option<&mut OwnerScoreSession<'_>>,
    ) -> Result<(), crate::LayoutError> {
        trace_optimizer_state(format_args!("initialize"), &self.graph);
        if crate::internal::trace_env_enabled("TALA_TRACE_FIXED_NODES") {
            eprint!("FIXED_NODES_RUST cell={}", self.graph.cell_size);
            for node in &self.graph.nodes {
                eprint!(
                    " {}=fixed:{} pos={:?}",
                    node.tala_id,
                    node.fixed_top_left.is_some(),
                    node.position
                );
            }
            eprintln!();
        }
        let iterations = (90.0 * (node_count as f64).sqrt()) as usize;
        let initial_temperature = 2.0 * (node_count as f64).sqrt();
        if iterations == 0 {
            #[cfg(feature = "diagnostic-traces")]
            let mut observer = trace_observer;
            let (rng, _) = self.run_sizeless_anneal_state_for_node_count(
                node_count,
                #[cfg(feature = "diagnostic-traces")]
                &mut observer,
            );
            self.graph.anchor_fixed_positions(true);
            self.next_rng_float = Some(rng.clone_without_trace().float64());
            return Ok(());
        }
        let cooling_factor = (1.0 / iterations as f64 * (0.2 / initial_temperature).ln()).exp();
        if crate::internal::trace_env_enabled("TALA_TRACE_COOLING") {
            eprintln!(
                "COOLING_RUST sized node_count={} iterations={} initial_temp={:.17e} factor={:.17e} factor_bits={:016x}",
                node_count,
                iterations,
                initial_temperature,
                cooling_factor,
                cooling_factor.to_bits(),
            );
        }
        let mut temperature;
        // As in the recovered Go loop, sized compaction continues with the
        // orientation toggle left by the sizeless compaction phase.
        let (rng, mut horizontal_compaction, carried_temperature) = self
            .run_sizeless_anneal_state_and_temperature_for_node_count(
                node_count,
                #[cfg(feature = "diagnostic-traces")]
                &mut trace_observer,
                owner_session.as_deref_mut(),
            )?;
        trace_optimizer_state(format_args!("after-sizeless"), &self.graph);
        temperature = carried_temperature;
        self.graph.transition_compact();
        #[cfg(feature = "diagnostic-traces")]
        if let Some(observer) = trace_observer.as_deref_mut() {
            observer("orthogonal-after-transition-compaction", &self.graph);
        }
        trace_optimizer_state(format_args!("transition"), &self.graph);
        self.graph.anchor_fixed_positions(true);
        self.graph.snap_nonfixed_to_cells();
        trace_placement_state("sized-transition", &self.trace_root, &self.graph);
        self.graph.initialize_turn_cost();
        trace_optimizer_state(format_args!("constructor-before"), &self.graph);
        let mut optimizer = sized::SizedOptimizer::new(&mut self.graph, rng, edge_abduction_nodes)
            .with_obstacles(self.sized_obstacles.clone());
        if let Some(session) = owner_session.as_deref_mut() {
            optimizer
                .initialize_owner_setup(session, &SizelessLayoutContext)
                .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?;
        }
        trace_optimizer_state(format_args!("constructor-after"), optimizer.graph());
        // The recovered placement calls SyncHerdFences immediately after
        // constructing NewSizedOptimizer and only then reaches its pre-sized
        // trace boundary. Keep that ordering: the fence publication moves
        // shared container geometry and is observable by the first sized pass.
        optimizer
            .sync_herd_fences_with_owner(owner_session.as_deref_mut())
            .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        trace_optimizer_state(format_args!("constructor-sync"), optimizer.graph());
        trace_optimizer_state(format_args!("pre-sized"), optimizer.graph());
        if first_node_only {
            if let Some(session) = owner_session.as_deref_mut() {
                optimizer
                    .optimize_with_owner_nears(temperature, true, session, &SizelessLayoutContext)
                    .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            } else {
                optimizer.optimize_one(temperature);
            }
        } else {
            let trace_sized_pass_target = crate::internal::trace_env_value("TALA_TRACE_SIZED_PASS")
                .and_then(|target| target.parse::<usize>().ok());
            for pass_index in 0..pass_count {
                let iteration = iterations / 2 + 1 + pass_index;
                let trace_sized_pass = trace_sized_pass_target == Some(iteration);
                if trace_sized_pass {
                    eprintln!("SIZED_PASS_RUST begin={iteration}");
                }
                trace_optimizer_state(format_args!("sized-before-{iteration}"), optimizer.graph());
                if trace_sized_pass {
                    // Diagnostic-only process-local gate; the plugin runs this
                    // placement path single-threaded during trace capture.
                    crate::internal::set_sized_pass_trace_active(true);
                }
                if let Some(session) = owner_session.as_deref_mut() {
                    optimizer
                        .optimize_with_owner_nears(
                            temperature,
                            false,
                            session,
                            &SizelessLayoutContext,
                        )
                        .map_err(|error| {
                            crate::LayoutError::PlacementTransaction(error.to_string())
                        })?;
                } else {
                    optimizer.optimize(temperature);
                }
                trace_sized_boundary("after-optimize", optimizer.graph());
                if trace_sized_pass {
                    crate::internal::set_sized_pass_trace_active(false);
                }
                if compact_during_passes && iteration.is_multiple_of(9) {
                    let factor = (1.0
                        + (2.0 * (iterations as f64 - iteration as f64 - 30.0))
                            / (0.5 * iterations as f64))
                        .max(1.0);
                    if let Some(session) = owner_session.as_deref_mut() {
                        optimizer
                            .compact_with_owner_nears(horizontal_compaction, factor, session)
                            .map_err(|error| {
                                crate::LayoutError::PlacementTransaction(error.to_string())
                            })?;
                    } else {
                        optimizer
                            .compact(horizontal_compaction, factor)
                            .map_err(|e| crate::LayoutError::PlacementTransaction(e.to_string()))?;
                    }
                    optimizer
                        .join_distanced_clusters_with_owner(owner_session.as_deref_mut())
                        .map_err(|error| {
                            crate::LayoutError::PlacementTransaction(error.to_string())
                        })?;
                    if trace_sized_pass {
                        trace_sized_boundary("after-cluster-join", optimizer.graph());
                    }
                    optimizer
                        .sync_herd_fences_with_owner(owner_session.as_deref_mut())
                        .map_err(|error| {
                            crate::LayoutError::PlacementTransaction(error.to_string())
                        })?;
                    if trace_sized_pass {
                        trace_sized_boundary("after-herd-sync", optimizer.graph());
                    }
                    horizontal_compaction = !horizontal_compaction;
                    trace_sized_boundary("after-compaction", optimizer.graph());
                }
                trace_sized_boundary("before-state-trace", optimizer.graph());
                trace_optimizer_state(
                    format_args!("sized-{}", iterations / 2 + 1 + pass_index),
                    optimizer.graph(),
                );
                let placement_phase = format!("sized-{iteration}");
                trace_placement_state(&placement_phase, &self.trace_root, optimizer.graph());
                if trace_sized_pass {
                    eprintln!("SIZED_PASS_RUST after={iteration}");
                }
                temperature *= cooling_factor;
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if let Some(observer) = trace_observer.as_deref_mut() {
            observer("orthogonal-after-sized", optimizer.graph());
        }
        if zero_optimize {
            optimizer
                .join_distanced_clusters_with_owner(owner_session.as_deref_mut())
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            optimizer
                .sync_herd_fences_with_owner(owner_session.as_deref_mut())
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
            for zero_pass in 0..10 {
                let changed = if let Some(session) = owner_session.as_deref_mut() {
                    optimizer
                        .optimize_with_owner_nears(0.0, false, session, &SizelessLayoutContext)
                        .map_err(|error| {
                            crate::LayoutError::PlacementTransaction(error.to_string())
                        })?
                } else {
                    optimizer.optimize(0.0)
                };
                if crate::internal::trace_env_enabled("TALA_TRACE_ZERO")
                    && optimizer.graph().nodes.len() == 4
                {
                    eprintln!("ZERO_RUST pass={} changed={}", zero_pass, changed);
                }
                trace_optimizer_state(format_args!("zero"), optimizer.graph());
                if !changed {
                    break;
                }
            }
        }
        let rng = optimizer.into_rng();
        self.graph.anchor_fixed_positions(false);
        // SplitSubgraphs shares the actual Go nodes with the owning graph.
        // Sized placement uses a detached workspace here; commit its final
        // moves before a subsequent component reads those shared positions.
        // The owner move carries descendants and preserves Point identity.
        if let Some(session) = owner_session {
            session
                .update_current_member_positions(&self.graph)
                .map_err(|error| crate::LayoutError::PlacementTransaction(error.to_string()))?;
        }
        trace_placement_state("optimized", &self.trace_root, &self.graph);
        #[cfg(feature = "diagnostic-traces")]
        if let Some(observer) = trace_observer {
            observer("orthogonal-after-optimized", &self.graph);
        }
        self.next_rng_float = Some(rng.clone_without_trace().float64());
        Ok(())
    }

    /// Route the placed arena, preserving preprocessing state across the stage
    /// boundary as TALA's graph does.
    fn route_edges_impl(&mut self) {
        // EdgeRouting creates fresh split Graph owners; the coordinator warms
        // each owner after its route geometry is prepared.
        self.graph.refresh_node_order_membership();
        let routed_edges = routing::route_edges(&self.graph);
        for (edge_index, route) in routed_edges.into_iter().enumerate() {
            // ARM64 provenance: recovered routeLoops in dwarf_loop_router.go
            // unconditionally assigns this position while routing a labelled
            // self-loop.
            if self.graph.edge_records[edge_index].from == self.graph.edge_records[edge_index].to
                && let Some(label) = self.graph.edge_records[edge_index].label.as_mut()
            {
                label.position = LabelPosition::OutsideTopCenter;
            }
            self.graph.edge_records[edge_index]
                .route
                .replace(RoutePoints::literal(&(route)));
        }
    }

    /// Checked production edge-routing boundary. The compatibility wrapper
    /// above remains useful for source-shaped unit tests that exercise a
    /// route stage without an installed cancellation context.
    fn route_edges_impl_checked(&mut self) -> Result<(), crate::LayoutError> {
        self.graph.refresh_node_order_membership();
        let routed_edges = routing::route_edges_with_cancellation_and_aggregate(
            &self.graph,
            self.seed,
            self.route_stage_work.clone(),
        )?;
        for (edge_index, route) in routed_edges.into_iter().enumerate() {
            let Some(route) = route else {
                continue;
            };
            if self.graph.edge_records[edge_index].from == self.graph.edge_records[edge_index].to
                && let Some(label) = self.graph.edge_records[edge_index].label.as_mut()
            {
                label.position = LabelPosition::OutsideTopCenter;
            }
            self.graph.edge_records[edge_index].route.replace(route);
        }
        // The pass's OVG and search state are dropped; give the embedder a
        // chance to return that memory before the next seed starts routing.
        super::run_routing_release_hook();
        Ok(())
    }

    pub(super) fn run_edge_routing(&mut self) {
        self.route_edges_impl();
        self.trace_route_stage("EdgeRouting");
    }

    pub(super) fn run_edge_routing_checked(&mut self) -> Result<(), crate::LayoutError> {
        self.route_edges_impl_checked()?;
        self.trace_route_stage("EdgeRouting");
        Ok(())
    }

    pub(super) fn run_crosshatch(&mut self) -> Result<(), crate::LayoutError> {
        self.route_stage_work = routing::RouteStageWork::new(routing::MAX_ROUTE_STAGE_WORK_UNITS);
        routing::crosshatch_with_budget(&mut self.graph, &self.route_stage_work)
            .map_err(|error| error.into_layout_error(self.seed))?;
        self.trace_route_stage("Crosshatch");
        Ok(())
    }

    pub(super) fn run_dejitter(&mut self) {
        self.run_edge_route = routing::dejitter(&mut self.graph);
        self.trace_route_stage("Dejitter");
    }

    pub(super) fn run_second_edge_routing(&mut self) -> Result<(), crate::LayoutError> {
        // Go creates a fresh RouteGraph aggregate for the second routing pass;
        // the first pass has its own envelope and does not share work with
        // post-processing after this boundary.
        self.route_stage_work =
            routing::RouteStageWork::new(routing::MAX_EDGE_ROUTING_STAGE_WORK_UNITS);
        if self.run_edge_route
            || self
                .graph
                .edge_records
                .first()
                .is_some_and(|edge| edge.route.is_empty())
        {
            // Keep the second pipeline stage as one trace event. Calling the
            // public wrapper here would emit an inner EdgeRouting event and
            // shift every following stage, while Go's stage runner reports
            // only the outer invocation.
            self.route_edges_impl_checked()?;
        }
        // The Go pipeline emits the same EdgeRouting event name for its
        // second routing pass; the pass identity is already established by
        // event order and the preceding stage snapshot.
        self.trace_route_stage("EdgeRouting");
        Ok(())
    }

    pub(super) fn run_simplify_edge_routes(&mut self) {
        routing::simplify_edge_routes(&mut self.graph);
        self.trace_route_stage("SimplifyEdgeRoutes");
    }

    pub(super) fn run_simplify_edge_routes_checked(&mut self) -> Result<(), crate::LayoutError> {
        // Each v0.9.0 post-processing stage owns a fresh 50M envelope; only
        // the two EdgeRouting passes use the larger OVG+flavor aggregate.
        self.route_stage_work = routing::RouteStageWork::new(routing::MAX_ROUTE_STAGE_WORK_UNITS);
        routing::simplify_edge_routes_with_budget(&mut self.graph, &self.route_stage_work)
            .map_err(|error| error.into_layout_error(self.seed))?;
        self.trace_route_stage("SimplifyEdgeRoutes");
        Ok(())
    }

    pub(super) fn run_swap_edge_ports(&mut self) {
        routing::swap_edge_ports(&mut self.graph);
        self.trace_route_stage("SwapEdgePorts");
    }

    pub(super) fn run_swap_edge_ports_checked(&mut self) -> Result<(), crate::LayoutError> {
        self.route_stage_work = routing::RouteStageWork::new(routing::MAX_ROUTE_STAGE_WORK_UNITS);
        routing::swap_edge_ports_with_budget(&mut self.graph, &self.route_stage_work)
            .map_err(|error| error.into_layout_error(self.seed))?;
        self.trace_route_stage("SwapEdgePorts");
        Ok(())
    }

    pub(super) fn run_straight_edges_fallback(&mut self) {
        routing::straight_edges_fallback(&mut self.graph);
        self.trace_route_stage("StraightEdgesFallback");
    }

    pub(super) fn run_straight_edges_fallback_checked(&mut self) -> Result<(), crate::LayoutError> {
        self.route_stage_work = routing::RouteStageWork::new(routing::MAX_ROUTE_STAGE_WORK_UNITS);
        routing::straight_edges_fallback_with_budget(&mut self.graph, &self.route_stage_work)
            .map_err(|_| {
                if crate::internal::cancellation_requested() {
                    crate::internal::cancellation_layout_error()
                        .unwrap_or(crate::LayoutError::Cancelled)
                } else {
                    crate::LayoutError::WorkLimit {
                        seed: self.seed,
                        location: "StraightEdgesFallback",
                        limit: routing::MAX_ROUTE_STAGE_WORK_UNITS,
                    }
                }
            })?;
        self.trace_route_stage("StraightEdgesFallback");
        Ok(())
    }

    pub(super) fn run_balance_edge_segments(&mut self) {
        routing::balance_edge_segments(&mut self.graph);
        self.trace_route_stage("BalanceEdgeSegments");
    }

    pub(super) fn run_balance_edge_segments_checked(&mut self) -> Result<(), crate::LayoutError> {
        self.route_stage_work = routing::RouteStageWork::new(routing::MAX_ROUTE_STAGE_WORK_UNITS);
        routing::balance_edge_segments_with_budget(&mut self.graph, &self.route_stage_work)
            .map_err(|error| error.into_layout_error(self.seed))?;
        self.trace_route_stage("BalanceEdgeSegments");
        Ok(())
    }

    pub(super) fn run_fix_cluster_edge_branching(&mut self) {
        routing::fix_cluster_edge_branching(&mut self.graph);
        self.trace_route_stage("FixClusterEdgeBranching");
    }

    pub(super) fn run_fix_cluster_edge_branching_checked(
        &mut self,
    ) -> Result<(), crate::LayoutError> {
        self.route_stage_work = routing::RouteStageWork::new(routing::MAX_ROUTE_STAGE_WORK_UNITS);
        routing::fix_cluster_edge_branching_with_budget(&mut self.graph, &self.route_stage_work)
            .map_err(|error| error.into_layout_error(self.seed))?;
        self.trace_route_stage("FixClusterEdgeBranching");
        Ok(())
    }

    pub(super) fn run_trace_edges_to_shape_border(&mut self) {
        routing::trace_edges_to_shape_border(&mut self.graph);
        self.trace_route_stage("TraceEdgesToShapeBorder");
    }

    pub(super) fn run_trace_edges_to_shape_border_checked(
        &mut self,
    ) -> Result<(), crate::LayoutError> {
        self.route_stage_work = routing::RouteStageWork::new(routing::MAX_ROUTE_STAGE_WORK_UNITS);
        routing::trace_edges_to_shape_border_with_budget(&mut self.graph, &self.route_stage_work)
            .map_err(|error| error.into_layout_error(self.seed))?;
        self.trace_route_stage("TraceEdgesToShapeBorder");
        Ok(())
    }

    pub(super) fn run_reorder_duplicates(&mut self) {
        labels::reorder_duplicates(&mut self.graph);
    }

    pub(super) fn run_reorder_duplicates_checked(&mut self) -> Result<(), crate::LayoutError> {
        labels::reorder_duplicates_checked(&mut self.graph).map_err(|error| match error {
            super::limits::Error::Context {
                cause: super::limits::ContextError::Cancelled,
                ..
            } => crate::LayoutError::Cancelled,
            super::limits::Error::Context {
                cause: super::limits::ContextError::DeadlineExceeded,
                ..
            } => crate::LayoutError::DeadlineExceeded,
            error => crate::LayoutError::EngineValidation(error.to_string()),
        })?;
        Ok(())
    }

    pub(super) fn run_second_bin_pack(&mut self) -> Result<(), limits::Error> {
        // The second recovered pass evaluates hierarchy groups against the
        // concrete routed segments and translates accepted routes together
        // with their component. Root packing remains on the corpus-proven flat
        // path until its hierarchy proxies are represented in ArenaGraph.
        if self.graph.is_root_grid_only() && self.graph.root_grid_bin_pack_completed {
            return Ok(());
        }
        self.graph.bin_pack_recovered()
    }

    pub(super) fn run_place_labels(&mut self) -> Result<(), crate::LayoutError> {
        #[cfg(feature = "diagnostic-traces")]
        labels::trace_label_placement_input(&self.graph);
        labels::place_labels_checked(&mut self.graph).map_err(|error| match error {
            limits::Error::Context {
                cause: limits::ContextError::Cancelled,
                ..
            } => crate::LayoutError::Cancelled,
            limits::Error::Context {
                cause: limits::ContextError::DeadlineExceeded,
                ..
            } => crate::LayoutError::DeadlineExceeded,
            limits::Error::SeedInvariant(_) => crate::LayoutError::SeedInvariant,
            error => crate::LayoutError::EngineValidation(error.to_string()),
        })
    }

    pub(super) fn run_nudge_edge_channels_checked(&mut self) -> Result<(), crate::LayoutError> {
        self.route_stage_work = routing::RouteStageWork::new(routing::MAX_ROUTE_STAGE_WORK_UNITS);
        match routing::nudge_edge_channels_with_budget(&mut self.graph, &self.route_stage_work) {
            Ok(()) => {}
            Err(error) if error.is_route_work_limit() => {}
            Err(error) => return Err(error.into_layout_error(self.seed)),
        }
        self.trace_route_stage("NudgeEdgeChannels");
        Ok(())
    }

    pub(super) fn run_shortcut_edge_routes_checked(&mut self) -> Result<(), crate::LayoutError> {
        self.route_stage_work = routing::RouteStageWork::new(routing::MAX_ROUTE_STAGE_WORK_UNITS);
        // The optional source pass swallows only its own route-work sentinel,
        // after the atomic boundary has restored all state.
        match routing::shortcut_edge_routes_with_budget(&mut self.graph, &self.route_stage_work) {
            Ok(()) => {}
            Err(error) if error.is_route_work_limit() => {}
            Err(error) => return Err(error.into_layout_error(self.seed)),
        }
        self.trace_route_stage("ShortcutEdgeRoutes");
        Ok(())
    }

    pub(super) fn run_normalize(&mut self) -> Result<(), crate::LayoutError> {
        self.graph.normalize_seed()
    }

    pub(super) fn snapshot(&self, stage: LayoutStage) -> LayoutSnapshot {
        let mut retained_graph = self.graph.clone();
        retained_graph.detach_cost_owner();
        LayoutSnapshot {
            stage,
            cell_size: self.graph.cell_size,
            turn_cost: self.graph.raw_turn_cost(),
            run_edge_route: self.run_edge_route,
            nodes: self
                .graph
                .nodes
                .iter()
                .filter(|node| {
                    self.graph.topology_arena_id().is_none()
                        || self
                            .graph
                            .topology_source_input_node(node.input_id)
                            .is_some()
                })
                .map(|node| LayoutNodeState {
                    node: node.input_id,
                    tala_id: node.tala_id,
                    shape: node.shape,
                    position: self.graph.position(node.input_id),
                    rect: Rect {
                        origin: self.graph.position(node.input_id).unwrap_or_default(),
                        size: node.rect.size,
                    },
                    label_size: node.label_size,
                    font_size: node.font_size,
                    label_position: node.label_position,
                    icon_position: node.icon_position,
                    sizeless_edge_length: self.graph.sizeless_edge_length(node.input_id, true),
                })
                .collect(),
            // This public input-ID result retains input allocation order;
            // GraphJSON separately serializes actual Graph.Edges ordering.
            edges: self
                .graph
                .edge_records
                .iter()
                .enumerate()
                .filter(|(index, _)| self.graph.edge_order.contains(&EdgeId(*index as u32)))
                .map(|(_, edge)| LayoutEdgeState {
                    edge: edge.input_id,
                    tala_id: edge.tala_id,
                    source_node: edge.from,
                    target_node: edge.to,
                    is_curve: edge.is_curve,
                    points: edge.route.read().clone(),
                    label: edge.label.clone(),
                })
                .collect(),
            next_rng_float: self.next_rng_float,
            hierarchy_placements: self.hierarchy_placements.clone(),
            hierarchy_assignments: self.hierarchy_assignments.clone(),
            hierarchy_rng_shuffle_lengths: self.hierarchy_rng.shuffle_lengths().to_vec(),
            // Keep the completed arena for the adapter-level post-selection
            // CompoundCandidate pass. The field is serde-skipped, so this
            // does not alter the diagnostic wire format.
            graph: Some(retained_graph),
            layout_error: None,
        }
    }
}

#[cfg(test)]
mod route_curve_state_tests {
    use super::*;
    use crate::{Edge, Node, Size};

    #[test]
    fn fresh_layout_clears_curve_state_but_prearranged_routing_preserves_it() {
        let mut input = Graph::default();
        let source = input.add_node(Node::new(
            "source",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let target = input.add_node(Node::new(
            "target",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let edge = input.add_edge(Edge { source, target });
        input.set_edge_is_curve(edge, true);

        let full_layout = Pipeline::new(&input, 1, false, false);
        assert!(!full_layout.graph.edge_records[edge.0 as usize].is_curve);

        let route_existing = Pipeline::new(&input, 1, true, false);
        assert!(route_existing.graph.edge_records[edge.0 as usize].is_curve);
    }

    #[test]
    fn normalized_trace_records_curves_and_omits_polyline_defaults() {
        let polyline = NormalizedTraceEdge {
            id: "7".into(),
            index: 0,
            from: String::new(),
            to: String::new(),
            curve: false,
            route: Vec::new(),
        };
        let curve = NormalizedTraceEdge {
            id: "7".into(),
            index: 0,
            from: String::new(),
            to: String::new(),
            curve: true,
            route: Vec::new(),
        };

        assert_eq!(
            serde_json::to_value(polyline).unwrap(),
            serde_json::json!({"id":"7", "index":0, "from":"", "to":""})
        );
        assert_eq!(
            serde_json::to_value(curve).unwrap(),
            serde_json::json!({"id":"7", "index":0, "from":"", "to":"", "curve":true})
        );
    }
}
