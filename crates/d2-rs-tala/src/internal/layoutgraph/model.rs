// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Internal arena model and diagnostic snapshot types.
//!
//! Stable IDs keep identity explicit while placement stages temporarily replace
//! nodes with sequence, cluster, tree, and hierarchy carriers.

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Observable boundary in the production layout pipeline.
///
/// This enum is doc-hidden because snapshots are diagnostic, not a stable
/// application API. Variants progress in pipeline order.
pub enum LayoutStage {
    /// After unit-aspect and edge-dense nodes have been fitted for placement.
    Prescale,
    /// After hierarchy candidates have been assigned and placed.
    PreprocessHierarchies,
    /// After general-placement nodes receive initial integer cells.
    InitializeNodes,
    /// After one zero-temperature sizeless optimization probe.
    SizelessOptimize,
    /// After the first nonzero-temperature sizeless pass.
    SizelessFirstPass,
    /// After the first horizontal sizeless compaction.
    SizelessFirstCompaction,
    /// After the next cooled sizeless pass.
    SizelessSecondPass,
    /// After the second observed sizeless compaction boundary.
    SizelessSecondCompaction,
    /// After the complete sizeless annealing schedule.
    SizelessAnneal,
    /// After cell coordinates are compacted for real-size placement.
    TransitionCompaction,
    /// At the beginning of size-aware placement.
    SizedStart,
    /// After the first node proposal of sized optimization.
    SizedFirstNode,
    /// After the first full size-aware pass.
    SizedFirstPass,
    /// After the second full size-aware pass.
    SizedSecondPass,
    /// Immediately before scheduled sized compaction begins.
    SizedPreCompaction,
    /// After pre-compaction passes but before the first compaction operation.
    SizedBeforeFirstCompaction,
    /// After the first size-aware compaction.
    SizedFirstCompaction,
    /// After the second size-aware compaction.
    SizedSecondCompaction,
    /// After the complete size-aware annealing schedule.
    SizedAnneal,
    /// After the final zero-temperature size-aware pass.
    SizedZeroOptimize,
    /// After recursive scope placement has published node coordinates.
    NodePlacement,
    /// After the first swap-optimization pass.
    SwapFirstPass,
    /// After the second swap-optimization pass.
    SwapSecondPass,
    /// After the third swap-optimization pass.
    SwapThirdPass,
    /// After the complete swap and direction-refinement stage.
    SwapStuff,
    /// After eligible structures have been transposed.
    Transpose,
    /// After nearly shared axes have been aligned.
    AlignAxes,
    /// After uneven horizontal and vertical gaps have been reduced.
    GapNormalization,
    /// After aggregate cluster geometry has been refined.
    OptimizeClusters,
    /// After repeated structures have been balanced for symmetry.
    BalanceSymmetry,
    /// After eligible repeated distances have been equalized.
    Equidistance,
    /// After disconnected placed components have been packed before routing.
    FirstBinPack,
    /// After aggregate members return to live order and routing padding is added.
    Rescale,
    /// After the first ordinary edge-routing pass.
    EdgeRouting,
    /// After shared routing lanes have been crosshatched apart.
    Crosshatch,
    /// After an eligible short endpoint dogleg may have moved its leaf box.
    Dejitter,
    /// After routing is conditionally repeated following Dejitter or a missing route.
    SecondEdgeRouting,
    /// After redundant route points have been removed.
    SimplifyEdgeRoutes,
    /// After compatible endpoint segments have been uncrossed when geometrically safe.
    SwapEdgePorts,
    /// After eligible routes are replaced by a legal straight route only when cheaper.
    StraightEdgesFallback,
    /// After parallel orthogonal segments have been balanced.
    BalanceEdgeSegments,
    /// After branches near cluster boundaries have been repaired.
    FixClusterEdgeBranching,
    /// After endpoints have been clipped to exact rendered shape borders.
    TraceEdgesToShapeBorder,
    /// After an eligible labelled duplicate may exchange its polyline with an outside lane.
    ReorderDuplicates,
    /// After route-aware repacking and final node, edge, icon, and arrowhead-label placement.
    PlaceLabels,
    /// After all published geometry has been translated into its output frame.
    Normalize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Per-node geometry captured in a diagnostic [`LayoutSnapshot`].
pub struct LayoutNodeState {
    /// Stable input node ID.
    pub node: NodeId,
    /// Release-facing identity used to align diagnostic snapshots with the
    /// pristine TALA debug SVGs. This is diagnostic metadata only.
    pub tala_id: i64,
    /// Rendered shape used at this boundary.
    pub shape: ShapeKind,
    /// Active placement coordinate, when one has been assigned.
    pub position: Option<Point>,
    /// Current rectangle in the snapshot coordinate frame.
    pub rect: Rect,
    /// Current rendered label dimensions.
    pub label_size: Option<Size>,
    /// Current font size after any fitting step.
    pub font_size: Option<u32>,
    /// Current node-label position.
    pub label_position: LabelPosition,
    /// Current icon position.
    pub icon_position: Option<LabelPosition>,
    /// Edge-length contribution used by sizeless scoring for this node.
    pub sizeless_edge_length: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Per-edge geometry captured in a diagnostic [`LayoutSnapshot`].
pub struct LayoutEdgeState {
    /// Stable input edge ID.
    pub edge: EdgeId,
    /// Signed upstream identity, independent of the dense edge index.
    pub tala_id: i64,
    /// Current source node after preprocessing and aggregate reset.
    pub source_node: NodeId,
    /// Current target node after preprocessing and aggregate reset.
    pub target_node: NodeId,
    /// Whether the current route is an explicitly marked cubic curve.
    #[serde(default, skip_serializing_if = "bool_is_false")]
    pub is_curve: bool,
    /// Current source-to-target point list; empty before routing.
    pub points: Vec<Point>,
    /// Current label state, when the edge has a label.
    pub label: Option<EdgeLabel>,
}

fn bool_is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Crossing-reduction boundary recorded for a hierarchy.
pub enum HierarchyOrderStage {
    /// Stable order before crossing reduction.
    Initial,
    /// Order after local crossing minimization.
    AfterMinimize,
    /// Order after global sifting.
    AfterGlobalSifting,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// One real or dummy hierarchy node at a ranked level.
pub struct HierarchyPlacementNodeState {
    /// Stable input node ID, or `None` for a dummy long-edge segment.
    pub node: Option<NodeId>,
    /// Owning container when the ranked item represents nested content.
    pub container: Option<NodeId>,
    /// Zero-based hierarchy level.
    pub level: usize,
    /// Zero-based position within that level.
    pub rank: usize,
    /// Whether this entry is a synthetic dummy node.
    pub dummy: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// Ordered contents and outgoing crossing count for one hierarchy level.
pub struct HierarchyPlacementLevelState {
    /// Zero-based level number.
    pub level: usize,
    /// Nodes in left-to-right rank order.
    pub nodes: Vec<HierarchyPlacementNodeState>,
    /// Crossings between this level and the following level.
    pub crossings_to_next_level: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// Complete hierarchy ordering at one reduction boundary.
pub struct HierarchyOrderState {
    /// Algorithm boundary represented by this record.
    pub stage: HierarchyOrderStage,
    /// Ranked levels in ascending level order.
    pub levels: Vec<HierarchyPlacementLevelState>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Vertical-alignment and compaction perspective.
pub enum HierarchyAlignmentDirection {
    /// Traverse levels top-down and ranks left-to-right.
    TopLeft,
    /// Traverse levels top-down and ranks right-to-left.
    TopRight,
    /// Traverse levels bottom-up and ranks left-to-right.
    BottomLeft,
    /// Traverse levels bottom-up and ranks right-to-left.
    BottomRight,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Alignment-block data for one real or dummy hierarchy node.
pub struct HierarchyAlignmentNodeState {
    /// Stable node ID, or `None` for a dummy node.
    pub node: Option<NodeId>,
    /// Root real node of this node's aligned block.
    pub root: Option<NodeId>,
    /// Final node-axis coordinate in this perspective.
    pub x: f64,
    /// Coordinate assigned to the alignment block.
    pub block_x: f64,
    /// Total width required by the block.
    pub block_size: f64,
    /// Clearance required to the left of the node.
    pub left_pad: f64,
    /// Clearance required to the right of the node.
    pub right_pad: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// One directional hierarchy alignment and compaction result.
pub struct HierarchyAlignmentState {
    /// Perspective used to produce this state.
    pub direction: HierarchyAlignmentDirection,
    /// Minimum block coordinate.
    pub min_x: f64,
    /// Maximum block coordinate.
    pub max_x: f64,
    /// Per-node alignment-block state.
    pub nodes: Vec<HierarchyAlignmentNodeState>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Ordering, alignment, and random-stream trace for one placed hierarchy.
pub struct HierarchyPlacementTrace {
    /// Invocation-local hierarchy identifier.
    pub hierarchy_id: usize,
    /// Root nodes that entered the hierarchy builder.
    pub roots: Vec<NodeId>,
    /// Hierarchy RNG draw count before graph construction.
    pub rng_draws_before_build: u64,
    /// Hierarchy RNG draw count after graph construction.
    pub rng_draws_after_build: u64,
    /// Hierarchy RNG draw count after final placement.
    pub rng_draws_after_placement: u64,
    /// Observed node orders during crossing reduction.
    pub orders: Vec<HierarchyOrderState>,
    /// Observed directional alignment results.
    pub alignments: Vec<HierarchyAlignmentState>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// Decision trace for one automatically considered hierarchy component.
pub struct HierarchyAssignmentTrace {
    /// Container whose direct children formed the assignment scope; `None`
    /// identifies the root scope.
    pub scope: Option<NodeId>,
    /// Weakly connected component considered by assignment.
    pub component: Vec<NodeId>,
    /// Nodes remaining after eligibility filtering.
    pub candidate_nodes: Vec<NodeId>,
    /// Whether every structural eligibility predicate passed.
    pub eligible: bool,
    /// Whether the component reached network-simplex ranking.
    pub rank_attempted: bool,
    /// Whether the ranked component was accepted as a hierarchy.
    pub accepted: bool,
    /// Reserved trace field: D2 v0.9.0 hierarchy assignment has no RNG input,
    /// so this remains zero for every decision.
    pub rng_draws_before: u64,
    /// Reserved trace field: D2 v0.9.0 hierarchy assignment has no RNG input,
    /// so this remains zero for every decision.
    pub rng_draws_after: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
/// Serializable observation of one diagnostic layout boundary.
///
/// Snapshots are intended for compatibility analysis. They are not resumable
/// checkpoints and may evolve with the engine's internal stage model.
pub struct LayoutSnapshot {
    /// Requested observation boundary.
    pub stage: LayoutStage,
    /// Working lattice cell size.
    pub cell_size: f64,
    /// Current route-turn cost derived for the active graph.
    pub turn_cost: f64,
    /// Whether the pipeline has enabled ordinary edge routing.
    pub run_edge_route: bool,
    /// Stable node-order snapshot.
    pub nodes: Vec<LayoutNodeState>,
    /// Stable edge-order snapshot.
    pub edges: Vec<LayoutEdgeState>,
    /// Next value in the relevant random stream, when the boundary records a
    /// probe.
    pub next_rng_float: Option<f64>,
    /// Hierarchy placement traces collected so far.
    pub hierarchy_placements: Vec<HierarchyPlacementTrace>,
    /// Automatic hierarchy assignment decisions collected so far.
    pub hierarchy_assignments: Vec<HierarchyAssignmentTrace>,
    /// Lengths of hierarchy input slices shuffled by the hierarchy RNG.
    pub hierarchy_rng_shuffle_lengths: Vec<usize>,
    /// Internal graph state retained for the post-selection candidate pass.
    ///
    /// This is omitted from serialized diagnostics. Keeping the completed
    /// arena beside the public snapshot lets `RaceSeeds` run the v0.9.0
    /// `CompoundCandidate` pass without replaying the hot pipeline (which
    /// would duplicate trace events and consume a different RNG stream).
    #[allow(private_interfaces)]
    #[serde(skip)]
    pub(crate) graph: Option<ArenaGraph>,
    /// Internal placement failure carried through diagnostic snapshots so
    /// production seed execution can return the source-shaped error.
    #[serde(skip)]
    pub(crate) layout_error: Option<crate::LayoutError>,
}

impl LayoutSnapshot {
    /// Failure at an observed pipeline boundary, if layout stopped early.
    ///
    /// Diagnostic adapters must check this before publishing snapshot geometry.
    pub fn layout_error(&self) -> Option<&crate::LayoutError> {
        self.layout_error.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Orientation {
    None,
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

#[derive(Clone, Debug)]
pub(super) struct HerdAssignment {
    pub(super) orientation: Orientation,
    pub(super) value: f64,
    /// Numeric working-copy keys retain canonical owner indices (nil=-1).
    /// Actual pairing-map identity lives on OwnerHerd; TALA entity IDs cannot
    /// identify a node allocation, including when two nodes have the same ID.
    pub(super) same_side_paired: BTreeSet<i64>,
    pub(super) opposite_side_paired: BTreeSet<i64>,
}

/// A node's actual hierarchy pointer plus native placement bookkeeping.
/// `node` is the canonical level-map key retained by dense scope projections;
/// levels and LevelCount are always read from the shared source record.
#[derive(Clone)]
pub(super) struct HierarchyMembership {
    pub(super) id: usize,
    pub(super) scope: Option<NodeId>,
    pub(super) record: super::shared_topology::OwnerHierarchy,
    pub(super) node: NodeId,
}
impl HierarchyMembership {
    pub(super) fn level(&self) -> i64 {
        self.record.level(self.node)
    }
    pub(super) fn level_count(&self) -> i64 {
        self.record.level_count()
    }
}
impl PartialEq for HierarchyMembership {
    fn eq(&self, other: &Self) -> bool {
        self.record.same_allocation(&other.record)
    }
}
impl Eq for HierarchyMembership {}
impl std::fmt::Debug for HierarchyMembership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HierarchyMembership")
            .field("id", &self.id)
            .field("scope", &self.scope)
            .field("level", &self.level())
            .field("level_count", &self.level_count())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TreeRoutingNode {
    pub(super) parent: NodeId,
    pub(super) sentinel_edge: EdgeId,
    pub(super) orientation: Orientation,
    /// Order in the source container slice when the fringe node was peeled.
    /// Go preserves this order in placement-tree children; stable maps do not.
    pub(super) extraction_order: usize,
}

impl Orientation {
    pub(super) fn opposite(self) -> Self {
        match self {
            Self::None => Self::None,
            Self::TopLeft => Self::BottomRight,
            Self::Top => Self::Bottom,
            Self::TopRight => Self::BottomLeft,
            Self::Right => Self::Left,
            Self::BottomRight => Self::TopLeft,
            Self::Bottom => Self::Top,
            Self::BottomLeft => Self::TopRight,
            Self::Left => Self::Right,
        }
    }

    // Direct translation of recovered directionCompass.
    pub(super) fn compass(self) -> i32 {
        match self {
            Self::TopLeft => -1,
            Self::Top => 0,
            Self::TopRight => 1,
            Self::Right => 2,
            Self::BottomRight => 3,
            Self::Bottom => 4,
            Self::BottomLeft => -3,
            Self::Left => -2,
            Self::None => 0,
        }
    }

    pub(super) fn is_diagonal(self) -> bool {
        matches!(
            self,
            Self::TopLeft | Self::TopRight | Self::BottomLeft | Self::BottomRight
        )
    }

    pub(super) fn is_horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    pub(super) fn is_vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }

    pub(super) fn same_side(self, other: Self) -> bool {
        let sides = |orientation| match orientation {
            Self::TopLeft => 0b1001,
            Self::Top => 0b0001,
            Self::TopRight => 0b0011,
            Self::Right => 0b0010,
            Self::BottomRight => 0b0110,
            Self::Bottom => 0b0100,
            Self::BottomLeft => 0b1100,
            Self::Left => 0b1000,
            Self::None => 0,
        };
        sides(self) & sides(other) != 0
    }
}

pub(super) fn compass_delta(a: i32, b: i32) -> i32 {
    let delta = (a - b).abs();
    delta.min(8 - delta)
}

// Direct translation of recovered compassAxisDelta.
pub(super) fn compass_axis_delta(c1: i32, c2: i32) -> f64 {
    let c1_axis = c1 - ((c1 + 4) & -4);
    let c2_axis = c2 - ((c2 + 4) & -4);
    let mut delta = c2_axis - c1_axis;
    if delta == 3 {
        delta = -1;
    }
    f64::from(delta).abs()
}

pub(super) fn layout_orientation(direction: Direction) -> Orientation {
    match direction {
        Direction::Right => Orientation::Right,
        Direction::Left => Orientation::Left,
        Direction::Down => Orientation::Bottom,
        Direction::Up => Orientation::Top,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    Prescale,
    PreprocessSequences,
    Preprocess,
    PreprocessTrees,
    PreprocessContainers,
    PreprocessHierarchies,
    PreprocessClusters,
    PreprocessHubs,
    NodePlacement,
    SwapStuff,
    Transpose,
    AlignAxes,
    GapNormalization,
    OptimizeClusters,
    BalanceSymmetry,
    Equidistance,
    BinPack,
    CleanupStuff,
    Rescale,
    EdgeRouting,
    Crosshatch,
    Dejitter,
    SimplifyEdgeRoutes,
    SwapEdgePorts,
    StraightEdgesFallback,
    BalanceEdgeSegments,
    FixClusterEdgeBranching,
    TraceEdgesToShapeBorder,
    ReorderDuplicates,
    PlaceLabels,
    Normalize,
}

impl Stage {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Prescale => "Prescale",
            Self::PreprocessSequences => "PreprocessSequences",
            Self::Preprocess => "Preprocess",
            Self::PreprocessTrees => "PreprocessTrees",
            Self::PreprocessContainers => "PreprocessContainers",
            Self::PreprocessHierarchies => "PreprocessHierarchies",
            Self::PreprocessClusters => "PreprocessClusters",
            Self::PreprocessHubs => "PreprocessHubs",
            Self::NodePlacement => "NodePlacement",
            Self::SwapStuff => "SwapStuff",
            Self::Transpose => "Transpose",
            Self::AlignAxes => "AlignAxes",
            Self::GapNormalization => "GapNormalization",
            Self::OptimizeClusters => "OptimizeClusters",
            Self::BalanceSymmetry => "BalanceSymmetry",
            Self::Equidistance => "Equidistance",
            Self::BinPack => "BinPack",
            Self::CleanupStuff => "CleanupStuff",
            Self::Rescale => "Rescale",
            Self::EdgeRouting => "EdgeRouting",
            Self::Crosshatch => "Crosshatch",
            Self::Dejitter => "Dejitter",
            Self::SimplifyEdgeRoutes => "SimplifyEdgeRoutes",
            Self::SwapEdgePorts => "SwapEdgePorts",
            Self::StraightEdgesFallback => "StraightEdgesFallback",
            Self::BalanceEdgeSegments => "BalanceEdgeSegments",
            Self::FixClusterEdgeBranching => "FixClusterEdgeBranching",
            Self::TraceEdgesToShapeBorder => "TraceEdgesToShapeBorder",
            Self::ReorderDuplicates => "ReorderDuplicates",
            Self::PlaceLabels => "PlaceLabels",
            Self::Normalize => "Normalize",
        }
    }
}

// Exact order recovered from NewPipeline in dwarf_pipeline.go.
pub(super) const STAGES: &[Stage] = &[
    Stage::Prescale,
    Stage::PreprocessSequences,
    Stage::Preprocess,
    Stage::PreprocessTrees,
    Stage::PreprocessContainers,
    Stage::PreprocessHierarchies,
    Stage::PreprocessClusters,
    Stage::PreprocessHubs,
    Stage::NodePlacement,
    Stage::SwapStuff,
    Stage::Transpose,
    Stage::AlignAxes,
    Stage::GapNormalization,
    Stage::AlignAxes,
    Stage::OptimizeClusters,
    Stage::AlignAxes,
    Stage::BalanceSymmetry,
    Stage::Equidistance,
    Stage::AlignAxes,
    Stage::BinPack,
    Stage::CleanupStuff,
    Stage::Rescale,
    Stage::EdgeRouting,
    Stage::Crosshatch,
    Stage::Dejitter,
    Stage::EdgeRouting,
    Stage::SimplifyEdgeRoutes,
    Stage::SwapEdgePorts,
    Stage::StraightEdgesFallback,
    Stage::BalanceEdgeSegments,
    Stage::FixClusterEdgeBranching,
    Stage::TraceEdgesToShapeBorder,
    Stage::ReorderDuplicates,
    Stage::BinPack,
    Stage::PlaceLabels,
    Stage::Normalize,
];

// Laid out by hand for the placement scans: the first 64-byte line holds
// the box (position, rect) and the shape and container flags; the second the
// spacing inputs (margins, loop offsets, incident edges); then the identity
// and hierarchy fields. The remaining fields follow by alignment.
#[repr(C, align(64))]
pub(super) struct ArenaNode {
    pub position: Option<Point>,
    pub rect: Rect,
    pub shape: ShapeKind,
    pub is_container: bool,
    // `Graph.placeNodes` inserts the original Go node pointers into each
    // temporary children graph. Their container bit and original parent
    // pointer therefore survive even though that graph owns no nested
    // hierarchy. Keep this scoring identity separate from `is_container`,
    // which controls the Rust arena's actual hierarchy behavior.
    pub(super) scoring_is_container: bool,
    /// The temporary scope node represents an active cluster or sequence
    /// vessel. TALA's `Node.getSymmetry` does not restore edge-abduction
    /// endpoints through either kind of vessel.
    pub(super) scoring_is_aggregate_vessel: bool,
    /// Recovered `Node.IsInvisible`. Invisible nodes still own ports and a
    /// center OVG node, but that center is not marked `IsNodeCenter`.
    pub is_invisible: bool,
    /// Source label text can be explicitly empty while measured dimensions
    /// remain present. Routing excludes those labels from obstacle scans.
    pub(super) label_text_present: bool,
    pub force_hierarchy: bool,
    pub has_icon: bool,
    pub layout_margins: Insets,
    /// TALA `Node.LoopOffsets`, populated by `Pipeline.PreprocessStage`.
    pub(super) loop_offsets: Option<LoopOffsetsMap>,
    pub edges: IncidentEdges,
    pub tala_id: i64,
    pub container: Option<NodeId>,
    pub(super) cluster: Option<usize>,
    pub(super) sequence: Option<usize>,
    /// Original Go `Node.Container` chain retained by temporary child graphs.
    ///
    /// SplitSubgraphs owns only a subset of `Graph.Nodes`, while transactions
    /// still validate every entry in the shared `Graph.Containers` map.  The
    /// chain is therefore needed to distinguish an external obstacle from an
    /// ancestor that `Graph.IsBadState` places in its exception set.
    pub(super) scoring_container_ancestors: Vec<i64>,
    /// Stable D2-visible debug identity used by diagnostic traces. Keep this
    /// beside the numeric TALA identity so temporary placement scopes can
    /// emit the same IDs after the owning Graph has been projected away.
    pub external_id: String,
    /// Adapter-set D2ID pointer used by source errors; absent for raw TALA nodes.
    pub d2_id: Option<String>,
    pub declared_size: Option<Size>,
    /// Serialized position hint used by order-sensitive preprocessing. This
    /// is intentionally separate from `fixed_top_left` and does not constrain
    /// placement.
    pub initial_position: Option<Point>,
    /// Coordinate translation introduced when Rust copies a completed child
    /// graph beneath a carrier that TALA still leaves unpositioned. External
    /// transaction views subtract this translation until the parent graph
    /// gives the carrier a real position.
    pub(super) unpositioned_scope_translation: Option<Point>,
    /// Sole actual nullable Near-map authority. Numerical Clone detaches;
    /// explicit retained headers and cursors preserve source map aliases.
    pub nears: super::node_near_map::NodeNearMap,
    pub(super) herd_assignment: Option<HerdAssignment>,
    pub fixed_top_left: Option<Point>,
    pub(super) hierarchy: Option<HierarchyMembership>,
    pub desired_width: Option<f64>,
    pub desired_height: Option<f64>,
    /// Minimum `wrapChildren` box implied by D2 label geometry that the
    /// flattened adapter has already folded into this arranged node.
    pub folded_label_min_size: Option<Size>,
    pub label_size: Option<Size>,
    pub(super) scoring_container_parent: Option<i64>,
    /// Temporary vessel that owns this external cluster member during a
    /// placement-scope transaction.
    pub(super) scoring_cluster_vessel: Option<i64>,
    pub(super) transaction_anchor_tala_id: Option<i64>,
    pub(super) transaction_anchor_offset: Option<Point>,
    pub content_insets: Insets,
    /// TALA `Node.padding`, populated once by `Node.UpdateSpacing` from fixed
    /// inside label and icon positions. Unlike `content_insets`, this is not a
    /// snapshot of a particular `getContainerPadding` result.
    pub(super) node_padding: Insets,
    pub grid_rows: Option<usize>,
    pub grid_columns: Option<usize>,
    pub external_label: Option<ExternalLabel>,
    /// D2 SQL-table row count used by TALA's temporary hierarchy children.
    pub(super) table_column_count: Option<usize>,
    pub input_id: NodeId,
    pub font_size: Option<u32>,
    /// Nearest node represented in the temporary placement graph that carries
    /// this external container. TALA keeps shared node pointers, so moving the
    /// represented ancestor also moves the external descendant. The flattened
    /// adapter retains that relationship as an anchor plus relative offset.
    /// Canonical node backing a projected value snapshot. This is supplied
    /// during scope induction, before local NodeIds are remapped; entity IDs
    /// cannot establish pointer identity.
    pub(super) owner_geometry_source: Option<NodeId>,
    pub(super) owner_geometry_anchor: Option<NodeId>,
    /// The completed child scope has published its shared-pointer frame. The
    /// translation remains available for the later carrier copy-back, but it
    /// must not be subtracted again from external transaction projections.
    pub(super) scope_translation_materialized: bool,
    /// D2 sequence-diagram scope marker retained for diagnostic projection.
    /// This is separate from TALA's sequence aggregate carrier above.
    pub(super) is_sequence_diagram: bool,
    pub label_position: LabelPosition,
    /// TALA's private `Label.isPositionFixed` bit. Automatically synthesized
    /// outside labels remain eligible for `Graph.PlaceLabels`.
    pub label_position_fixed: bool,
    /// Upstream Icon.positionFixed, separate from its selected position.
    pub icon_position_fixed: bool,
    /// Arrangement of a temporary TALA cluster vessel carried by this stable
    /// Rust node. `Node.edgeLength` gives such vessels a distinct direction.
    pub(super) scoring_cluster_arrangement: Option<ClusterArrangement>,
    /// The projection bridge may need a concrete coordinate for a shared Go
    /// pointer whose `Box.TopLeft` is still nil. Preserve that logical nilness
    /// after anchor materialization so lifecycle code such as
    /// `Cluster.ArrangeClusterNodes` can take its recovered nil branch once.
    pub(super) transaction_position_was_nil: bool,
    /// This external carrier participates in overlap validation but must not
    /// be refit by Transaction.repositionContainers during speculative moves.
    pub(super) transaction_validation_only: bool,
    pub canvas_position: Option<CanvasPosition>,
    pub label_aware_grid: bool,
    pub packed_grid: bool,
    pub is_3d: bool,
    pub is_multiple: bool,
    pub icon_position: Option<LabelPosition>,
}

// Debug keeps the declaration order this struct had before its fields were
// laid out for cache locality, so diagnostic output is unchanged.
impl std::fmt::Debug for ArenaNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArenaNode")
            .field("label_text_present", &self.label_text_present)
            .field("input_id", &self.input_id)
            .field("external_id", &self.external_id)
            .field("d2_id", &self.d2_id)
            .field("tala_id", &self.tala_id)
            .field("rect", &self.rect)
            .field("declared_size", &self.declared_size)
            .field("layout_margins", &self.layout_margins)
            .field("position", &self.position)
            .field("initial_position", &self.initial_position)
            .field(
                "unpositioned_scope_translation",
                &self.unpositioned_scope_translation,
            )
            .field(
                "scope_translation_materialized",
                &self.scope_translation_materialized,
            )
            .field("edges", &self.edges)
            .field("loop_offsets", &self.loop_offsets)
            .field("nears", &self.nears)
            .field("herd_assignment", &self.herd_assignment)
            .field("container", &self.container)
            .field("fixed_top_left", &self.fixed_top_left)
            .field("force_hierarchy", &self.force_hierarchy)
            .field("hierarchy", &self.hierarchy)
            .field("sequence", &self.sequence)
            .field("cluster", &self.cluster)
            .field("is_sequence_diagram", &self.is_sequence_diagram)
            .field("desired_width", &self.desired_width)
            .field("desired_height", &self.desired_height)
            .field("folded_label_min_size", &self.folded_label_min_size)
            .field("label_size", &self.label_size)
            .field("font_size", &self.font_size)
            .field("label_position", &self.label_position)
            .field("label_position_fixed", &self.label_position_fixed)
            .field("icon_position_fixed", &self.icon_position_fixed)
            .field("shape", &self.shape)
            .field("is_invisible", &self.is_invisible)
            .field("is_container", &self.is_container)
            .field("scoring_is_container", &self.scoring_is_container)
            .field("scoring_container_parent", &self.scoring_container_parent)
            .field(
                "scoring_cluster_arrangement",
                &self.scoring_cluster_arrangement,
            )
            .field(
                "scoring_is_aggregate_vessel",
                &self.scoring_is_aggregate_vessel,
            )
            .field("scoring_cluster_vessel", &self.scoring_cluster_vessel)
            .field(
                "scoring_container_ancestors",
                &self.scoring_container_ancestors,
            )
            .field("owner_geometry_source", &self.owner_geometry_source)
            .field("owner_geometry_anchor", &self.owner_geometry_anchor)
            .field(
                "transaction_anchor_tala_id",
                &self.transaction_anchor_tala_id,
            )
            .field("transaction_anchor_offset", &self.transaction_anchor_offset)
            .field(
                "transaction_position_was_nil",
                &self.transaction_position_was_nil,
            )
            .field(
                "transaction_validation_only",
                &self.transaction_validation_only,
            )
            .field("content_insets", &self.content_insets)
            .field("node_padding", &self.node_padding)
            .field("grid_rows", &self.grid_rows)
            .field("grid_columns", &self.grid_columns)
            .field("canvas_position", &self.canvas_position)
            .field("label_aware_grid", &self.label_aware_grid)
            .field("packed_grid", &self.packed_grid)
            .field("is_3d", &self.is_3d)
            .field("is_multiple", &self.is_multiple)
            .field("external_label", &self.external_label)
            .field("icon_position", &self.icon_position)
            .field("has_icon", &self.has_icon)
            .field("table_column_count", &self.table_column_count)
            .finish()
    }
}

// Field-wise derived Clone. `clone_from` is the derived `*self = source.clone()`
// assigned field by field: owned strings and the ancestor chain reuse their
// buffers, and edge and near-map storage is refilled in place only where no
// other handle can observe it (see their clone_from).
impl Clone for ArenaNode {
    fn clone(&self) -> Self {
        self.clone_with_buffers(
            self.external_id.clone(),
            self.d2_id.clone(),
            self.scoring_container_ancestors.clone(),
            self.edges.clone(),
            self.nears.clone(),
        )
    }

    fn clone_from(&mut self, source: &Self) {
        let Self {
            label_text_present,
            input_id,
            external_id,
            d2_id,
            tala_id,
            rect,
            declared_size,
            layout_margins,
            position,
            initial_position,
            unpositioned_scope_translation,
            scope_translation_materialized,
            edges,
            loop_offsets,
            nears,
            herd_assignment,
            container,
            fixed_top_left,
            force_hierarchy,
            hierarchy,
            sequence,
            cluster,
            is_sequence_diagram,
            desired_width,
            desired_height,
            folded_label_min_size,
            label_size,
            font_size,
            label_position,
            label_position_fixed,
            icon_position_fixed,
            shape,
            is_invisible,
            is_container,
            scoring_is_container,
            scoring_container_parent,
            scoring_cluster_arrangement,
            scoring_is_aggregate_vessel,
            scoring_cluster_vessel,
            scoring_container_ancestors,
            owner_geometry_source,
            owner_geometry_anchor,
            transaction_anchor_tala_id,
            transaction_anchor_offset,
            transaction_position_was_nil,
            transaction_validation_only,
            content_insets,
            node_padding,
            grid_rows,
            grid_columns,
            canvas_position,
            label_aware_grid,
            packed_grid,
            is_3d,
            is_multiple,
            external_label,
            icon_position,
            has_icon,
            table_column_count,
        } = self;
        *label_text_present = source.label_text_present;
        *input_id = source.input_id;
        external_id.clone_from(&source.external_id);
        d2_id.clone_from(&source.d2_id);
        *tala_id = source.tala_id;
        *rect = source.rect;
        *declared_size = source.declared_size;
        *layout_margins = source.layout_margins;
        *position = source.position;
        *initial_position = source.initial_position;
        *unpositioned_scope_translation = source.unpositioned_scope_translation;
        *scope_translation_materialized = source.scope_translation_materialized;
        edges.clone_from(&source.edges);
        loop_offsets.clone_from(&source.loop_offsets);
        nears.clone_from(&source.nears);
        herd_assignment.clone_from(&source.herd_assignment);
        *container = source.container;
        *fixed_top_left = source.fixed_top_left;
        *force_hierarchy = source.force_hierarchy;
        hierarchy.clone_from(&source.hierarchy);
        *sequence = source.sequence;
        *cluster = source.cluster;
        *is_sequence_diagram = source.is_sequence_diagram;
        *desired_width = source.desired_width;
        *desired_height = source.desired_height;
        *folded_label_min_size = source.folded_label_min_size;
        *label_size = source.label_size;
        *font_size = source.font_size;
        *label_position = source.label_position;
        *label_position_fixed = source.label_position_fixed;
        *icon_position_fixed = source.icon_position_fixed;
        *shape = source.shape;
        *is_invisible = source.is_invisible;
        *is_container = source.is_container;
        *scoring_is_container = source.scoring_is_container;
        *scoring_container_parent = source.scoring_container_parent;
        *scoring_cluster_arrangement = source.scoring_cluster_arrangement;
        *scoring_is_aggregate_vessel = source.scoring_is_aggregate_vessel;
        *scoring_cluster_vessel = source.scoring_cluster_vessel;
        scoring_container_ancestors.clone_from(&source.scoring_container_ancestors);
        *owner_geometry_source = source.owner_geometry_source;
        *owner_geometry_anchor = source.owner_geometry_anchor;
        *transaction_anchor_tala_id = source.transaction_anchor_tala_id;
        *transaction_anchor_offset = source.transaction_anchor_offset;
        *transaction_position_was_nil = source.transaction_position_was_nil;
        *transaction_validation_only = source.transaction_validation_only;
        *content_insets = source.content_insets;
        *node_padding = source.node_padding;
        *grid_rows = source.grid_rows;
        *grid_columns = source.grid_columns;
        *canvas_position = source.canvas_position;
        *label_aware_grid = source.label_aware_grid;
        *packed_grid = source.packed_grid;
        *is_3d = source.is_3d;
        *is_multiple = source.is_multiple;
        *external_label = source.external_label;
        *icon_position = source.icon_position;
        *has_icon = source.has_icon;
        *table_column_count = source.table_column_count;
    }
}

impl ArenaNode {
    /// A clone whose edge and near-map handles are supplied by the caller.
    /// Projections that immediately replace both fields use this instead of
    /// forking storage that would be discarded.
    pub(super) fn clone_with_handles(
        &self,
        edges: IncidentEdges,
        nears: super::node_near_map::NodeNearMap,
    ) -> Self {
        self.clone_with_buffers(
            self.external_id.clone(),
            self.d2_id.clone(),
            self.scoring_container_ancestors.clone(),
            edges,
            nears,
        )
    }

    fn clone_with_buffers(
        &self,
        external_id: String,
        d2_id: Option<String>,
        scoring_container_ancestors: Vec<i64>,
        edges: IncidentEdges,
        nears: super::node_near_map::NodeNearMap,
    ) -> Self {
        Self {
            label_text_present: self.label_text_present,
            input_id: self.input_id,
            external_id,
            d2_id,
            tala_id: self.tala_id,
            rect: self.rect,
            declared_size: self.declared_size,
            layout_margins: self.layout_margins,
            position: self.position,
            initial_position: self.initial_position,
            unpositioned_scope_translation: self.unpositioned_scope_translation,
            scope_translation_materialized: self.scope_translation_materialized,
            edges,
            loop_offsets: self.loop_offsets.clone(),
            nears,
            herd_assignment: self.herd_assignment.clone(),
            container: self.container,
            fixed_top_left: self.fixed_top_left,
            force_hierarchy: self.force_hierarchy,
            hierarchy: self.hierarchy.clone(),
            sequence: self.sequence,
            cluster: self.cluster,
            is_sequence_diagram: self.is_sequence_diagram,
            desired_width: self.desired_width,
            desired_height: self.desired_height,
            folded_label_min_size: self.folded_label_min_size,
            label_size: self.label_size,
            font_size: self.font_size,
            label_position: self.label_position,
            label_position_fixed: self.label_position_fixed,
            icon_position_fixed: self.icon_position_fixed,
            shape: self.shape,
            is_invisible: self.is_invisible,
            is_container: self.is_container,
            scoring_is_container: self.scoring_is_container,
            scoring_container_parent: self.scoring_container_parent,
            scoring_cluster_arrangement: self.scoring_cluster_arrangement,
            scoring_is_aggregate_vessel: self.scoring_is_aggregate_vessel,
            scoring_cluster_vessel: self.scoring_cluster_vessel,
            scoring_container_ancestors,
            owner_geometry_source: self.owner_geometry_source,
            owner_geometry_anchor: self.owner_geometry_anchor,
            transaction_anchor_tala_id: self.transaction_anchor_tala_id,
            transaction_anchor_offset: self.transaction_anchor_offset,
            transaction_position_was_nil: self.transaction_position_was_nil,
            transaction_validation_only: self.transaction_validation_only,
            content_insets: self.content_insets,
            node_padding: self.node_padding,
            grid_rows: self.grid_rows,
            grid_columns: self.grid_columns,
            canvas_position: self.canvas_position,
            label_aware_grid: self.label_aware_grid,
            packed_grid: self.packed_grid,
            is_3d: self.is_3d,
            is_multiple: self.is_multiple,
            external_label: self.external_label,
            icon_position: self.icon_position,
            has_icon: self.has_icon,
            table_column_count: self.table_column_count,
        }
    }
}

// Laid out by hand: the spacing scan's per-edge reads (endpoints and
// minimum sizes) share the first line. The remaining fields follow by
// alignment.
#[derive(Clone)]
#[repr(C, align(64))]
pub(super) struct ArenaEdge {
    pub min_width: f64,
    pub min_height: f64,
    /// Signed source EntityID; never an index into arena storage.
    pub tala_id: i64,
    pub route: RoutePoints,
    pub from: NodeId,
    pub to: NodeId,
    pub input_id: EdgeId,
    pub source_arrowhead: Option<String>,
    pub target_arrowhead: Option<String>,
    pub label: Option<EdgeLabel>,
    pub source_arrowhead_label: Option<ArrowheadLabel>,
    pub target_arrowhead_label: Option<ArrowheadLabel>,
    pub style: crate::EdgeStyle,
    pub source_table_column: Option<usize>,
    pub target_table_column: Option<usize>,
    /// SQL-table row inventories belong to the original serialized
    /// endpoints.  Temporary hierarchy graphs can reconnect the same edge to
    /// a container vessel, so retaining them on the arena edge keeps
    /// `getFacingTablePorts` faithful when an edge abduction supplies the
    /// original endpoint boxes.
    pub source_table_column_count: Option<usize>,
    pub target_table_column_count: Option<usize>,
    /// Stored upstream IsInvisible flag, independent of later style changes.
    pub invisible: bool,
    /// Upstream Label.positionFixed; independent of the position enum.
    pub label_position_fixed: bool,
    /// Direct GraphJSON label positions, including arrowhead labels.
    pub source_label_positions: [Option<LabelPosition>; 3],
    /// Whether this route is explicitly marked as a cubic curve by GraphJSON.
    pub is_curve: bool,
    pub source_arrow: bool,
    pub target_arrow: bool,
}

// Debug keeps the declaration order this struct had before its fields were
// laid out for cache locality, so diagnostic output is unchanged.
impl std::fmt::Debug for ArenaEdge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArenaEdge")
            .field("input_id", &self.input_id)
            .field("tala_id", &self.tala_id)
            .field("invisible", &self.invisible)
            .field("label_position_fixed", &self.label_position_fixed)
            .field("source_label_positions", &self.source_label_positions)
            .field("from", &self.from)
            .field("to", &self.to)
            .field("route", &self.route)
            .field("is_curve", &self.is_curve)
            .field("source_arrow", &self.source_arrow)
            .field("target_arrow", &self.target_arrow)
            .field("source_arrowhead", &self.source_arrowhead)
            .field("target_arrowhead", &self.target_arrowhead)
            .field("label", &self.label)
            .field("source_arrowhead_label", &self.source_arrowhead_label)
            .field("target_arrowhead_label", &self.target_arrowhead_label)
            .field("style", &self.style)
            .field("source_table_column", &self.source_table_column)
            .field("target_table_column", &self.target_table_column)
            .field("source_table_column_count", &self.source_table_column_count)
            .field("target_table_column_count", &self.target_table_column_count)
            .field("min_width", &self.min_width)
            .field("min_height", &self.min_height)
            .finish()
    }
}

impl ArenaEdge {
    pub(super) fn is_invisible(&self) -> bool {
        self.invisible
    }

    pub(super) fn has_table_column(&self) -> bool {
        self.source_table_column.is_some() || self.target_table_column.is_some()
    }

    pub(super) fn is_between_table_columns(&self) -> bool {
        self.source_table_column.is_some() && self.target_table_column.is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ClusterArrangement {
    Row,
    Column,
}

#[derive(Clone, Debug)]
pub(super) struct ClusterState {
    pub(super) members: Vec<NodeId>,
    pub(super) arrangement: ClusterArrangement,
    pub(super) desired_arrangement: ClusterArrangement,
    pub(super) padding: f64,
    pub(super) vessel_tala_id: i64,
    pub(super) fixed_size: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ExternalClusterLayout {
    pub(super) arrangement: ClusterArrangement,
    pub(super) padding: f64,
    pub(super) fixed_size: bool,
}

#[derive(Clone, Debug)]
pub(super) struct SequenceState {
    pub(super) members: Vec<NodeId>,
    pub(super) vessel_tala_id: i64,
    pub(super) container: Option<NodeId>,
    /// `Sequence.abductEdges` records every edge with exactly one endpoint in
    /// the sequence. `Graph.direct` refuses to mirror while such a sequence
    /// is active because its vessel/member edge ownership is transitional.
    pub(super) has_edge_abductions: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectedAdjacent {
    pub(super) owner: NodeId,
    pub(super) tala_id: i64,
    pub(super) container_tala_id: Option<i64>,
    pub(super) offset: Point,
    pub(super) size: Size,
    /// TALA `Node.edgeLength` measures an abducted cluster member from its
    /// vessel while retaining the member box for orientation/routing.
    pub(super) cluster_member: bool,
}

impl ProjectedAdjacent {
    fn same_bits(self, other: Self) -> bool {
        self.owner == other.owner
            && self.tala_id == other.tala_id
            && self.container_tala_id == other.container_tala_id
            && self.offset.x.to_bits() == other.offset.x.to_bits()
            && self.offset.y.to_bits() == other.offset.y.to_bits()
            && self.size.width.to_bits() == other.size.width.to_bits()
            && self.size.height.to_bits() == other.size.height.to_bits()
            && self.cluster_member == other.cluster_member
    }
}

/// A retained original Go node pointer, independently of its nullable TopLeft.
/// The optional carrier is local; source_node stays in the owning arena across induced
/// graphs. Missing geometry never turns this node reference into a nil pointer.
#[derive(Clone, Copy, Debug)]
pub(super) struct AbductedNodeRef {
    pub(super) source_node: Option<NodeId>,
    pub(super) source_owner: Option<NodeId>,
    pub(super) owner_tala_id: Option<i64>,
    pub(super) owner: Option<NodeId>,
    pub(super) tala_id: i64,
    pub(super) container_tala_id: Option<i64>,
    pub(super) offset: Option<Point>,
    pub(super) size: Size,
    pub(super) cluster_member: bool,
}

impl AbductedNodeRef {
    pub(super) fn projection(self) -> Option<ProjectedAdjacent> {
        Some(ProjectedAdjacent {
            owner: self.owner?,
            tala_id: self.tala_id,
            container_tala_id: self.container_tala_id,
            offset: self.offset?,
            size: self.size,
            cluster_member: self.cluster_member,
        })
    }
}

impl From<ProjectedAdjacent> for AbductedNodeRef {
    fn from(projected: ProjectedAdjacent) -> Self {
        Self {
            // Authored projections do not provide an owning-arena pointer.
            source_node: None,
            source_owner: None,
            owner_tala_id: None,
            owner: Some(projected.owner),
            tala_id: projected.tala_id,
            container_tala_id: projected.container_tala_id,
            offset: Some(projected.offset),
            size: projected.size,
            cluster_member: projected.cluster_member,
        }
    }
}

/// One entry from an abducted SQL table's surviving `Node.Edges` slice.
///
/// `Graph.abductEdges` reconnects cross-container edges onto a direct-child
/// carrier, but same-child descendant edges remain attached to their original
/// endpoint pointers. `Node.edgeLength` later walks that original slice when
/// it restores an `EdgeAbduction`, so the temporary arena must retain both
/// the remote table geometry and the column index at the restored table.
#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectedTableColumnNeighbor {
    pub(super) other: ProjectedAdjacent,
    pub(super) column_index: usize,
}

#[derive(Clone, Debug)]
pub(super) struct ProjectedClusterDistance {
    pub(super) offset: Point,
    pub(super) size: Size,
    pub(super) arrangement: ClusterArrangement,
    pub(super) vessel_tala_id: i64,
    /// Recovered `Cluster.getExternalConnectedNodes` inventory retained from
    /// the cluster's own edge-abduction layer. These endpoints can be hidden
    /// inside the current parent carrier and therefore need not occur among
    /// the parent placement graph's projected edges.
    pub(super) external_connected: Vec<ProjectedAdjacent>,
}

/// A Go layout node pointer restored from a graphjson aggregate record.
///
/// Ordinary graph nodes use their stable arena identity. Aggregate vessels
/// are separate objects in Go and therefore retain their signed identity even
/// when they are absent from the owning Graph.Nodes slice. Unknown values are
/// retained defensively for programmatically constructed adapter metadata;
/// graphjson preflight rejects them before this boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum AggregateNodeRef {
    GraphNode { node: NodeId, tala_id: i64 },
    Vessel { tala_id: i64 },
    Unknown { tala_id: i64 },
}

impl AggregateNodeRef {
    pub(super) fn tala_id(self) -> i64 {
        match self {
            Self::GraphNode { tala_id, .. }
            | Self::Vessel { tala_id }
            | Self::Unknown { tala_id } => tala_id,
        }
    }

    pub(super) fn graph_node(self) -> Option<NodeId> {
        match self {
            Self::GraphNode { node, .. } => Some(node),
            Self::Vessel { .. } | Self::Unknown { .. } => None,
        }
    }
}

/// A hydrated Go `EdgeAbduction`: its edge and node identities have been
/// resolved once at the graphjson publication boundary, while aggregate
/// vessels remain distinct from their stable member carriers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ResolvedAggregateEdgeAbduction {
    pub(super) edge_tala_id: i64,
    pub(super) edge: Option<EdgeId>,
    pub(super) originally_from: Option<AggregateNodeRef>,
    pub(super) originally_to: Option<AggregateNodeRef>,
    pub(super) current_from: Option<AggregateNodeRef>,
    pub(super) current_to: Option<AggregateNodeRef>,
}

/// A nonnil current endpoint of an owning EdgeAbduction. The outer Option
/// retains true nil; Foreign never aliases a dense node in an induced graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AbductionNodeLocation {
    Local(NodeId),
    Foreign,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AbductionCurrentNodeRef {
    pub(super) location: AbductionNodeLocation,
    pub(super) source_node: Option<NodeId>,
    pub(super) tala_id: Option<i64>,
    pub(super) container_tala_id: Option<i64>,
    pub(super) position: Option<Point>,
    pub(super) size: Option<Size>,
    pub(super) is_aggregate_vessel: bool,
}

impl AbductionCurrentNodeRef {
    pub(super) fn local(node: NodeId) -> Self {
        Self {
            location: AbductionNodeLocation::Local(node),
            source_node: None,
            tala_id: None,
            container_tala_id: None,
            position: None,
            size: None,
            is_aggregate_vessel: false,
        }
    }

    pub(super) fn local_with_source(node: NodeId, source_node: NodeId) -> Self {
        Self {
            source_node: Some(source_node),
            ..Self::local(node)
        }
    }

    pub(super) fn local_node(self) -> Option<NodeId> {
        match self.location {
            AbductionNodeLocation::Local(node) => Some(node),
            AbductionNodeLocation::Foreign => None,
        }
    }
}

/// Numerical views of Go's shared obstruction pointers. A rollback snapshot
/// retains the same values until a candidate actually changes a projection.
#[derive(Clone, Debug, Default)]
pub(super) struct SharedProjections(Arc<Vec<ProjectedAdjacent>>);

impl SharedProjections {
    pub(super) fn update_projections(&mut self, mut update: impl FnMut(&mut ProjectedAdjacent)) {
        if let Some(values) = Arc::get_mut(&mut self.0) {
            for projection in values {
                update(projection);
            }
            return;
        }
        // Evaluate each update once, including diagnostic observations. A
        // no-op reconciliation must keep the shared allocation intact.
        for index in 0..self.len() {
            let original = self[index];
            let mut projection = original;
            update(&mut projection);
            if !projection.same_bits(original) {
                Arc::make_mut(&mut self.0)[index] = projection;
            }
        }
    }
}

impl From<Vec<ProjectedAdjacent>> for SharedProjections {
    fn from(values: Vec<ProjectedAdjacent>) -> Self {
        Self(Arc::new(values))
    }
}

impl FromIterator<ProjectedAdjacent> for SharedProjections {
    fn from_iter<T: IntoIterator<Item = ProjectedAdjacent>>(values: T) -> Self {
        values.into_iter().collect::<Vec<_>>().into()
    }
}

impl std::ops::Deref for SharedProjections {
    type Target = Vec<ProjectedAdjacent>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for SharedProjections {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

impl<'a> IntoIterator for &'a SharedProjections {
    type Item = &'a ProjectedAdjacent;
    type IntoIter = std::slice::Iter<'a, ProjectedAdjacent>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod projection_snapshot_tests {
    use super::*;

    #[test]
    fn obstruction_snapshots_share_unchanged_values_and_isolate_changed_bits() {
        let nan = f64::from_bits(0x7ff8_0000_0000_1234);
        let mut projections: SharedProjections = vec![ProjectedAdjacent {
            owner: NodeId(0),
            tala_id: 17,
            container_tala_id: None,
            offset: Point { x: -0.0, y: nan },
            size: Size {
                width: 10.0,
                height: 20.0,
            },
            cluster_member: false,
        }]
        .into();
        let rollback = projections.clone();
        projections.update_projections(|_| {});
        assert!(Arc::ptr_eq(&projections.0, &rollback.0));
        projections.update_projections(|projection| projection.offset.x = 0.0);
        assert!(!Arc::ptr_eq(&projections.0, &rollback.0));
        assert_eq!(rollback[0].offset.x.to_bits(), (-0.0_f64).to_bits());
        assert_eq!(projections[0].offset.x.to_bits(), 0.0_f64.to_bits());
        assert_eq!(projections[0].offset.y.to_bits(), nan.to_bits());
        assert_eq!(rollback[0].offset.y.to_bits(), nan.to_bits());
        projections.update_projections(|projection| projection.size.width = 30.0);
        assert_eq!(rollback[0].size.width, 10.0);
        assert_eq!(projections[0].size.width, 30.0);
    }
}

/// One nonnil pointer in the source Graph.Containers obstruction slices.
/// Local identifies a geometry carrier, independently of the canonical node
/// pointer; a foreign pointer never aliases a component's dense node index.
#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectedObstructionRef {
    pub(super) source_arena: Option<OwnerArenaId>,
    pub(super) source_node: Option<NodeId>,
    pub(super) source_owner: Option<NodeId>,
    pub(super) location: AbductionNodeLocation,
    pub(super) tala_id: i64,
    pub(super) source_container: Option<NodeId>,
    pub(super) container_tala_id: Option<i64>,
    pub(super) position: Option<Point>,
    pub(super) size: Size,
    pub(super) projection: Option<ProjectedAdjacent>,
    pub(super) graph_owner: Option<OwnerGraphId>,
    /// Receiver ownership follows the retained source carrier closure. It
    /// does not mean membership in the receiver's Graph.Nodes slice.
    pub(super) same_graph_as_replacement: bool,
    pub(super) is_endpoint: bool,
    pub(super) is_replacement_ancestor: bool,
    pub(super) is_cluster_vessel: bool,
    pub(super) is_endpoint_cluster_vessel: bool,
}

#[derive(Debug)]
pub(super) struct ProjectedObstructionSet {
    pub(super) source_container: Option<NodeId>,
    pub(super) container_tala_id: Option<i64>,
    /// None is the actual nil child slice; Some(empty) is allocated empty.
    pub(super) items: Option<Vec<ProjectedObstructionRef>>,
}

// Plain data: clone_from reuses the item buffer of a rollback target.
impl Clone for ProjectedObstructionSet {
    fn clone(&self) -> Self {
        Self {
            source_container: self.source_container,
            container_tala_id: self.container_tala_id,
            items: self.items.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.source_container = source.source_container;
        self.container_tala_id = source.container_tala_id;
        self.items.clone_from(&source.items);
    }
}

#[derive(Debug, Default)]
pub(super) struct OrderedObstructionInventory {
    pub(super) source_arena: Option<OwnerArenaId>,
    pub(super) sets_present: bool,
    pub(super) node_replacement: Option<NodeId>,
    pub(super) adjacent_replacement: Option<NodeId>,
    // Go retains the Graph.Containers slices through geometry transactions
    // and SplitSubgraphs. Share this value snapshot until its projections
    // actually change; rollback clones need only another slice handle.
    pub(super) sets: Arc<[ProjectedObstructionSet]>,
}

// The inventory is a value snapshot. Mutable projection updates detach it.
impl Clone for OrderedObstructionInventory {
    fn clone(&self) -> Self {
        Self {
            source_arena: self.source_arena,
            sets_present: self.sets_present,
            node_replacement: self.node_replacement,
            adjacent_replacement: self.adjacent_replacement,
            sets: self.sets.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.source_arena = source.source_arena;
        self.sets_present = source.sets_present;
        self.node_replacement = source.node_replacement;
        self.adjacent_replacement = source.adjacent_replacement;
        self.sets.clone_from(&source.sets);
    }
}

impl OrderedObstructionInventory {
    pub(super) fn update_projections(&mut self, mut update: impl FnMut(&mut ProjectedAdjacent)) {
        if let Some(sets) = Arc::get_mut(&mut self.sets) {
            for projection in sets
                .iter_mut()
                .filter_map(|set| set.items.as_mut())
                .flatten()
                .filter_map(|item| item.projection.as_mut())
            {
                update(projection);
            }
            return;
        }
        // Reconciliation often writes the same bits. Evaluate each update
        // once in the original order, including its diagnostic observations,
        // and copy the inventory only if it changes a projection.
        for set_index in 0..self.sets.len() {
            let len = self.sets[set_index].items.as_ref().map_or(0, Vec::len);
            for item_index in 0..len {
                let Some(original) =
                    self.sets[set_index].items.as_ref().unwrap()[item_index].projection
                else {
                    continue;
                };
                let mut projection = original;
                update(&mut projection);
                if !projection.same_bits(original) {
                    Arc::make_mut(&mut self.sets)[set_index]
                        .items
                        .as_mut()
                        .unwrap()[item_index]
                        .projection = Some(projection);
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SizedEdgeAbduction {
    /// Source edge lineage in the owning arena. An intra-container edge is
    /// retained for restoration even though it has no temporary graph edge.
    pub(super) source_edge: EdgeId,
    /// Go's Edge.ID, retained independently from a temporary local index.
    pub(super) edge_tala_id: i64,
    pub(super) edge: Option<EdgeId>,
    /// None is a genuinely nil source pointer; Some(Foreign) remains nonnil.
    pub(super) current_from: Option<AbductionCurrentNodeRef>,
    pub(super) current_to: Option<AbductionCurrentNodeRef>,
    pub(super) originally_from: Option<AbductedNodeRef>,
    pub(super) originally_to: Option<AbductedNodeRef>,
    pub(super) originally_from_table_neighbors: Vec<ProjectedTableColumnNeighbor>,
    pub(super) originally_to_table_neighbors: Vec<ProjectedTableColumnNeighbor>,
    /// Owning-arena container keys for original endpoints. They intentionally
    /// remain stable across temporary graphs with dense local node IDs.
    pub(super) originally_from_container: Option<NodeId>,
    pub(super) originally_to_container: Option<NodeId>,
    /// AlignAxes passes only Sequence.EdgeAbductions to Node.edgeLength.
    /// Cluster and ordinary hierarchy abductions remain on the live carrier
    /// endpoints for that objective.
    pub(super) sequence_abduction: bool,
    pub(super) obstructions_from_to: Vec<ProjectedAdjacent>,
    pub(super) obstructions_to_from: Vec<ProjectedAdjacent>,
    pub(super) obstruction_sets_from_to: OrderedObstructionInventory,
    pub(super) obstruction_sets_to_from: OrderedObstructionInventory,
}

#[derive(Clone, Debug)]
pub(super) struct ScopeNodeMetadata {
    pub(super) node: NodeId,
    pub(super) is_container: bool,
    /// The owning-arena equivalent of recovered `Node.getContainer()` while
    /// this node is present in a temporary placement scope. `None` is a real
    /// key: Go's `reachableContainers` map records the nil container for
    /// root-scope nodes.
    pub(super) parent_container: Option<NodeId>,
    pub(super) parent_tala_id: Option<i64>,
    pub(super) container_ancestors: Vec<i64>,
    pub(super) herd_assignment: Option<HerdAssignment>,
    pub(super) tala_id: Option<i64>,
    pub(super) hierarchy: Option<HierarchyMembership>,
    pub(super) position: Option<Point>,
    pub(super) cluster_arrangement: Option<ClusterArrangement>,
    pub(super) is_aggregate_vessel: bool,
    pub(super) node_padding: Insets,
    pub(super) content_insets: Insets,
    pub(super) label_size: Option<Size>,
    pub(super) label_position: LabelPosition,
    pub(super) external_label: Option<ExternalLabel>,
    pub(super) icon_position: Option<LabelPosition>,
}

#[derive(Clone, Debug)]
pub(super) struct ProjectedClusterLayout {
    pub(super) size: Size,
    pub(super) padding: f64,
    pub(super) members: BTreeMap<i64, (Point, Size)>,
}

#[derive(Clone, Debug)]
pub(super) struct ProjectedCluster {
    pub(super) node: NodeId,
    pub(super) cluster_index: usize,
    pub(super) arrangement: ClusterArrangement,
    /// The cluster's requested arrangement before this temporary graph is
    /// optimized. Go's `Cluster.optimize` keeps this value when no positioned
    /// external endpoints are available, so it must be carried separately
    /// from the current arrangement.
    pub(super) desired_arrangement: ClusterArrangement,
    pub(super) row: ProjectedClusterLayout,
    pub(super) column: ProjectedClusterLayout,
}

/// One current edge from a node in the owning graph's `Graph.Nodes` slice.
///
/// Temporary placement graphs do not retain the owning graph's edge IDs, but
/// `Node.getDeltaTo` still observes connectivity and the largest edge spacing
/// dimensions through the shared node pointer.  Stable TALA identities keep
/// that directed receiver-side edge inventory available to transactions.
#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectedTransactionEdge {
    pub(super) adjacent_tala_id: i64,
    pub(super) min_width: f64,
    pub(super) min_height: f64,
}

/// One entry from the owning graph's current `Graph.Nodes` slice.
///
/// The vector containing these entries is ordered exactly like the recovered
/// Go slice. `node` is a fallback snapshot; transaction validation resolves a
/// live box from local/shared-pointer projections before consulting it.
#[derive(Clone, Debug)]
pub(super) struct ProjectedTransactionNode {
    // Structural identities belong to the source arena, not an induced NodeId space.
    pub(super) structural_container_tala_id: Option<i64>,
    pub(super) structural_ancestor_tala_ids: Vec<i64>,
    pub(super) node: ArenaNode,
    pub(super) container_tala_id: Option<i64>,
    pub(super) ancestor_tala_ids: Vec<i64>,
    pub(super) child_tala_ids: Vec<i64>,
    pub(super) edges: Vec<ProjectedTransactionEdge>,
}

#[derive(Clone, Debug)]
pub(super) struct ProjectedTransactionState {
    pub(super) nodes: Vec<ProjectedTransactionNode>,
    pub(super) original_geometry: Vec<(Option<Point>, Size)>,
    pub(super) existing_overlaps: BTreeSet<(usize, usize)>,
    pub(super) existing_exact_overlaps: BTreeSet<(usize, usize)>,
}

/// Readonly scope callers retain the actual source arena and explicit scope
/// arguments. Compact local storage cannot recover original pointer targets.
#[derive(Clone, Debug)]
pub(super) struct PlacementOwnerSource {
    pub(super) input: Graph,
    pub(super) graph: ArenaGraph,
    pub(super) root: Option<NodeId>,
    pub(super) materialized: BTreeSet<Option<NodeId>>,
    pub(super) source_orientation_disabled_for_loop: bool,
}

#[derive(Clone, Debug)]
pub(super) struct PlacementScope {
    pub(super) owner_source: Option<std::sync::Arc<PlacementOwnerSource>>,
    pub(super) graph: Graph,
    /// Fixed sibling boxes passed down by `placeNodes` to both the sized
    /// optimizer and component packing. These stay outside its movable graph.
    pub(super) sized_obstacles: Vec<Rect>,
    /// Source-container context consumed by D2 v0.9.0's optional
    /// `orientSourceInterior` pass after component combination.
    pub(super) source_orientation: SourceOrientationContext,
    /// Full owning graph retained only for source-equivalent placement
    /// snapshots. Component optimizers operate on the direct-child graph,
    /// while Go's placement trace walks the complete owning Graph.Nodes
    /// slice through shared node pointers.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) trace_owner_graph: ArenaGraph,
    /// Direct roots appended to this scope's `childrenGraph`. This is kept
    /// separately from optimizer order because `AddNodeUnchecked` assigns the
    /// node's live Graph owner before placement reorders the children.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) trace_children_graph_node_order: Vec<i64>,
    /// Number of direct children represented by this placement scope.
    pub(super) owner_node_count: usize,
    /// Node count of the complete graph passed to `Graph.placeNodes`.
    pub(super) owning_graph_node_count: usize,
    /// Stable debug identity of the scope root used by the upstream
    /// placement trace. The top-level scope has an empty root.
    pub(super) root_external_id: String,
    /// `Graph.CopyEntitiesFrom` aliases the owning graph's `Directions` map
    /// into every temporary children graph. Stable IDs preserve those pointer
    /// keys even when the keyed container is not materialized in `graph`.
    pub(super) scoring_directions_by_tala: BTreeMap<Option<i64>, Direction>,
    /// Original endpoint row inventories for each projected edge.  The
    /// projected `Graph` carries current carrier nodes, so these cannot be
    /// reconstructed from its node metadata after `Pipeline::new`.
    pub(super) edge_table_column_counts: BTreeMap<EdgeId, (Option<usize>, Option<usize>)>,
    /// Minimum edge dimensions retained when a projected carrier edge is
    /// materialized from the owning arena. These values originate from the
    /// original D2 edge and survive hierarchy reconnection in Go.
    pub(super) projected_edge_minimums: BTreeMap<EdgeId, (f64, f64)>,
    /// Graph.Edges order recovered from the owning Go graph. Temporary Rust
    /// scopes materialize projected and restored-tree edges in separate
    /// passes, so their dense storage order is not itself authoritative.
    pub(super) temporary_edge_order: Vec<EdgeId>,
    /// Canonical source edge identity to its actual temporary storage slot.
    pub(super) edge_old_to_new: BTreeMap<EdgeId, EdgeId>,
    pub(super) old_to_new: BTreeMap<NodeId, NodeId>,
    /// Retained branching-tree nodes are allocated in the temporary graph so
    /// PlaceTrees can reconnect them, but remain absent from Graph.Nodes until
    /// that phase.
    pub(super) tree_auxiliary_nodes: BTreeSet<NodeId>,
    pub(super) tree_routing_nodes: BTreeMap<NodeId, TreeRoutingNode>,
    /// Original retained-tree edge IDs mapped to their temporary placement
    /// graph IDs. `Tree.positionEdgeLabels` mutates those temporary edge
    /// labels before `Graph.placeNodes` copies the trees back to the owner.
    pub(super) tree_edge_old_to_new: BTreeMap<EdgeId, EdgeId>,
    /// Original endpoint reconnections in this scope's abduction order.
    pub(super) restored_endpoint_order: Vec<(NodeId, EdgeId)>,
    /// Endpoint-local edge slices after `abductEdges` retains direct-child
    /// edges and appends edges reconnected from descendants.
    pub(super) incident_edge_order: BTreeMap<NodeId, Vec<EdgeId>>,
    /// `Graph.Hubs` projected from the graph on which TALA ran `AddHubs`.
    ///
    /// `placeNodes` subsequently abducts descendant edges onto direct-child
    /// carriers, but that must not invent new hub/spoke relationships.
    pub(super) hubs: BTreeMap<NodeId, Vec<NodeId>>,
    /// Recovered `placeChildrenOrder` result in the owning arena's IDs.
    ///
    /// TALA derives this only from the current endpoints of edge abductions;
    /// ordinary edges in the temporary children graph do not participate.
    pub(super) child_order: Vec<NodeId>,
    pub(super) assigned_nears: Vec<(NodeId, NodeId)>,
    pub(super) common_uncle_groups: Vec<Vec<NodeId>>,
    pub(super) edge_abduction_nodes: BTreeSet<NodeId>,
    pub(super) sized_adjacent_overrides: Vec<((NodeId, EdgeId), ProjectedAdjacent)>,
    /// Cluster-vessel box retained by an original endpoint inside a projected
    /// descendant carrier, keyed by that carrier and original TALA node ID.
    pub(super) sized_cluster_distance_boxes: BTreeMap<(NodeId, i64), ProjectedClusterDistance>,
    /// `Graph.abductEdges` result in its observable slice order.
    ///
    /// `Node.edgeLength` does not associate these records with their `Edge`
    /// pointer. For each incident edge it consumes the first unused record
    /// whose current endpoint pair matches, so parallel abducted edges can
    /// intentionally receive original endpoints in a different edge-ID
    /// pairing.
    pub(super) sized_edge_abductions: Vec<SizedEdgeAbduction>,
    pub(super) sized_projected_obstructions: BTreeMap<(NodeId, EdgeId), SharedProjections>,
    pub(super) sized_projected_obstruction_sets:
        BTreeMap<(NodeId, EdgeId), OrderedObstructionInventory>,
    /// Original child-to-child edges that collapse onto one carrier and are
    /// therefore absent from the temporary Graph.Edges slice. The original
    /// Go node pointers retain those edges for Node.getSymmetry.
    pub(super) sized_collapsed_symmetry_neighbors: BTreeMap<i64, Vec<ProjectedAdjacent>>,
    /// Original endpoint pairs absent from temporary Graph.Edges because
    /// their current endpoints collapse onto one carrier.
    pub(super) collapsed_restored_mirror_edges:
        Vec<(NodeId, i64, Option<NodeId>, i64, Option<NodeId>)>,
    pub(super) scoring_node_metadata: Vec<ScopeNodeMetadata>,
    /// Original cluster geometry retained behind each temporary vessel.
    ///
    /// `Graph.placeNodes` invokes `Cluster.optimize(ctx, true)` after the
    /// sized optimizer and before directing each SplitSubgraphs result.
    pub(super) projected_clusters: Vec<ProjectedCluster>,
    /// Snapshot-only copy of the complete owning hierarchy. The temporary
    /// `Graph` intentionally contains only direct scope members, while Go's
    /// trace walks shared child pointers retained by the owner graph.
    pub(super) trace_container_children: BTreeMap<i64, Vec<ArenaNode>>,
    pub(super) transaction_external_containers: Vec<ArenaNode>,
    /// Positioned materialized containers used only as overlap-validation
    /// obstacles. They are not moved, wrapped, or included in score inputs.
    pub(super) transaction_validation_external_containers: Vec<ArenaNode>,
    /// Direct child nodes retained by TALA's shared `Containers` map,
    /// including containers whose shared node has no position yet.
    pub(super) transaction_external_container_children: BTreeMap<i64, Vec<ArenaNode>>,
    /// Hidden sequence/cluster members retained solely for the shared-pointer
    /// `moveNodeWithChildren` translation walk. They are not ordinary fitting
    /// or scoring children of the temporary graph.
    pub(super) transaction_external_aggregate_children: BTreeMap<i64, Vec<ArenaNode>>,
    pub(super) transaction_external_cluster_layouts: BTreeMap<i64, ExternalClusterLayout>,
    /// Full current owning-graph `Graph.Nodes` state used by projected
    /// `Transaction.Commit`. Unlike the external Containers projections,
    /// membership in this vector means the node was actually snapshotted by
    /// `GraphState.Update`.
    pub(super) projected_transaction_nodes: Vec<ProjectedTransactionNode>,
}

#[derive(Clone, Debug)]
pub(super) struct SourceOrientationContext {
    /// The owner container is outside the temporary direct-child graph.
    pub(super) root: Option<ArenaNode>,
    /// Node count observed by D2's `orientSourceInterior` call after
    /// `CombineSubgraphs` adds each active direct child and all descendants.
    /// For the top-level call this is the full graph inventory.
    pub(super) graph_node_count: usize,
    /// Directions are map entries, so `None` means the source map has no
    /// explicit direction at that key (distinct from the default Down value).
    pub(super) parent_direction: Option<Direction>,
    pub(super) root_direction: Option<Direction>,
    /// Exact length of the source root's `Edges` slice. The source-orientation
    /// trace and minimum-degree gate observe this count before validating any
    /// edge.
    pub(super) root_adjacency_edge_count: usize,
    /// Projected source-boundary edges available for candidate validation.
    /// Undirected and bidirectional edges are retained as `None`, because the
    /// Go pass rejects the whole candidate when any root edge is not a single
    /// directed edge leaving the root.
    pub(super) root_edges: Vec<Option<(i64, i64)>>,
    pub(super) disabled_for_loop: bool,
    pub(super) has_ancestor_obstacles: bool,
    pub(super) root_padding: Insets,
}

impl Default for SourceOrientationContext {
    fn default() -> Self {
        Self {
            root: None,
            graph_node_count: 0,
            parent_direction: None,
            root_direction: None,
            root_adjacency_edge_count: 0,
            root_edges: Vec::new(),
            disabled_for_loop: false,
            has_ancestor_obstacles: false,
            root_padding: Insets::uniform(0.0),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SubtreeMirror {
    pub(super) mirror_x: bool,
    pub(super) mirror_y: bool,
    pub(super) reachable_containers: BTreeSet<Option<NodeId>>,
}

#[derive(Clone, Debug)]
pub(super) struct FlatScopePlacement {
    pub(super) graph: ArenaGraph,
    /// Whether the final source-container transaction committed an interior
    /// rotation. Go's temporary scope aliases the owner nodes, so the owner
    /// observes these coordinates before its subsequent container fit.
    pub(super) source_orientation_applied: bool,
    pub(super) next_rng_float: Option<f64>,
    /// Axes mirrored by each SplitSubgraphs result's recovered `Graph.direct`.
    ///
    /// TALA's temporary graphs share node pointers with the owning graph. The
    /// reachable-container set preserves which descendants its `rdfsWalk`
    /// actually reflects and which it visits only for postorder refitting.
    pub(super) descendant_mirrors: BTreeMap<NodeId, SubtreeMirror>,
    /// Arrangement changes committed by the post-placement, flip-only
    /// `Cluster.optimize` calls, keyed by the owning arena's cluster index.
    pub(super) cluster_arrangements: BTreeMap<usize, ClusterArrangement>,
    /// Desired arrangements selected by `Cluster.optimize`, including a
    /// request whose centered and fallback transactions both roll back.
    /// TALA mutates DesiredArrangement before opening either transaction.
    pub(super) cluster_desired_arrangements: BTreeMap<usize, ClusterArrangement>,
    /// Final temporary cluster-vessel positions in this scope's coordinate
    /// system, keyed by the owning arena's cluster index.
    pub(super) cluster_vessel_positions: BTreeMap<usize, Point>,
    /// Direct children retained through an induced graph's shared Containers
    /// map, expressed relative to their current container box. Ordinary
    /// children from mirrored components are omitted because the owning
    /// graph's recursive mirror copyback moves those shared pointers.
    pub(super) external_shared_child_offsets: Vec<(i64, i64, Point)>,
    /// Ordinary external children mutated by a mirrored induced graph.
    ///
    /// TALA's induced graph aliases these child pointers with its owner. Rust
    /// clones them, so their final post-`direct` offsets must be copied back
    /// after the owner's recursive mirror replay. This includes temporary
    /// cluster vessels: publication updates their separate pending position
    /// rather than moving a stable member node.
    pub(super) mirrored_external_child_offsets: Vec<(i64, i64, Point)>,
}

#[derive(Clone, Copy)]
pub(super) enum RoutingCostField {
    Crossing,
    Turn,
    NonCenterPort,
}

impl RoutingCostField {
    fn read(self, values: (f64, f64, f64)) -> f64 {
        match self {
            Self::Crossing => values.0,
            Self::Turn => values.1,
            Self::NonCenterPort => values.2,
        }
    }

    fn write(self, values: &mut (f64, f64, f64), value: f64) {
        match self {
            Self::Crossing => values.0 = value,
            Self::Turn => values.1 = value,
            Self::NonCenterPort => values.2 = value,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Crossing => 0,
            Self::Turn => 1,
            Self::NonCenterPort => 2,
        }
    }
}

type EdgeLengthMap = Arc<std::sync::Mutex<HashMap<u64, f64>>>;

#[derive(Debug)]
struct GraphCostState {
    routing: (f64, f64, f64),
    // The map reference is distinct from the Graph.costMu owner. A source
    // placement snapshot reinstalls this original map, including nil mode.
    edge_length: Option<EdgeLengthMap>,
}

/// Source layoutgraph.PlacementCostSnapshot. Capture is explicit and the
/// placement caller charges its work guard before copying the map. Dropping
/// this value does not roll back ordinary speculative scoring.
#[derive(Debug)]
pub(super) struct PlacementCostSnapshot {
    owner: Arc<GraphCostCache>,
    cache_ref: Option<EdgeLengthMap>,
    cache: Option<HashMap<u64, f64>>,
    routing: (f64, f64, f64),
}

/// Actual native objects and raw map contents. The saved map lives inline in
/// its snapshot; its address is not a claim about a Go map allocation.
#[cfg(any(test, feature = "diagnostic-traces"))]
#[derive(Debug)]
pub(super) struct PlacementCostMapObservation {
    pub(super) reference_address: Option<usize>,
    pub(super) map_object_address: usize,
    pub(super) length: usize,
    pub(super) capacity: usize,
    pub(super) entries: Vec<(u64, u64)>,
}

#[cfg(any(test, feature = "diagnostic-traces"))]
#[derive(Debug)]
pub(super) struct PlacementCostSnapshotObservation {
    pub(super) snapshot_address: usize,
    pub(super) cost_owner_address: usize,
    pub(super) original_cache: Option<PlacementCostMapObservation>,
    pub(super) saved_cache: Option<PlacementCostMapObservation>,
    pub(super) owner_current_cache: Option<PlacementCostMapObservation>,
    pub(super) saved_routing_addresses: [usize; 3],
    pub(super) saved_routing_bits: [u64; 3],
    pub(super) owner_routing_addresses: [usize; 3],
    pub(super) owner_routing_bits: [u64; 3],
}

/// Raw current cost fields and the genuine current map allocation. This is
/// observational storage, never a captured optimizer rollback value.
#[cfg(any(test, feature = "diagnostic-traces"))]
#[derive(Debug)]
pub(super) struct GraphCostObservation {
    pub(super) cost_owner_address: usize,
    pub(super) cache: Option<PlacementCostMapObservation>,
    pub(super) routing_addresses: [usize; 3],
    pub(super) routing_bits: [u64; 3],
}

/// The retained alias's actual map and its relationship to the owner's
/// current reference, observed together under the owner's original lock.
#[cfg(any(test, feature = "diagnostic-traces"))]
#[derive(Debug)]
pub(super) struct RetainedCostMapObservation {
    pub(super) cost_owner_address: usize,
    pub(super) cache: PlacementCostMapObservation,
    pub(super) current_cache_reference_address: Option<usize>,
    pub(super) is_current: bool,
}

#[cfg(any(test, feature = "diagnostic-traces"))]
fn observe_live_placement_cost_map(reference: &EdgeLengthMap) -> PlacementCostMapObservation {
    let map = reference.lock().expect("edge-length map lock poisoned");
    PlacementCostMapObservation {
        reference_address: Some(Arc::as_ptr(reference) as usize),
        map_object_address: &*map as *const HashMap<u64, f64> as usize,
        length: map.len(),
        capacity: map.capacity(),
        entries: map
            .iter()
            .map(|(&key, &value)| (key, value.to_bits()))
            .collect(),
    }
}

impl PlacementCostSnapshot {
    /// Observe the actual held snapshot/owner/old map under the original lock
    /// order. Entry order is the actual HashMap traversal, with no sorting or
    /// lazy routing-cost reads. Every allocation here is diagnostic storage.
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn diagnostic_observe(&self) -> PlacementCostSnapshotObservation {
        fn observe_map(
            map: &HashMap<u64, f64>,
            reference_address: Option<usize>,
        ) -> PlacementCostMapObservation {
            PlacementCostMapObservation {
                reference_address,
                map_object_address: map as *const HashMap<u64, f64> as usize,
                length: map.len(),
                capacity: map.capacity(),
                entries: map
                    .iter()
                    .map(|(&key, &value)| (key, value.to_bits()))
                    .collect(),
            }
        }
        let state = self.owner.state.read().expect("graph-cost lock poisoned");
        let observe_shared = |reference: &EdgeLengthMap| {
            let map = reference.lock().expect("edge-length map lock poisoned");
            observe_map(&map, Some(Arc::as_ptr(reference) as usize))
        };
        PlacementCostSnapshotObservation {
            snapshot_address: self as *const Self as usize,
            cost_owner_address: Arc::as_ptr(&self.owner) as usize,
            original_cache: self.cache_ref.as_ref().map(observe_shared),
            saved_cache: self.cache.as_ref().map(|map| observe_map(map, None)),
            owner_current_cache: state.edge_length.as_ref().map(observe_shared),
            saved_routing_addresses: [
                &self.routing.0 as *const f64 as usize,
                &self.routing.1 as *const f64 as usize,
                &self.routing.2 as *const f64 as usize,
            ],
            saved_routing_bits: [
                self.routing.0.to_bits(),
                self.routing.1.to_bits(),
                self.routing.2.to_bits(),
            ],
            owner_routing_addresses: [
                &state.routing.0 as *const f64 as usize,
                &state.routing.1 as *const f64 as usize,
                &state.routing.2 as *const f64 as usize,
            ],
            owner_routing_bits: [
                state.routing.0.to_bits(),
                state.routing.1.to_bits(),
                state.routing.2.to_bits(),
            ],
        }
    }

    pub(super) fn restore(&self) {
        let mut state = self.owner.state.write().expect("graph-cost lock poisoned");
        if let Some(cache_ref) = &self.cache_ref {
            let mut cache = cache_ref.lock().expect("edge-length map lock poisoned");
            cache.clear();
            cache.extend(
                self.cache
                    .as_ref()
                    .expect("enabled placement snapshot has copied entries")
                    .iter()
                    .map(|(&key, &value)| (key, value)),
            );
        }
        state.edge_length = self.cache_ref.clone();
        state.routing = self.routing;
        self.owner.publish_routing_bits(state.routing);
    }
}

/// Opaque source map alias for identity controls. Every access keeps the
/// original Graph owner lock ahead of the map lock, even after map replacement.
#[cfg(any(test, feature = "diagnostic-traces"))]
#[derive(Clone, Debug)]
pub(super) struct EdgeLengthCacheAlias {
    owner: Arc<GraphCostCache>,
    cache_ref: EdgeLengthMap,
}

#[cfg(any(test, feature = "diagnostic-traces"))]
impl EdgeLengthCacheAlias {
    /// Read the actual retained map, without cloning its storage or routing
    /// through a newly created placement snapshot. Preserve its traversal.
    pub(super) fn diagnostic_observe(&self) -> RetainedCostMapObservation {
        let state = self.owner.state.read().expect("graph-cost lock poisoned");
        RetainedCostMapObservation {
            cost_owner_address: Arc::as_ptr(&self.owner) as usize,
            cache: observe_live_placement_cost_map(&self.cache_ref),
            current_cache_reference_address: state
                .edge_length
                .as_ref()
                .map(|current| Arc::as_ptr(current) as usize),
            is_current: state
                .edge_length
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &self.cache_ref)),
        }
    }

    pub(super) fn store(&self, key: u64, value: f64) {
        let _state = self.owner.state.write().expect("graph-cost lock poisoned");
        self.cache_ref
            .lock()
            .expect("edge-length map lock poisoned")
            .insert(key, value);
    }

    pub(super) fn remove(&self, key: u64) {
        let _state = self.owner.state.write().expect("graph-cost lock poisoned");
        self.cache_ref
            .lock()
            .expect("edge-length map lock poisoned")
            .remove(&key);
    }

    pub(super) fn lookup(&self, key: u64) -> Option<f64> {
        let _state = self.owner.state.read().expect("graph-cost lock poisoned");
        self.cache_ref
            .lock()
            .expect("edge-length map lock poisoned")
            .get(&key)
            .copied()
    }

    pub(super) fn entries(&self) -> HashMap<u64, f64> {
        let _state = self.owner.state.read().expect("graph-cost lock poisoned");
        self.cache_ref
            .lock()
            .expect("edge-length map lock poisoned")
            .clone()
    }

    pub(super) fn same_map(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner) && Arc::ptr_eq(&self.cache_ref, &other.cache_ref)
    }

    pub(super) fn is_current(&self) -> bool {
        let state = self.owner.state.read().expect("graph-cost lock poisoned");
        state
            .edge_length
            .as_ref()
            .is_some_and(|cache_ref| Arc::ptr_eq(cache_ref, &self.cache_ref))
    }
}

/// One Graph.costMu owner: raw routing triples, lazy first readers and the
/// placement map all use the same read/write lock. Map contents require a
/// second lock so safe aliases can retain the original map across replacement;
/// the lock order is always Graph owner then map. Geometry-only candidate
/// views share the owner; an independent Graph gets a new one.
#[derive(Debug)]
pub(super) struct GraphCostCache {
    state: std::sync::RwLock<GraphCostState>,
    /// Bits of `state.routing`, republished by every routing write before
    /// its write lock is released. The lazy getters' first, single-field
    /// read uses it; raw triples and every write still take the lock.
    routing_bits: [std::sync::atomic::AtomicU64; 3],
}

impl GraphCostCache {
    /// cache_bridge.go40–54: raw current fields under the actual cost owner
    /// lock, followed by its map lock. No lazy getter, clone, sort or write.
    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn diagnostic_observe(&self) -> GraphCostObservation {
        let state = self.state.read().expect("graph-cost lock poisoned");
        GraphCostObservation {
            cost_owner_address: self as *const Self as usize,
            cache: state
                .edge_length
                .as_ref()
                .map(observe_live_placement_cost_map),
            routing_addresses: [
                &state.routing.0 as *const f64 as usize,
                &state.routing.1 as *const f64 as usize,
                &state.routing.2 as *const f64 as usize,
            ],
            routing_bits: [
                state.routing.0.to_bits(),
                state.routing.1.to_bits(),
                state.routing.2.to_bits(),
            ],
        }
    }

    pub(super) fn new(routing: (f64, f64, f64)) -> Self {
        let bits = |value: f64| std::sync::atomic::AtomicU64::new(value.to_bits());
        Self {
            state: std::sync::RwLock::new(GraphCostState {
                routing,
                edge_length: Some(Arc::new(std::sync::Mutex::new(HashMap::new()))),
            }),
            routing_bits: [bits(routing.0), bits(routing.1), bits(routing.2)],
        }
    }

    /// Call with the write lock held, after changing `state.routing`.
    fn publish_routing_bits(&self, routing: (f64, f64, f64)) {
        for (slot, value) in self
            .routing_bits
            .iter()
            .zip([routing.0, routing.1, routing.2])
        {
            slot.store(value.to_bits(), std::sync::atomic::Ordering::Release);
        }
    }

    pub(super) fn raw(&self) -> (f64, f64, f64) {
        self.state.read().expect("graph-cost lock poisoned").routing
    }

    pub(super) fn restore_routing(&self, routing: (f64, f64, f64)) {
        let mut state = self.state.write().expect("graph-cost lock poisoned");
        state.routing = routing;
        self.publish_routing_bits(state.routing);
    }

    pub(super) fn set(&self, field: RoutingCostField, value: f64) {
        let mut state = self.state.write().expect("graph-cost lock poisoned");
        field.write(&mut state.routing, value);
        self.publish_routing_bits(state.routing);
    }

    pub(super) fn get_or_compute(
        &self,
        field: RoutingCostField,
        compute: impl FnOnce() -> f64,
    ) -> f64 {
        self.get_or_compute_optional(field, || Some(compute()))
    }

    /// NonCenterPortCost returns +0 without updating its raw zero sentinel
    /// when Graph.Edges is empty. `None` models that source early return.
    pub(super) fn get_or_compute_optional(
        &self,
        field: RoutingCostField,
        compute: impl FnOnce() -> Option<f64>,
    ) -> f64 {
        // The published bits equal this field of `raw()`: every write
        // republishes them before releasing the lock. A poisoned lock keeps
        // the locked read and its panic.
        let cached = if self.state.is_poisoned() {
            field.read(self.raw())
        } else {
            f64::from_bits(
                self.routing_bits[field.index()].load(std::sync::atomic::Ordering::Acquire),
            )
        };
        // Match Go's numeric zero sentinel: both signed zeros are lazy, while
        // NaNs and infinities are cached values whose exact bits are retained.
        if cached != 0.0 {
            return cached;
        }
        let mut state = self.state.write().expect("graph-cost lock poisoned");
        let cached = field.read(state.routing);
        if cached != 0.0 {
            return cached;
        }
        match compute() {
            Some(value) => {
                field.write(&mut state.routing, value);
                self.publish_routing_bits(state.routing);
                value
            }
            None => 0.0,
        }
    }

    pub(super) fn halve_turn(&self) {
        let mut state = self.state.write().expect("graph-cost lock poisoned");
        state.routing.1 /= 2.0;
        self.publish_routing_bits(state.routing);
    }

    pub(super) fn lookup_edge_length(&self, key: u64) -> Option<f64> {
        let state = self.state.read().expect("graph-cost lock poisoned");
        state.edge_length.as_ref().and_then(|cache| {
            cache
                .lock()
                .expect("edge-length map lock poisoned")
                .get(&key)
                .copied()
        })
    }

    pub(super) fn store_edge_length(&self, key: u64, value: f64) {
        let state = self.state.write().expect("graph-cost lock poisoned");
        if let Some(cache) = &state.edge_length {
            cache
                .lock()
                .expect("edge-length map lock poisoned")
                .insert(key, value);
        }
    }

    pub(super) fn edge_length_entries(&self) -> usize {
        let state = self.state.read().expect("graph-cost lock poisoned");
        state.edge_length.as_ref().map_or(0, |cache| {
            cache.lock().expect("edge-length map lock poisoned").len()
        })
    }

    pub(super) fn edge_length_cache_enabled(&self) -> bool {
        self.state
            .read()
            .expect("graph-cost lock poisoned")
            .edge_length
            .is_some()
    }

    pub(super) fn disable_edge_length_cache(&self) {
        self.state
            .write()
            .expect("graph-cost lock poisoned")
            .edge_length = None;
    }

    pub(super) fn clear_edge_length(&self) {
        let state = self.state.write().expect("graph-cost lock poisoned");
        if let Some(cache) = &state.edge_length {
            cache.lock().expect("edge-length map lock poisoned").clear();
        }
    }

    pub(super) fn reset_placement(&self) {
        let mut state = self.state.write().expect("graph-cost lock poisoned");
        if let Some(cache) = &state.edge_length {
            cache.lock().expect("edge-length map lock poisoned").clear();
        }
        state.routing = (0.0, 0.0, 0.0);
        self.publish_routing_bits(state.routing);
    }

    /// Source PublishDeserializedGraph replaces the map and raw costs while
    /// retaining the destination Graph.costMu owner and all detached aliases.
    pub(super) fn publish_deserialized_routing(&self, routing: (f64, f64, f64)) {
        let mut state = self.state.write().expect("graph-cost lock poisoned");
        state.edge_length = Some(Arc::new(std::sync::Mutex::new(HashMap::new())));
        state.routing = routing;
        self.publish_routing_bits(state.routing);
    }

    pub(super) fn snapshot_placement(self: &Arc<Self>) -> PlacementCostSnapshot {
        #[cfg(feature = "diagnostic-traces")]
        crate::internal::call_counts::count!(PlacementCostSnapshot);
        let state = self.state.read().expect("graph-cost lock poisoned");
        let cache = state
            .edge_length
            .as_ref()
            .map(|cache| cache.lock().expect("edge-length map lock poisoned").clone());
        PlacementCostSnapshot {
            owner: self.clone(),
            cache_ref: state.edge_length.clone(),
            cache,
            routing: state.routing,
        }
    }

    #[cfg(any(test, feature = "diagnostic-traces"))]
    pub(super) fn edge_length_cache_alias(self: &Arc<Self>) -> Option<EdgeLengthCacheAlias> {
        let state = self.state.read().expect("graph-cost lock poisoned");
        state
            .edge_length
            .as_ref()
            .map(|cache_ref| EdgeLengthCacheAlias {
                owner: self.clone(),
                cache_ref: cache_ref.clone(),
            })
    }
}

#[cfg(test)]
mod turn_cache_contract_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn lazy_turn_zero_sentinel_and_nonfinite_bits_match_go() {
        for zero in [0.0_f64, -0.0_f64] {
            let cache = GraphCostCache::new((0.0, zero, 0.0));
            let computations = AtomicUsize::new(0);
            for _ in 0..2 {
                assert_eq!(
                    cache
                        .get_or_compute(RoutingCostField::Turn, || {
                            computations.fetch_add(1, Ordering::Relaxed);
                            0.0
                        })
                        .to_bits(),
                    0
                );
            }
            assert_eq!(computations.load(Ordering::Relaxed), 2);
        }
        for bits in [
            0x7ff8_0000_0000_0042,
            f64::INFINITY.to_bits(),
            f64::NEG_INFINITY.to_bits(),
        ] {
            let cache = GraphCostCache::new((0.0, f64::from_bits(bits), 0.0));
            assert_eq!(
                cache
                    .get_or_compute(RoutingCostField::Turn, || panic!(
                        "cached nonzero must not compute"
                    ))
                    .to_bits(),
                bits
            );
            assert_eq!(cache.raw().1.to_bits(), bits);
        }
    }

    #[test]
    fn lazy_turn_concurrent_first_read_computes_once() {
        let cache = Arc::new(GraphCostCache::new((0.0, 0.0, 0.0)));
        let computations = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let cache = &cache;
                let computations = &computations;
                scope.spawn(move || {
                    assert_eq!(
                        cache.get_or_compute(RoutingCostField::Turn, || {
                            computations.fetch_add(1, Ordering::Relaxed);
                            112.5
                        }),
                        112.5
                    )
                });
            }
        });
        assert_eq!(computations.load(Ordering::Relaxed), 1);
        cache.halve_turn();
        assert_eq!(cache.raw().1, 56.25);
        cache.set(RoutingCostField::Turn, 0.0);
        assert_eq!(cache.raw().1.to_bits(), 0);
    }

    #[test]
    fn all_routing_scalars_preserve_cached_nonfinite_payloads() {
        for field in [
            RoutingCostField::Crossing,
            RoutingCostField::Turn,
            RoutingCostField::NonCenterPort,
        ] {
            for bits in [
                0x7ff8_1234_5678_9abc,
                f64::INFINITY.to_bits(),
                f64::NEG_INFINITY.to_bits(),
            ] {
                let cache = GraphCostCache::new((0.0, 0.0, 0.0));
                cache.set(field, f64::from_bits(bits));
                assert_eq!(
                    cache
                        .get_or_compute(field, || panic!("cached nonzero must not compute"))
                        .to_bits(),
                    bits
                );
                assert_eq!(field.read(cache.raw()).to_bits(), bits);
            }
        }
    }

    #[test]
    fn raw_routing_snapshots_do_not_mix_concurrent_restores() {
        let cache = GraphCostCache::new((1.0, 2.0, 3.0));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..1000 {
                    cache.restore_routing((4.0, 5.0, 6.0));
                    cache.restore_routing((1.0, 2.0, 3.0));
                }
            });
            for _ in 0..1000 {
                let raw = cache.raw();
                assert!(raw == (1.0, 2.0, 3.0) || raw == (4.0, 5.0, 6.0));
            }
        });
    }
}

#[cfg(test)]
mod placement_cost_snapshot_contract_tests {
    use super::*;

    #[test]
    fn placement_snapshot_restores_original_map_identity() {
        // Original Go TestPlacementCostSnapshotRestoresCacheIdentity:
        // replace only the map, then mutate the retained original alias.
        let cache = Arc::new(GraphCostCache::new((30.0, 40.0, 50.0)));
        cache.store_edge_length(1, 10.0);
        cache.store_edge_length(2, 20.0);
        let original = cache.edge_length_cache_alias().unwrap();
        let snapshot = cache.snapshot_placement();
        {
            let mut state = cache.state.write().unwrap();
            state.edge_length = Some(Arc::new(std::sync::Mutex::new(HashMap::from([(3, 300.0)]))));
            state.routing = (300.0, 400.0, 500.0);
        }
        assert!(!original.is_current());
        snapshot.restore();
        assert!(Arc::ptr_eq(&cache, &snapshot.owner));
        assert!(original.same_map(&cache.edge_length_cache_alias().unwrap()));
        original.store(99, 990.0);
        assert_eq!(cache.lookup_edge_length(99), Some(990.0));
        original.remove(99);
        assert_eq!(cache.raw(), (30.0, 40.0, 50.0));
        assert_eq!(cache.edge_length_entries(), 2);
        assert_eq!(cache.lookup_edge_length(1), Some(10.0));
        assert_eq!(cache.lookup_edge_length(2), Some(20.0));
        assert_eq!(cache.lookup_edge_length(3), None);
        eprintln!(
            "PLACEMENT_COST_OWNER_RESTORE same_owner={} same_map={} entries={} raw={:?}",
            Arc::ptr_eq(&cache, &snapshot.owner),
            original.is_current(),
            cache.edge_length_entries(),
            cache.raw()
        );
    }

    #[test]
    fn reset_placement_preserves_cache_mode_and_identity() {
        // Original Go TestResetPlacementCostsPreservesCacheModeAndIdentity.
        let cache = Arc::new(GraphCostCache::new((20.0, 30.0, 40.0)));
        let original = cache.edge_length_cache_alias().unwrap();
        cache.store_edge_length(1, 10.0);
        cache.reset_placement();
        assert!(original.is_current());
        assert_eq!(cache.edge_length_entries(), 0);
        original.store(2, 20.0);
        assert_eq!(cache.lookup_edge_length(2), Some(20.0));
        assert_eq!(cache.raw(), (0.0, 0.0, 0.0));

        cache.disable_edge_length_cache();
        cache.restore_routing((20.0, 30.0, 40.0));
        cache.reset_placement();
        cache.store_edge_length(3, 30.0);
        assert!(!cache.edge_length_cache_enabled());
        assert_eq!(cache.lookup_edge_length(3), None);
        assert_eq!(cache.edge_length_entries(), 0);
        assert_eq!(cache.raw(), (0.0, 0.0, 0.0));
        assert_eq!(original.lookup(2), Some(20.0));
    }

    #[test]
    fn placement_snapshot_restores_disabled_cache_without_enabling_it() {
        let cache = Arc::new(GraphCostCache::new((3.0, 5.0, 7.0)));
        cache.disable_edge_length_cache();
        let snapshot = cache.snapshot_placement();
        assert!(snapshot.cache_ref.is_none() && snapshot.cache.is_none());
        cache.publish_deserialized_routing((30.0, 50.0, 70.0));
        cache.store_edge_length(1, 10.0);
        snapshot.restore();
        assert!(!cache.edge_length_cache_enabled());
        cache.store_edge_length(2, 20.0);
        assert_eq!(cache.edge_length_entries(), 0);
        assert_eq!(cache.lookup_edge_length(1), None);
        assert_eq!(cache.lookup_edge_length(2), None);
        assert_eq!(cache.raw(), (3.0, 5.0, 7.0));
    }

    #[test]
    fn deserialized_cost_publication_replaces_map_under_original_owner() {
        // Cost-owner part of original Go
        // TestPublishDeserializedGraphReplacesPlacementCache.
        let cache = Arc::new(GraphCostCache::new((0.0, 0.0, 0.0)));
        let owner = cache.clone();
        cache.store_edge_length(1, 10.0);
        let original = cache.edge_length_cache_alias().unwrap();
        cache.publish_deserialized_routing((2.0, 3.0, 4.0));
        assert!(Arc::ptr_eq(&owner, &cache));
        assert!(cache.edge_length_cache_enabled());
        assert_eq!(cache.edge_length_entries(), 0);
        assert!(!original.is_current());
        assert_eq!(original.lookup(1), Some(10.0));
        original.store(2, 20.0);
        assert_eq!(cache.lookup_edge_length(2), None);
        assert_eq!(cache.raw(), (2.0, 3.0, 4.0));
    }

    #[test]
    fn placement_snapshot_preserves_exact_nonfinite_and_signed_zero_bits() {
        let raw_bits = [
            0x8000_0000_0000_0000,
            0x7ff8_1234_5678_9abc,
            0xfff0_0000_0000_0000,
        ];
        let cache = Arc::new(GraphCostCache::new((
            f64::from_bits(raw_bits[0]),
            f64::from_bits(raw_bits[1]),
            f64::from_bits(raw_bits[2]),
        )));
        let entries = [
            (1, 0x8000_0000_0000_0000),
            (2, 0x7ff8_1234_5678_9abc),
            (3, 0x7ff0_0000_0000_0000),
            (4, 0xfff0_0000_0000_0000),
        ];
        for (key, bits) in entries {
            cache.store_edge_length(key, f64::from_bits(bits));
        }
        let original = cache.edge_length_cache_alias().unwrap();
        let snapshot = cache.snapshot_placement();
        // Capture must be a raw read, including the lazy -0 sentinel.
        let raw = cache.raw();
        assert_eq!(
            [raw.0.to_bits(), raw.1.to_bits(), raw.2.to_bits()],
            raw_bits
        );
        cache.publish_deserialized_routing((1.0, 2.0, 3.0));
        original.store(99, 990.0);
        for _ in 0..2 {
            snapshot.restore();
            let raw = cache.raw();
            assert_eq!(
                [raw.0.to_bits(), raw.1.to_bits(), raw.2.to_bits()],
                raw_bits
            );
            assert!(original.is_current());
            assert_eq!(cache.edge_length_entries(), entries.len());
            assert_eq!(cache.lookup_edge_length(99), None);
            for (key, bits) in entries {
                assert_eq!(cache.lookup_edge_length(key).unwrap().to_bits(), bits);
            }
            original.store(99, 990.0);
        }
    }

    #[test]
    fn dropping_placement_snapshot_keeps_speculative_memoized_costs() {
        let cache = Arc::new(GraphCostCache::new((3.0, 5.0, 7.0)));
        cache.store_edge_length(1, 10.0);
        let snapshot = cache.snapshot_placement();
        cache.store_edge_length(2, 20.0);
        cache.restore_routing((30.0, 50.0, 70.0));
        drop(snapshot);
        assert_eq!(cache.raw(), (30.0, 50.0, 70.0));
        assert_eq!(cache.lookup_edge_length(1), Some(10.0));
        assert_eq!(cache.lookup_edge_length(2), Some(20.0));
    }
}

#[cfg(test)]
mod placement_snapshot_observer_contract_tests {
    use super::*;

    fn entries(observation: &PlacementCostMapObservation) -> HashMap<u64, u64> {
        observation.entries.iter().copied().collect()
    }

    #[test]
    fn raw_snapshot_observer_keeps_saved_clone_detached_alias_and_live_owner_distinct() {
        let saved_bits = [
            0x8000_0000_0000_0000,
            0x7ff8_0000_0000_0042,
            0x7ff0_0000_0000_0000,
        ];
        let cache = Arc::new(GraphCostCache::new((
            f64::from_bits(saved_bits[0]),
            f64::from_bits(saved_bits[1]),
            f64::from_bits(saved_bits[2]),
        )));
        cache.store_edge_length(7, f64::from_bits(0x8000_0000_0000_0000));
        let alias = cache.edge_length_cache_alias().unwrap();
        let snapshot = cache.snapshot_placement();
        let original_reference = snapshot.cache_ref.as_ref().unwrap();
        cache.publish_deserialized_routing((
            0.0,
            -f64::INFINITY,
            f64::from_bits(0x7ff8_0000_0000_0099),
        ));
        cache.store_edge_length(11, 11.25);
        alias.store(13, f64::from_bits(0x7ff8_0000_0000_0077));
        let before_counts = [
            Arc::strong_count(&cache),
            Arc::strong_count(original_reference),
        ];

        let observed = snapshot.diagnostic_observe();
        assert_eq!(
            observed.snapshot_address,
            &snapshot as *const PlacementCostSnapshot as usize
        );
        assert_eq!(observed.cost_owner_address, Arc::as_ptr(&cache) as usize);
        assert_eq!(observed.saved_routing_bits, saved_bits);
        assert_eq!(
            observed.owner_routing_bits,
            [0, f64::NEG_INFINITY.to_bits(), 0x7ff8_0000_0000_0099]
        );
        let original = observed.original_cache.as_ref().unwrap();
        let saved = observed.saved_cache.as_ref().unwrap();
        let current = observed.owner_current_cache.as_ref().unwrap();
        assert_eq!(
            original.reference_address,
            Some(Arc::as_ptr(original_reference) as usize)
        );
        assert_ne!(current.reference_address, original.reference_address);
        assert_eq!(saved.reference_address, None);
        assert_eq!(
            saved.map_object_address,
            snapshot.cache.as_ref().unwrap() as *const HashMap<u64, f64> as usize
        );
        assert_ne!(original.map_object_address, saved.map_object_address);
        assert_eq!(
            entries(original),
            HashMap::from([(7, 0x8000_0000_0000_0000), (13, 0x7ff8_0000_0000_0077)])
        );
        assert_eq!(entries(saved), HashMap::from([(7, 0x8000_0000_0000_0000)]));
        assert_eq!(entries(current), HashMap::from([(11, 11.25_f64.to_bits())]));
        assert_eq!(original.length, 2);
        assert_eq!(saved.length, 1);
        assert!(original.capacity >= original.length && saved.capacity >= saved.length);
        assert!(
            observed
                .saved_routing_addresses
                .iter()
                .all(|&address| address != 0)
        );
        assert_ne!(
            observed.saved_routing_addresses,
            observed.owner_routing_addresses
        );
        assert_eq!(Arc::strong_count(&cache), before_counts[0]);
        assert_eq!(Arc::strong_count(original_reference), before_counts[1]);
        assert!(!alias.is_current());
        assert_eq!(alias.lookup(13).unwrap().to_bits(), 0x7ff8_0000_0000_0077);
        assert_eq!(cache.lookup_edge_length(11), Some(11.25));

        snapshot.restore();
        let restored = snapshot.diagnostic_observe();
        assert_eq!(
            restored
                .owner_current_cache
                .as_ref()
                .unwrap()
                .reference_address,
            original.reference_address
        );
        assert_eq!(
            entries(restored.original_cache.as_ref().unwrap()),
            entries(saved)
        );
        assert_eq!(restored.owner_routing_bits, saved_bits);
        assert!(alias.is_current());
        assert_eq!(cache.lookup_edge_length(13), None);
    }

    #[test]
    fn raw_snapshot_observer_preserves_original_nil_and_current_allocated_empty_cache() {
        let cache = Arc::new(GraphCostCache::new((-0.0, 0.0, -0.0)));
        cache.disable_edge_length_cache();
        let snapshot = cache.snapshot_placement();
        let nil_observation = snapshot.diagnostic_observe();
        assert!(nil_observation.original_cache.is_none());
        assert!(nil_observation.saved_cache.is_none());
        assert!(nil_observation.owner_current_cache.is_none());
        assert!(!cache.edge_length_cache_enabled());

        cache.publish_deserialized_routing((0.0, -0.0, 0.0));
        let strong_count = Arc::strong_count(&cache);
        let observed = snapshot.diagnostic_observe();
        assert!(observed.original_cache.is_none() && observed.saved_cache.is_none());
        let current = observed.owner_current_cache.unwrap();
        assert!(current.reference_address.is_some());
        assert!(current.map_object_address != 0);
        assert_eq!(current.length, 0);
        assert!(current.entries.is_empty());
        assert_eq!(
            observed.saved_routing_bits,
            [(-0.0_f64).to_bits(), 0, (-0.0_f64).to_bits()]
        );
        assert_eq!(observed.owner_routing_bits, [0, (-0.0_f64).to_bits(), 0]);
        assert_eq!(Arc::strong_count(&cache), strong_count);
        assert!(cache.edge_length_cache_enabled());
        assert_eq!(cache.raw().1.to_bits(), (-0.0_f64).to_bits());

        snapshot.restore();
        let restored = snapshot.diagnostic_observe();
        assert!(restored.owner_current_cache.is_none());
        assert_eq!(
            restored.owner_routing_bits,
            nil_observation.saved_routing_bits
        );
    }
}

/// layoutgraph/neighbor_requirements.go: independent Go integer fields.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LongDistanceNeighborRequirements {
    pub(super) edge_count: i64,
    pub(super) max_width: i64,
    pub(super) max_height: i64,
}

/// A value whose mutable accesses are versioned. Every `DerefMut` (and
/// mutable iteration) stamps a fresh process-unique generation, and clones
/// keep their source's stamp, so two values with equal generations have equal
/// contents. Memoized projection passes use the stamps to recognize state they
/// have already brought to a fixpoint.
pub(super) struct Tracked<T> {
    value: T,
    generation: u64,
}

fn next_tracked_generation() -> u64 {
    // Each thread draws generations from its own block of a shared counter,
    // so stamps stay unique across seed threads without a hot atomic.
    const BLOCK: u64 = 1 << 32;
    static NEXT_BLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    thread_local! {
        static NEXT: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
    }
    NEXT.with(|next| {
        let (mut generation, mut end) = next.get();
        if generation == end {
            generation = NEXT_BLOCK.fetch_add(1, std::sync::atomic::Ordering::Relaxed) * BLOCK + 1;
            end = generation + BLOCK - 1;
        }
        next.set((generation + 1, end));
        generation
    })
}

impl<T> Tracked<T> {
    pub(super) fn new(value: T) -> Self {
        Self {
            value,
            generation: next_tracked_generation(),
        }
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// Restore a stamp taken before a mutable pass that left the contents
    /// bit-for-bit unchanged.
    pub(super) fn restore_unchanged_generation(&mut self, generation: u64) {
        self.generation = generation;
    }
}

impl<T: Default> Default for Tracked<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Clone> Clone for Tracked<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            generation: self.generation,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.value.clone_from(&source.value);
        self.generation = source.generation;
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Tracked<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(formatter)
    }
}

impl<T: FromIterator<A>, A> FromIterator<A> for Tracked<T> {
    fn from_iter<I: IntoIterator<Item = A>>(iter: I) -> Self {
        Self::new(T::from_iter(iter))
    }
}

impl<T> From<T> for Tracked<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> std::ops::Deref for Tracked<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> std::ops::DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.generation = next_tracked_generation();
        &mut self.value
    }
}

impl<'a, T> IntoIterator for &'a Tracked<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        (&self.value).into_iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Tracked<T>
where
    &'a mut T: IntoIterator,
{
    type Item = <&'a mut T as IntoIterator>::Item;
    type IntoIter = <&'a mut T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        (&mut **self).into_iter()
    }
}

/// Generations of the six sized projection caches, in field order.
pub(super) type ProjectionGenerations = [u64; 6];

/// Inputs of a projection pass that left the caches at a fixpoint.
#[derive(Debug, Default)]
pub(super) struct ProjectionMemo {
    /// Recent `reconcile_projected_offsets_from_positions` fixpoints: the
    /// cache generations, owner positions and child positions (as bits).
    pub(super) reconciled: Vec<(
        ProjectionGenerations,
        Vec<Option<(u64, u64)>>,
        Vec<(i64, Option<(u64, u64)>)>,
    )>,
    /// Recent `set_projected_cache_sizes` fixpoints: generations and updates.
    pub(super) sized: Vec<(ProjectionGenerations, Vec<(i64, (u64, u64))>)>,
}

#[derive(Debug)]
pub(super) struct ArenaGraph {
    pub nodes: Vec<ArenaNode>,
    /// Canonical owning topology and graph views. Retained entities and their
    /// live owner references are independent of every Graph.Nodes member slice.
    pub(super) shared_topology: SharedTopology,
    /// Current recovered `Graph.Nodes` slice order.
    ///
    /// Stable Rust IDs let aggregate members remain allocated while TALA
    /// temporarily replaces them with sequence and cluster vessels. Keep the
    /// observable owner order separately so order-sensitive stages still see
    /// the same remove/append lifecycle as the Go graph.
    pub(super) node_order: Vec<NodeId>,
    /// Fast membership for the stable Graph.Nodes slice during the sized
    /// optimizer. The order itself remains authoritative; this cache is only
    /// enabled at a scoring boundary and is invalidated by order rebuilds.
    pub(super) node_order_membership: Vec<bool>,
    pub(super) node_order_membership_valid: bool,
    /// Cached aggregate materialization flags for the current Graph.Nodes
    /// order. During scoring and gap normalization, sequence/cluster
    /// membership is stable while geometry changes frequently; keep the
    /// recovered Go pointer-state predicate out of every geometry lookup.
    pub(super) active_sequence_flags: Vec<bool>,
    pub(super) active_cluster_flags: Vec<bool>,
    pub(super) active_sequence_indices: Vec<Option<usize>>,
    pub(super) active_cluster_indices: Vec<Option<usize>>,
    pub(super) active_sequence_owners: Vec<NodeId>,
    pub(super) active_cluster_owners: Vec<NodeId>,
    /// Cached `Node.getContainer` result while aggregate membership is
    /// materialized. This topology is fixed for an optimizer pass even though
    /// node and vessel geometry changes for every candidate.
    pub(super) active_node_containers: Arc<Vec<Option<NodeId>>>,
    /// Stable member identity to its current aggregate vessel identity.
    ///
    /// The projected child boxes are mutable, so only cache this topological
    /// relation; callers continue to read live vessel geometry.
    pub(super) active_aggregate_vessels_by_member: Arc<HashMap<i64, i64>>,
    /// Current local aggregate vessel carrier by stable TALA identity.
    pub(super) active_aggregate_vessel_nodes: Arc<HashMap<i64, NodeId>>,
    /// Per TALA identity in Graph.Nodes: its first node, and whether another
    /// distinct node carries it. Built with the membership caches; `None`
    /// until a membership refresh.
    pub(super) node_order_tala_carriers:
        Option<Arc<crate::internal::IdHashMap<i64, (NodeId, bool)>>>,
    pub(super) active_graph_node_order: Vec<NodeId>,
    /// Diagnostic model of the live `Node.Graph` pointer and its owner's
    /// current `Graph.Nodes` slice. The shared view follows clones just as Go
    /// nodes retain pointers across temporary placement graphs.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) trace_owner_node_orders: Arc<std::sync::Mutex<BTreeMap<i64, Vec<i64>>>>,
    /// Owner slices observed at each recursive root callback. Go can change a
    /// node's Graph pointer between sibling calls, so a single final owner per
    /// node is insufficient for phase snapshots.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) trace_root_owner_node_orders: Arc<std::sync::Mutex<BTreeMap<i64, Vec<i64>>>>,
    /// Initial live Containers child slices captured before Rust's postorder
    /// scope materialization. Diagnostic projections restore tree children
    /// only after their corresponding scope completes in Go's recursive walk.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) trace_root_owner_container_children: Arc<std::sync::Mutex<BTreeMap<i64, Vec<i64>>>>,
    /// Completion order of nested placement scopes in the recovered recursive
    /// walk. Tree children become visible in the owner graph only after their
    /// owning scope completes; input hierarchy order does not capture
    /// edge-driven `PlaceChildrenOrder` traversal.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) trace_container_placement_ranks: Arc<std::sync::Mutex<BTreeMap<i64, usize>>>,
    /// Durable TALA identities for containers reachable from the owning
    /// placement graph's direct roots. Projected aliases keep original arena
    /// IDs while induced graphs use compact IDs, so mirror reachability needs
    /// this explicit identity set at the direct-pass boundary.
    pub(super) reachable_container_tala_ids: Option<BTreeSet<i64>>,
    /// True when the adapter supplied serialized `ChildrenArray` order.
    /// In-memory fixtures have no such source lifecycle metadata.
    pub(super) source_hierarchy_ordered: bool,
    /// Stable sibling order supplied by serialized `ChildrenArray` records.
    /// Aggregate placement can remove and reintroduce members in the live
    /// Containers slices, so retain the source order for projected maps that
    /// still expose those shared child pointers.
    pub(super) source_container_child_order: BTreeMap<i64, Vec<i64>>,
    /// Container child order frozen after TALA preprocessing and keyed by
    /// stable TALA identity. Connectivity observes these pointers even when
    /// induced placement scopes compact their arena indices or reorder their
    /// active child vectors.
    pub(super) connected_container_child_order: BTreeMap<i64, Vec<i64>>,
    /// Source-frame positions and parent identities for every serialized
    /// node. Temporary induced graphs omit hidden descendants from `nodes`,
    /// but projected edge scoring still needs their original parent-relative
    /// deltas when following shared Go node pointers.
    pub(super) source_positions_by_tala: BTreeMap<i64, Point>,
    pub(super) source_parent_tala_by_tala: BTreeMap<i64, Option<i64>>,
    /// Canonical allocations. Removing graph membership never moves these IDs.
    pub edge_records: Vec<ArenaEdge>,
    /// Original endpoint identities for each serialized edge. Placement
    /// scopes reattach edges to carrier nodes, but `mirrorAxes` reachability
    /// still follows the shared original edge pointers.
    pub(super) source_edge_endpoints: Vec<(i64, i64)>,
    /// Original endpoint identities keyed by the stable serialized TALA edge
    /// identity. Temporary placement graphs densify `input_id`, so indexed
    /// lookup is not valid after a split or projection.
    pub(super) source_edge_endpoints_by_id: BTreeMap<i64, (i64, i64)>,
    pub(super) edge_order: GraphEdges,
    /// Endpoint-local Node.Edges order reconstructed by placement scopes.
    /// Graph.buildTunnels consumes this pointer-visible order, while the
    /// owning arena's stable edge slice may still contain external aliases.
    pub(super) incident_edge_order: BTreeMap<NodeId, Vec<EdgeId>>,
    /// External incident edges visible through aliased Go node pointers but
    /// omitted from this temporary graph's projected edge inventory.
    pub(super) external_edge_counts: Vec<usize>,
    // TALA's optimizers receive one materialized Graph owner at a time. The
    // top-level Rust arena may temporarily contain several hierarchy scopes;
    // placement_scope_owned distinguishes that adapter state from both a flat
    // input graph and the temporary child Graphs built by Graph.placeNodes.
    pub(super) placement_scope_owned: bool,
    pub(super) root_hierarchy: bool,
    pub(super) containers: ContainerMap,
    /// Historical traversal image; live traversal reads Containers directly.
    pub(super) descendant_cache: Vec<Vec<NodeId>>,
    pub directions: BTreeMap<Option<NodeId>, Direction>,
    pub(super) scoring_directions: BTreeMap<Option<NodeId>, Direction>,
    /// TALA's `Graph.CopyEntitiesFrom` shares the direction maps keyed by
    /// original node pointers with every temporary graph. Dense Rust scope
    /// IDs cannot preserve that pointer identity, so retain the same maps by
    /// stable TALA node identity for scoring and projected replacements.
    pub(super) directions_by_tala: BTreeMap<Option<i64>, Direction>,
    pub(super) scoring_directions_by_tala: BTreeMap<Option<i64>, Direction>,
    pub cell_size: f64,
    /// CellSize on the owning Graph whose recursive `after-fit` callback is
    /// being mirrored. GraphJSON snapshots preserve this serialized value,
    /// while ordinary D2 inputs begin with zero even when placement computes
    /// a cell size on temporary child graphs.
    #[cfg(feature = "diagnostic-traces")]
    pub(super) source_cell_size: f64,
    pub(super) cost_cache: Arc<GraphCostCache>,
    /// Values captured by the OVG router after its actual graph first readers.
    /// Hop scoring must not accidentally perform another lazy graph read.
    pub(super) routing_costs: (f64, f64, f64),
    pub(super) max_spacing_delta: f64,
    pub(super) placement_component: Vec<Option<usize>>,
    pub(super) common_uncle_siblings: GraphNodeMap,
    /// Recovered `Graph.Hubs`, populated once by `PreprocessHubs`.
    pub(super) hubs: GraphNodeMap,
    pub(super) sized_adjacent_overrides: Tracked<BTreeMap<(NodeId, EdgeId), ProjectedAdjacent>>,
    pub(super) sized_cluster_distance_boxes:
        Tracked<BTreeMap<(NodeId, i64), ProjectedClusterDistance>>,
    pub(super) sized_edge_abductions: Tracked<Vec<SizedEdgeAbduction>>,
    /// Go distinguishes a nil scoring argument from a present empty slice.
    pub(super) sized_edge_abductions_present: bool,
    /// Legacy numerical-adapter cache for requirement values. Sized construction
    /// writes the shared canonical field; materialization clears this cache.
    pub(super) long_distance_neighbor_data:
        Vec<Option<BTreeMap<NodeId, LongDistanceNeighborRequirements>>>,
    pub(super) sized_projected_obstructions: Tracked<BTreeMap<(NodeId, EdgeId), SharedProjections>>,
    pub(super) sized_projected_obstruction_sets:
        Tracked<BTreeMap<(NodeId, EdgeId), OrderedObstructionInventory>>,
    pub(super) sized_collapsed_symmetry_neighbors: Tracked<BTreeMap<i64, Vec<ProjectedAdjacent>>>,
    /// Fixpoints of the projection passes over the six caches above.
    pub(super) projection_memo: ProjectionMemo,
    /// Main-graph SwapOptimize passes nil edge abductions to Node.edgeLength.
    /// The stable arena normally reconstructs aggregate endpoint projections;
    /// this transient flag suppresses that reconstruction for the swap score.
    pub(super) suppress_aggregate_projection: bool,
    /// SwapOptimize scores the synthetic aggregate vessel itself. Stable
    /// arena members normally retain projected member boxes, so this flag is
    /// scoped to that global swap scorer rather than ordinary placement.
    pub(super) swap_scoring_vessel_boxes: bool,
    /// Positioned container nodes retained by TALA's shared child graph but
    /// absent from this SplitSubgraphs `Nodes` slice.
    pub(super) transaction_external_containers: Vec<ArenaNode>,
    /// Positioned materialized containers used only as overlap-validation
    /// obstacles. They are not moved, wrapped, or included in score inputs.
    pub(super) transaction_validation_external_containers: Vec<ArenaNode>,
    pub(super) transaction_external_container_children: BTreeMap<i64, Vec<ArenaNode>>,
    /// Snapshot-only view of the complete source hierarchy. Induced placement
    /// graphs intentionally trim `containers` to direct members, while Go's
    /// trace instrumentation still follows shared descendant pointers.
    pub(super) trace_container_children: BTreeMap<i64, Vec<ArenaNode>>,
    pub(super) transaction_external_aggregate_children: BTreeMap<i64, Vec<ArenaNode>>,
    pub(super) transaction_external_cluster_layouts: BTreeMap<i64, ExternalClusterLayout>,
    pub(super) projected_transaction_nodes: Vec<ProjectedTransactionNode>,
    pub(super) root_grid_bin_pack_completed: bool,
    pub(super) sequences: Vec<SequenceState>,
    /// True once `assign_sequences` has populated every node backlink.
    ///
    /// Manually materialized recovery tests may provide only the Sequence
    /// slice, so lookups retain their evidence-compatible scan fallback until
    /// this phase has completed.
    pub(super) sequence_backlinks_complete: bool,
    pub(super) clusters: Vec<ClusterState>,
    /// Geometry of each live temporary cluster vessel. TALA materializes a
    /// vessel once during AddClusters and retains that box while nested
    /// members are laid out; deriving it from the members on every lookup
    /// incorrectly lets later container expansion resize the vessel.
    pub(super) cluster_vessel_sizes: Vec<Size>,
    /// Serialized vessel dimensions keyed by the signed TALA identity. The
    /// Go decoder owns a distinct vessel pointer; retain its box even though
    /// the stable arena uses a member carrier for ordinary NodeId operations.
    pub(super) serialized_vessel_sizes_by_tala: BTreeMap<i64, Size>,
    /// Serialized vessel origins keyed by the signed TALA identity. This is
    /// the source position of the auxiliary pointer, not a member position.
    pub(super) serialized_vessel_positions_by_tala: BTreeMap<i64, Point>,
    pub(super) serialized_cluster_arrangements_by_tala: BTreeMap<i64, String>,
    /// Resolved edge-abduction records attached to each decoded cluster
    /// vessel. The map key is the vessel identity, never a reconstructed
    /// member-set identity.
    pub(super) serialized_cluster_edge_abductions:
        BTreeMap<i64, Vec<ResolvedAggregateEdgeAbduction>>,
    /// Edge-abduction records attached to decoded sequence vessels.
    pub(super) serialized_sequence_edge_abductions:
        BTreeMap<i64, Vec<ResolvedAggregateEdgeAbduction>>,
    /// Serialized cluster vessel container identities. The source cluster
    /// owns a separate Vessel.Container pointer, so do not infer it only from
    /// the first stable member when a decoded snapshot supplies the record.
    pub(super) serialized_cluster_containers_by_tala: BTreeMap<i64, Option<i64>>,
    /// Top-left of each real temporary TALA cluster vessel.
    ///
    /// Stable arena IDs retain member nodes instead of allocating the vessel,
    /// so its independent box is carried here across nested sequence syncs.
    pub(super) pending_cluster_vessel_positions: BTreeMap<usize, Point>,
    /// True once `assign_clusters` has populated every node backlink.
    pub(super) cluster_backlinks_complete: bool,
    pub(super) preprocessed_tree_children: BTreeMap<Option<NodeId>, Vec<NodeId>>,
    /// Sentinel edges disconnected by `ExtractTrees` and appended again by
    /// `putBackNonBranchingTrees`. Reconnection appends these to both
    /// endpoint-local edge slices as well as `Graph.Edges`.
    pub(super) restored_tree_edges: BTreeSet<EdgeId>,
    /// Root-first `reconnectTree` order observed by shared endpoint nodes.
    ///
    /// `Graph.placeNodes` copies tree edges back to the owner Graph in a
    /// distinct descendant-first order, but its temporary graph shares Node
    /// pointers with that owner. Preserve the endpoint-local lifecycle
    /// separately instead of sorting `Node.Edges` to `Graph.Edges`.
    pub(super) published_tree_node_edge_order: Vec<EdgeId>,
    pub(super) restored_hierarchy_endpoint_order: Vec<(NodeId, EdgeId)>,
    /// Recovered `Graph.NodeToTree` ownership. Unlike a route-time topology
    /// guess, this survives tree preprocessing and carries the exact sentinel
    /// edge that TALA routes before ordinary OVG edges.
    pub(super) tree_routing_nodes: BTreeMap<NodeId, TreeRoutingNode>,
    /// Recovered `Graph.Trees` keys retained while branching descendants are
    /// absent from a temporary `SplitSubgraphs` node slice.
    pub(super) tree_sentinels: BTreeSet<NodeId>,
}

pub(super) fn fnv1a32(bytes: &[u8]) -> i64 {
    bytes.iter().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(16_777_619)
    }) as i64
}

#[derive(Clone, Debug)]
pub(super) struct Pipeline {
    pub input: Graph,
    pub graph: ArenaGraph,
    /// Fixed ancestor/sibling boxes penalized by D2 v0.9.0's sized optimizer.
    pub(super) sized_obstacles: Vec<Rect>,
    pub seed: i64,
    pub prearranged: bool,
    pub fixed_sizes: bool,
    pub completed_stages: Vec<Stage>,
    pub(super) misc_rng: super::go_rng::GoRng,
    /// Node placement opens its RNG before initializing the graph, then passes
    /// the untouched stream into the optimizer. Keep that source lifecycle
    /// explicit across the two Rust stages.
    pub(super) node_placement_rng: Option<super::go_rng::GoRng>,
    /// TALA keeps hierarchy ranking/placement on a separate RNG seeded with
    /// the graph seed.  It must never consume the optimizer's stream.
    pub(super) hierarchy_rng: super::go_rng::GoRng,
    pub(super) hierarchy_assignments: Vec<HierarchyAssignmentTrace>,
    pub(super) hierarchy_placements: Vec<HierarchyPlacementTrace>,
    pub(super) next_rng_float: Option<f64>,
    /// Recovered `Pipeline.runAlignAxis`: stages publish whether the following
    /// AlignAxes stage should run, and AlignAxes clears the carrier on entry.
    pub(super) run_align_axis: bool,
    pub(super) run_edge_route: bool,
    /// Whether this pipeline represents Go's top-level one-node fast path.
    pub(super) singleton_fast_path: bool,
    /// CompoundCandidate reuses the ordinary routing helpers but publishes
    /// its own `Compound/...` stage names. Suppress helper-level events while
    /// that candidate is being evaluated.
    pub(super) suppress_route_trace: bool,
    /// Diagnostic-only identity of the owning placement scope. This mirrors
    /// Go's `root.DebugID()` without affecting layout state.
    pub(super) trace_root: String,
    /// Shared v0.9.0 edge-routing work envelope. It is created per pipeline
    /// and reused by route post-processing so a stage cannot evade the
    /// aggregate limit by splitting work across helper calls.
    pub(super) route_stage_work: std::sync::Arc<super::routing::RouteStageWork>,
    /// `orientationContext` scans the source graph once per layout attempt;
    /// the result is shared by its recursive placement scopes.
    pub(super) source_orientation_disabled_for_loop: bool,
}
use super::*;

#[cfg(test)]
mod live_cost_observer_contract_tests {
    use super::*;

    fn counts(
        costs: &Arc<GraphCostCache>,
        old: &EdgeLengthCacheAlias,
        current: &EdgeLengthCacheAlias,
    ) -> (usize, usize, usize) {
        (
            Arc::strong_count(costs),
            Arc::strong_count(&old.cache_ref),
            Arc::strong_count(&current.cache_ref),
        )
    }

    #[test]
    fn raw_live_cost_observers_keep_detached_alias_addresses_and_state_unchanged() {
        let nan = f64::from_bits(0x7ff8000000000042);
        let costs = Arc::new(GraphCostCache::new((-0.0, nan, f64::INFINITY)));
        costs.store_edge_length(17, -0.0);
        costs.store_edge_length(23, nan);
        let old = costs.edge_length_cache_alias().unwrap();
        costs.publish_deserialized_routing((nan, -0.0, f64::NEG_INFINITY));
        costs.store_edge_length(99, f64::NEG_INFINITY);
        let current = costs.edge_length_cache_alias().unwrap();
        assert!(!old.same_map(&current));
        let before_counts = counts(&costs, &old, &current);
        let expected_bits = [
            nan.to_bits(),
            (-0.0f64).to_bits(),
            f64::NEG_INFINITY.to_bits(),
        ];
        let live = costs.diagnostic_observe();
        let retained = old.diagnostic_observe();
        let active = current.diagnostic_observe();
        assert_eq!(counts(&costs, &old, &current), before_counts);
        assert_eq!(live.cost_owner_address, Arc::as_ptr(&costs) as usize);
        assert_eq!(retained.cost_owner_address, live.cost_owner_address);
        assert_eq!(active.cost_owner_address, live.cost_owner_address);
        assert_eq!(live.routing_bits, expected_bits);
        {
            let state = costs.state.read().unwrap();
            assert_eq!(
                live.routing_addresses,
                [
                    &state.routing.0 as *const f64 as usize,
                    &state.routing.1 as *const f64 as usize,
                    &state.routing.2 as *const f64 as usize
                ]
            );
            let map = current.cache_ref.lock().unwrap();
            assert_eq!(
                live.cache.as_ref().unwrap().map_object_address,
                &*map as *const HashMap<u64, f64> as usize
            );
        }
        let current_map = live.cache.as_ref().unwrap();
        assert_eq!(
            current_map.reference_address,
            Some(Arc::as_ptr(&current.cache_ref) as usize)
        );
        assert_eq!(
            current_map.reference_address,
            active.cache.reference_address
        );
        assert_eq!(
            current_map.map_object_address,
            active.cache.map_object_address
        );
        assert_eq!(current_map.entries, [(99, f64::NEG_INFINITY.to_bits())]);
        assert!(active.is_current);
        assert!(!retained.is_current);
        assert_eq!(
            retained.current_cache_reference_address,
            current_map.reference_address
        );
        assert_eq!(
            retained.cache.reference_address,
            Some(Arc::as_ptr(&old.cache_ref) as usize)
        );
        assert_ne!(
            retained.cache.reference_address,
            current_map.reference_address
        );
        assert_ne!(
            retained.cache.map_object_address,
            current_map.map_object_address
        );
        assert_eq!(retained.cache.length, 2);
        let retained_bits = retained
            .cache
            .entries
            .iter()
            .copied()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            retained_bits,
            BTreeMap::from([(17, (-0.0f64).to_bits()), (23, nan.to_bits())])
        );
        assert_eq!(old.lookup(17).unwrap().to_bits(), (-0.0f64).to_bits());
        assert_eq!(old.lookup(23).unwrap().to_bits(), nan.to_bits());
        assert_eq!(costs.lookup_edge_length(99), Some(f64::NEG_INFINITY));
        assert_eq!(costs.raw().0.to_bits(), expected_bits[0]);
        assert_eq!(costs.raw().1.to_bits(), expected_bits[1]);
        assert_eq!(costs.raw().2.to_bits(), expected_bits[2]);
        assert_eq!(counts(&costs, &old, &current), before_counts);
    }

    #[test]
    fn raw_live_cost_observer_preserves_nil_and_empty_without_lazy_reads() {
        let nan = f64::from_bits(0x7ff8000000000042);
        let costs = Arc::new(GraphCostCache::new((-0.0, 0.0, nan)));
        costs.disable_edge_length_cache();
        let references = Arc::strong_count(&costs);
        for _ in 0..2 {
            let observed = costs.diagnostic_observe();
            assert!(observed.cache.is_none());
            assert_eq!(
                observed.routing_bits,
                [(-0.0f64).to_bits(), 0.0f64.to_bits(), nan.to_bits()]
            );
            assert_eq!(Arc::strong_count(&costs), references);
            assert!(!costs.edge_length_cache_enabled());
        }
        costs.publish_deserialized_routing((0.0, -0.0, f64::INFINITY));
        let alias = costs.edge_length_cache_alias().unwrap();
        let owner_references = Arc::strong_count(&costs);
        let map_references = Arc::strong_count(&alias.cache_ref);
        let observed = costs.diagnostic_observe();
        let retained = alias.diagnostic_observe();
        assert_eq!(
            observed.routing_bits,
            [
                0.0f64.to_bits(),
                (-0.0f64).to_bits(),
                f64::INFINITY.to_bits()
            ]
        );
        let empty = observed.cache.as_ref().unwrap();
        assert_eq!(empty.length, 0);
        assert!(empty.entries.is_empty());
        assert!(empty.reference_address.is_some());
        assert_ne!(empty.map_object_address, 0);
        assert_eq!(empty.reference_address, retained.cache.reference_address);
        assert!(retained.is_current);
        assert_eq!(Arc::strong_count(&costs), owner_references);
        assert_eq!(Arc::strong_count(&alias.cache_ref), map_references);
        assert!(costs.edge_length_cache_enabled());
        assert_eq!(costs.edge_length_entries(), 0);
    }
}
