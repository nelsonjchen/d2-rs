// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rust TALA internals shaped after D2 v0.9.0's `d2talalayout/internal` tree.
//!
//! Stable `NodeId`/`EdgeId` arenas represent graph identity while keeping
//! mutation explicit and borrow-checker friendly. Recovery provenance belongs
//! beside the individual algorithms and their tests, not in the module name.

#![allow(dead_code)]
// Several recovered algorithms deliberately keep their source-shaped argument
// lists and tuple records because ordering and ownership are parity-sensitive.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

#[cfg(feature = "diagnostic-traces")]
pub(crate) mod call_counts;

use super::{
    ArrowheadLabel, CancellationToken, CandidateReport, CanvasPosition, Direction, EdgeId,
    EdgeLabel, ExternalLabel, ExternalSide, Graph, Insets, LabelPosition, LayoutError,
    LayoutOptions, LayoutReport, LayoutResult, NodeId, NodeLabelState, Point, Rect,
    SerializedAggregateEdgeAbduction, SerializedClusterState, SerializedSequenceState, ShapeKind,
    Size,
};
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "diagnostic-traces"))]
use std::cell::Cell;
use std::cell::RefCell;
#[cfg(feature = "diagnostic-traces")]
use std::collections::HashSet;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
#[cfg(feature = "diagnostic-traces")]
use std::ffi::OsString;
use std::sync::Arc;
#[cfg(feature = "diagnostic-traces")]
use std::sync::OnceLock;
#[cfg(feature = "diagnostic-traces")]
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// D2 v0.9.0 layoutgraph.ContainerPadding, used when ancestor obstacle boxes
/// are translated into a fixed container's local coordinate frame.
const CONTAINER_PADDING: f64 = 60.0;

/// Diagnostic flags are read from the process environment many times inside
/// transaction and scoring loops. Snapshot the TALA keys once; the layout
/// protocol configures these flags before entering the engine, and no layout
/// code mutates them while a graph is running.
#[cfg(feature = "diagnostic-traces")]
static ENVIRONMENT_KEYS: OnceLock<HashSet<OsString>> = OnceLock::new();

/// Embedder callback run after each checked EdgeRouting pass; see
/// `crate::set_routing_release_hook`.
static ROUTING_RELEASE_HOOK: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

pub(crate) fn set_routing_release_hook(hook: fn()) {
    let _ = ROUTING_RELEASE_HOOK.set(hook);
}

fn run_routing_release_hook() {
    if let Some(hook) = ROUTING_RELEASE_HOOK.get() {
        hook();
    }
}
#[cfg(feature = "diagnostic-traces")]
static TALA_TRACE_ENVIRONMENT_STATE: AtomicU8 = AtomicU8::new(0);
#[cfg(feature = "diagnostic-traces")]
static SIZED_PASS_TRACE_ACTIVE: AtomicBool = AtomicBool::new(false);

thread_local! {
    // Production work guards are source-shaped around a context interface.
    // Keep the public cancellation handle out of every recovered function
    // signature while still making those guards observe the caller's token.
    static ACTIVE_CANCELLATION: RefCell<Option<CancellationToken>> = const { RefCell::new(None) };
}

/// Runs a layout operation with a cancellation token installed for this
/// thread. Nested operations restore the previous token on normal return.
pub(crate) fn with_cancellation<T>(
    token: Option<&CancellationToken>,
    operation: impl FnOnce() -> T,
) -> T {
    struct RestoreCancellation(Option<CancellationToken>);
    impl Drop for RestoreCancellation {
        fn drop(&mut self) {
            ACTIVE_CANCELLATION.with(|active| {
                active.replace(self.0.take());
            });
        }
    }
    let restore =
        RestoreCancellation(ACTIVE_CANCELLATION.with(|active| active.replace(token.cloned())));
    let result = operation();
    drop(restore);
    result
}

/// Returns whether the current layout call has been canceled.
pub(crate) fn cancellation_requested() -> bool {
    ACTIVE_CANCELLATION.with(|active| {
        active
            .borrow()
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    })
}

/// Returns the Go-context-shaped `Done` state for production work guards.
pub(crate) fn cancellation_done() -> Option<bool> {
    ACTIVE_CANCELLATION.with(|active| {
        active
            .borrow()
            .as_ref()
            .map(CancellationToken::is_cancelled)
    })
}

/// Returns a clone of the caller's cancellation handle for work that crosses
/// an OS-thread boundary. The active handle is kept thread-local so ordinary
/// calls stay allocation-free; parallel TALA workers must explicitly install
/// the same handle before entering source-shaped guards.
pub(crate) fn current_cancellation() -> Option<CancellationToken> {
    ACTIVE_CANCELLATION.with(|active| active.borrow().clone())
}

/// Returns the context cause expected by the recovered work guards.
pub(crate) fn cancellation_error() -> Option<limits::ContextError> {
    match cancellation_layout_error()? {
        LayoutError::DeadlineExceeded => Some(limits::ContextError::DeadlineExceeded),
        _ => Some(limits::ContextError::Cancelled),
    }
}

/// The caller's typed context error at the current engine boundary.
pub(crate) fn cancellation_layout_error() -> Option<LayoutError> {
    ACTIVE_CANCELLATION.with(|active| active.borrow().as_ref().and_then(CancellationToken::error))
}

/// Checks the current cancellation request at a publication boundary.
pub(crate) fn check_cancellation() -> Result<(), LayoutError> {
    cancellation_layout_error().map_or(Ok(()), Err)
}

#[cfg(feature = "diagnostic-traces")]
pub(crate) fn set_sized_pass_trace_active(active: bool) {
    SIZED_PASS_TRACE_ACTIVE.store(active, Ordering::Relaxed);
}

#[cfg(not(feature = "diagnostic-traces"))]
#[inline(always)]
pub(crate) fn set_sized_pass_trace_active(_active: bool) {}

#[cfg(feature = "diagnostic-traces")]
const TALA_TRACE_ENVIRONMENT_ABSENT: u8 = 1;
#[cfg(feature = "diagnostic-traces")]
const TALA_TRACE_ENVIRONMENT_PRESENT: u8 = 2;

#[cfg(feature = "diagnostic-traces")]
#[inline(always)]
pub(crate) fn trace_env_enabled(name: &str) -> bool {
    // The sized-pass probe toggles this key between optimizer iterations so
    // that a single requested pass can be inspected without making every
    // optimizer call noisy.  Keep this one diagnostic gate live; the ordinary
    // TALA_TRACE_* flags remain snapshotted for the hot layout loops.
    if name == "TALA_TRACE_SIZED_PASS_ACTIVE" {
        return SIZED_PASS_TRACE_ACTIVE.load(Ordering::Relaxed);
    }
    if TALA_TRACE_ENVIRONMENT_STATE.load(Ordering::Relaxed) == TALA_TRACE_ENVIRONMENT_ABSENT {
        return false;
    }
    trace_env_enabled_slow(name)
}

#[cfg(feature = "diagnostic-traces")]
#[cold]
#[inline(never)]
fn trace_env_enabled_slow(name: &str) -> bool {
    let keys = ENVIRONMENT_KEYS.get_or_init(|| {
        std::env::vars_os()
            .filter_map(|(key, _)| {
                let name = key.to_string_lossy();
                (name.starts_with("D2RS_") || name.starts_with("TALA_TRACE_")).then_some(key)
            })
            .collect()
    });
    TALA_TRACE_ENVIRONMENT_STATE.store(
        if keys.is_empty() {
            TALA_TRACE_ENVIRONMENT_ABSENT
        } else {
            TALA_TRACE_ENVIRONMENT_PRESENT
        },
        Ordering::Relaxed,
    );
    keys.contains(std::ffi::OsStr::new(name))
}

#[cfg(feature = "diagnostic-traces")]
#[inline(always)]
pub(crate) fn trace_env_value(name: &str) -> Option<String> {
    if !trace_env_enabled(name) {
        return None;
    }
    std::env::var(name).ok()
}

#[cfg(not(feature = "diagnostic-traces"))]
#[inline(always)]
// Keep this a `const fn`: in ordinary builds every `if trace_env_enabled(...)`
// branch is folded during compilation, so the tracing probes and their
// formatting arguments do not survive into the optimized binary.
pub(crate) const fn trace_env_enabled(_name: &str) -> bool {
    false
}

#[cfg(not(feature = "diagnostic-traces"))]
#[inline(always)]
// `None` is likewise const-foldable at call sites that gate optional trace
// filters, including the hot placement and routing loops.
pub(crate) const fn trace_env_value(_name: &str) -> Option<String> {
    None
}

/// Hasher for lookup-only maps keyed by small integer identities (TALA ids,
/// node and edge indices). Uses the multiply-rotate mixing of rustc's FxHash.
/// Use it only where iteration order never reaches output; ordered or iterated
/// maps stay BTreeMap.
#[derive(Default, Clone, Copy)]
pub(crate) struct IdHasher(u64);
impl std::hash::Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }
    fn write_i64(&mut self, value: i64) {
        self.write_u64(value as u64);
    }
    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}
pub(crate) type IdHashMap<K, V> =
    std::collections::HashMap<K, V, std::hash::BuildHasherDefault<IdHasher>>;
pub(crate) type IdHashSet<K> =
    std::collections::HashSet<K, std::hash::BuildHasherDefault<IdHasher>>;

/// Hasher for lookup-only maps keyed by Go float map keys (f64 bit patterns).
/// Round coordinates leave the low mantissa bits zero, and the IdHasher
/// product keeps them zero, so hashbrown's low-bit bucket index would collide.
/// The murmur3 finalizer folds the high bits down before the index is taken.
#[derive(Default, Clone, Copy)]
pub(crate) struct FloatKeyHasher(IdHasher);
impl std::hash::Hasher for FloatKeyHasher {
    fn finish(&self) -> u64 {
        let mut hash = self.0.finish();
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        hash ^ (hash >> 33)
    }
    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }
    fn write_u64(&mut self, value: u64) {
        self.0.write_u64(value);
    }
}
pub(crate) type FloatKeyHashMap<K, V> =
    std::collections::HashMap<K, V, std::hash::BuildHasherDefault<FloatKeyHasher>>;
pub(crate) type FloatKeyHashSet<K> =
    std::collections::HashSet<K, std::hash::BuildHasherDefault<FloatKeyHasher>>;

#[path = "placement/alignment.rs"]
mod alignment;
#[path = "layoutgraph/connected_set.rs"]
mod connected_set;
pub(crate) mod go_cell;
use connected_set::ConnectedNode;
#[path = "layoutgraph/arena.rs"]
mod arena;
#[path = "layoutgraph/container_slices.rs"]
mod container_slices;
use container_slices::{ContainerMap, SharedNodeSlice, SharedSliceMap, SharedSliceMapSnapshot};
#[path = "layoutgraph/owner_tree_map.rs"]
mod owner_tree_map;
use owner_tree_map::OwnerTreeId;
#[path = "layoutgraph/incident_edges.rs"]
mod incident_edges;
#[path = "layoutgraph/node_near_map.rs"]
mod node_near_map;
#[path = "layoutgraph/owner_groups.rs"]
mod owner_groups;
#[path = "layoutgraph/owner_pointer_slices.rs"]
mod owner_pointer_slices;
#[path = "layoutgraph/owner_points.rs"]
mod owner_points;
#[path = "layoutgraph/route_points.rs"]
mod route_points;
use incident_edges::IncidentEdges;
#[path = "layoutgraph/graph_edges.rs"]
mod graph_edges;
use graph_edges::GraphEdges;
#[path = "layoutgraph/graph_node_maps.rs"]
mod graph_node_maps;
use graph_node_maps::GraphNodeMap;
#[path = "layoutgraph/arena_clone.rs"]
mod arena_clone;
use node_near_map::NodeNearMap;
use route_points::RoutePoints;
#[path = "layoutgraph/shared_topology.rs"]
mod shared_topology;
use shared_topology::{
    OwnerAbductionId, OwnerAbductionSlice, OwnerArenaId, OwnerEdgeAbduction, OwnerGraphId,
    OwnerGraphOwnershipJournal, OwnerGraphView, OwnerMemberHeader, OwnerTopologyError,
    SharedTopology, TopologyVessel,
};
#[path = "layoutgraph/owner_score_frame.rs"]
mod owner_score_frame;
use owner_score_frame::{OwnerScoreError, OwnerScoreFrame};
#[path = "layoutgraph/owner_score_session.rs"]
mod owner_score_session;
#[path = "layoutgraph/owner_swap_motion.rs"]
mod owner_swap_motion;
#[cfg(feature = "diagnostic-traces")]
use owner_score_frame::OwnerObstructionSet;
use owner_score_session::{CanonicalScoreMapping, OwnerNodeGeometry, OwnerScoreSession};
#[path = "packing/binpack.rs"]
mod binpack;
#[path = "packing/binpack_candidates.rs"]
mod binpack_candidates;
#[path = "packing/binpack_group_work.rs"]
mod binpack_group_work;
#[path = "packing/binpack_guard.rs"]
mod binpack_guard;
#[path = "packing/binpack_routed_container.rs"]
mod binpack_routed_container;
#[path = "packing/binpack_segments.rs"]
mod binpack_segments;
#[path = "packing/binpack_staging.rs"]
mod binpack_staging;
#[path = "packing/binpack_traversal.rs"]
mod binpack_traversal;
#[path = "graphbounds/bounds.rs"]
mod bounds;
#[path = "graphbounds/checked_bounds.rs"]
mod checked_bounds;
#[path = "grouping/clusters.rs"]
mod clusters;
#[path = "placement/compaction_candidates.rs"]
mod compaction_candidates;
#[path = "placement/compaction_checkpoint.rs"]
mod compaction_checkpoint;
#[path = "placement/compaction_clusters.rs"]
mod compaction_clusters;
#[path = "placement/compaction_sized.rs"]
mod compaction_sized;
#[path = "placement/compaction_sizeless.rs"]
mod compaction_sizeless;
#[path = "placement/compaction_transition.rs"]
mod compaction_transition;
#[path = "engine/compositions.rs"]
mod compositions;
#[path = "engine/compositions_nested.rs"]
mod compositions_nested;
#[path = "engine/compositions_packs.rs"]
mod compositions_packs;
#[path = "engine/compositions_structures.rs"]
mod compositions_structures;
#[path = "hierarchy/compound.rs"]
mod compound;
#[path = "engine/container_grids.rs"]
mod container_grids;
#[path = "placement/container_orientation.rs"]
mod container_orientation;
#[path = "packing/container_packing.rs"]
mod container_packing;
#[path = "engine/container_shapes.rs"]
mod container_shapes;
#[path = "engine/containers.rs"]
mod containers;
#[path = "placementcost/evaluation.rs"]
mod evaluation;
#[path = "placement/gap_guard.rs"]
mod gap_guard;
#[path = "placement/gap_normalization.rs"]
mod gap_normalization;
#[path = "placement/geometry_checkpoint.rs"]
mod geometry_checkpoint;
use geometry_checkpoint::GeometryCheckpoint;
#[cfg(test)]
#[path = "placement/gap_normalization_tests.rs"]
mod gap_normalization_tests;
#[cfg(feature = "diagnostic-traces")]
#[path = "engine/gap_stage_probe.rs"]
mod gap_stage_probe;
#[path = "placement/gap_transaction_work.rs"]
mod gap_transaction_work;
#[cfg(test)]
#[path = "placement/geometry_crosswalk_tests.rs"]
mod geometry_crosswalk_tests;
#[path = "runtime/go_rng.rs"]
mod go_rng;
#[path = "runtime/go_sort.rs"]
mod go_sort;
#[path = "hierarchy/hierarchy.rs"]
mod hierarchy;
#[path = "hierarchy/hierarchy_assignment.rs"]
mod hierarchy_assignment;
#[path = "hierarchy/hierarchy_network_simplex.rs"]
mod hierarchy_network_simplex;
#[path = "hierarchy/hierarchy_order.rs"]
mod hierarchy_order;
#[path = "hierarchy/hierarchy_placement.rs"]
mod hierarchy_placement;
#[path = "hierarchy/hierarchy_scope.rs"]
mod hierarchy_scope;
#[path = "invariant/mod.rs"]
mod invariant;
#[path = "labelgeom/mod.rs"]
mod labelgeom;
#[path = "labeling/labels.rs"]
mod labels;
#[path = "loops/offsets.rs"]
mod loop_offsets;
#[cfg(feature = "diagnostic-traces")]
#[path = "engine/loop_offsets_probe.rs"]
mod loop_offsets_probe;
#[path = "loops/mod.rs"]
mod loops;
#[cfg(feature = "diagnostic-traces")]
#[path = "engine/node_bounds_probe.rs"]
mod node_bounds_probe;
#[cfg(feature = "diagnostic-traces")]
#[path = "engine/route_owner_probe.rs"]
mod route_owner_probe;
#[path = "placement/transaction_container_refit.rs"]
mod transaction_container_refit;
#[path = "placement/trial_geometry.rs"]
mod trial_geometry;
use loop_offsets::*;
#[path = "engine/compound_routes.rs"]
mod compound_routes;
#[cfg(test)]
#[path = "loops/offsets_tests.rs"]
mod loop_offsets_tests;
#[path = "layoutgraph/model.rs"]
mod model;
#[path = "layoutgraph/node_requirements_map.rs"]
mod node_requirements_map;
#[path = "nodeshape/mod.rs"]
mod nodeshape;
#[path = "engine/normalization.rs"]
mod normalization;
#[path = "placement/occupied.rs"]
mod occupied;
#[path = "placement/optimization.rs"]
mod optimization;
#[path = "placement/optimizer_spatial_index.rs"]
mod optimizer_spatial_index;
#[path = "placement/owner_adjacents.rs"]
mod owner_adjacents;
#[path = "placement/owner_best_swap.rs"]
mod owner_best_swap;
#[path = "placement/owner_hub_spokes.rs"]
mod owner_hub_spokes;
#[path = "placement/owner_median.rs"]
mod owner_median;
#[path = "placement/owner_near_eligibility.rs"]
mod owner_near_eligibility;
#[path = "placement/owner_optimizer.rs"]
mod owner_optimizer;
#[path = "placement/owner_placement.rs"]
mod owner_placement;
#[path = "placement/owner_sizeless_constructor.rs"]
mod owner_sizeless_constructor;
#[path = "placementcost/owner_sizeless_geometry.rs"]
mod owner_sizeless_geometry;
#[path = "placement/owner_sizeless_loop.rs"]
mod owner_sizeless_loop;
#[path = "placement/owner_swap_candidates.rs"]
mod owner_swap_candidates;
#[path = "engine/pipeline.rs"]
mod pipeline;
#[path = "proximity/mod.rs"]
mod proximity;
#[path = "packing/root_packing.rs"]
mod root_packing;
#[path = "routing/mod.rs"]
mod routing;
#[path = "placementcost/scoring.rs"]
mod scoring;
#[path = "placementcost/cancellation.rs"]
mod scoring_context;
#[path = "placementcost/scoring_sizeless.rs"]
mod scoring_sizeless;
#[path = "grouping/sequences.rs"]
mod sequences;
#[path = "placement/sized.rs"]
mod sized;
#[path = "placement/sizeless.rs"]
mod sizeless;
#[path = "engine/snapshot.rs"]
mod snapshot;
#[cfg(feature = "diagnostic-traces")]
pub use gap_stage_probe::diagnostic_gap_stage_probe;
#[cfg(feature = "diagnostic-traces")]
pub use loop_offsets_probe::diagnostic_loop_offsets_probe;
#[cfg(feature = "diagnostic-traces")]
pub use node_bounds_probe::diagnostic_node_bounds_probe;
#[path = "placement/symmetry.rs"]
mod symmetry;
#[path = "placement/table_columns.rs"]
mod table_columns;
#[path = "placement/transactions.rs"]
mod transactions;
#[path = "trees/owner_publication.rs"]
mod tree_owner_publication;
#[path = "trees/trees.rs"]
mod trees;
#[path = "typedpool/mod.rs"]
mod typedpool;
#[path = "engine/validation.rs"]
mod validation;

use model::{
    AbductedNodeRef, AbductionCurrentNodeRef, AbductionNodeLocation, AggregateNodeRef, ArenaEdge,
    ArenaGraph, ArenaNode, ClusterArrangement, ClusterState, ExternalClusterLayout,
    FlatScopePlacement, GraphCostCache, HerdAssignment, HierarchyAlignmentDirection,
    HierarchyAlignmentNodeState, HierarchyAlignmentState, HierarchyAssignmentTrace,
    HierarchyMembership, HierarchyOrderStage, HierarchyOrderState, HierarchyPlacementLevelState,
    HierarchyPlacementNodeState, HierarchyPlacementTrace, OrderedObstructionInventory, Orientation,
    Pipeline, PlacementCostSnapshot, PlacementOwnerSource, PlacementScope, ProjectedAdjacent,
    ProjectedCluster, ProjectedClusterDistance, ProjectedClusterLayout, ProjectedObstructionRef,
    ProjectedObstructionSet, ProjectedTableColumnNeighbor, ProjectionGenerations, ProjectionMemo,
    ResolvedAggregateEdgeAbduction, RoutingCostField, ScopeNodeMetadata, SequenceState,
    SharedProjections, SizedEdgeAbduction, SourceOrientationContext, Stage, SubtreeMirror, Tracked,
    TreeRoutingNode, compass_axis_delta, compass_delta, fnv1a32, layout_orientation,
};
pub use model::{
    HierarchyAlignmentDirection as LayoutHierarchyAlignmentDirection,
    HierarchyAlignmentNodeState as LayoutHierarchyAlignmentNodeState,
    HierarchyAlignmentState as LayoutHierarchyAlignmentState,
    HierarchyAssignmentTrace as LayoutHierarchyAssignmentTrace,
    HierarchyOrderStage as LayoutHierarchyOrderStage,
    HierarchyOrderState as LayoutHierarchyOrderState,
    HierarchyPlacementLevelState as LayoutHierarchyPlacementLevelState,
    HierarchyPlacementNodeState as LayoutHierarchyPlacementNodeState,
    HierarchyPlacementTrace as LayoutHierarchyPlacementTrace, LayoutEdgeState, LayoutNodeState,
    LayoutSnapshot, LayoutStage,
};
pub use snapshot::layout_snapshot;
pub(crate) use snapshot::{layout_result, layout_result_with_cancellation};
pub(crate) use snapshot::{route_existing, route_existing_with_cancellation};

#[cfg(test)]
mod tests {
    // These closures hold mutable graph borrows; explicitly dropping them marks
    // the transition from fixture construction to assertions.
    #![allow(clippy::drop_non_drop)]

    use super::model::STAGES;

    include!("tests/core.rs");
    include!("tests/normalization_and_labels.rs");
    include!("tests/containers_and_compositions.rs");
    include!("tests/binpack_and_hierarchy.rs");
    include!("tests/routing.rs");
}

mod quality;

mod limits;

#[path = "placement/stress_initialize.rs"]
mod stress_initialize;

pub(crate) mod graphjson;

#[path = "placement/ancestor_obstacles.rs"]
mod ancestor_obstacles;
#[path = "packing/combine.rs"]
mod combine;
#[cfg(feature = "diagnostic-traces")]
#[path = "engine/combine_work_probe.rs"]
mod combine_work_probe;
#[cfg(feature = "diagnostic-traces")]
pub use combine_work_probe::diagnostic_combine_work_probe;

#[path = "placement/compaction_moves.rs"]
mod compaction_moves;
#[path = "placement/sized_scoring_work.rs"]
mod optimizer_scoring_work;

#[path = "placement/compaction_scoring.rs"]
mod compaction_scoring;
