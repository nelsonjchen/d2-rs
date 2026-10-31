// SPDX-License-Identifier: MPL-2.0
//! Process-wide kernel entries enabled by `D2RS_TALA_COUNTS=<fresh JSON path>`.
//! This module and every caller compile out without `diagnostic-traces`.
//!
//! Legacy diagnostic observers also execute work (including copies/refits).
//! Ordinary-work comparisons must disable those observers in a controlled build.
//! Counts accumulate across layout calls and seed threads; layout return
//! flushes after worker joins. They never reset behind another active layout.
//! Rust has no unified Transaction.Commit entry; that slot is reported as
//! null until its distributed boundaries have been aligned with Go.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

static OUTPUT: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static WRITE: Mutex<()> = Mutex::new(());
fn output() -> Option<&'static std::path::PathBuf> {
    OUTPUT
        .get_or_init(|| std::env::var_os("D2RS_TALA_COUNTS").map(Into::into))
        .as_ref()
}

#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum Counter {
    SizedOptimize,
    SizelessOptimize,
    SizedMove,
    SizedCandidateScore,
    NodeScore,
    NodeScorePrepare,
    GlobalScore,
    SwapTrial,
    TransposeTrial,
    AlignmentAttempt,
    GapPass,
    GapReduce,
    GapInnerTrial,
    RefitContainers,
    TransactionCommit,
    TransactionValidate,
    Binpack,
    BinpackTrial,
    RouteSearch,
    VisibilityBuild,
    QueuePop,
    GraphClone,
    GeometrySnapshot,
    GeometrySnapshotUnchecked,
    PlacementCostSnapshot,
    Layout,
    StagePrescale,
    StagePreprocessSequences,
    StagePreprocess,
    StagePreprocessTrees,
    StagePreprocessHierarchies,
    StagePreprocessClusters,
    StagePreprocessHubs,
    StageNodePlacement,
    StageSwapStuff,
    StageTranspose,
    StageAlignAxes,
    StageGapNormalization,
    StageOptimizeClusters,
    StageBalanceSymmetry,
    StageEquidistance,
    StageBinPack,
    StageCleanupStuff,
    StageRescale,
    StageEdgeRouting,
    StageCrosshatch,
    StageDejitter,
    StageSimplifyEdgeRoutes,
    StageSwapEdgePorts,
    StageStraightEdgesFallback,
    StageBalanceEdgeSegments,
    StageFixClusterEdgeBranching,
    StageTraceEdgesToShapeBorder,
    StageReorderDuplicates,
    StagePlaceLabels,
    StageNudgeEdgeChannels,
    StageShortcutEdgeRoutes,
    StageNormalize,
}
const NAMES: &[&str] = &[
    "sized_optimize",
    "sizeless_optimize",
    "sized_move",
    "sized_candidate_score",
    "node_score",
    "node_score_prepare",
    "global_score",
    "swap_trial",
    "transpose_trial",
    "alignment_attempt",
    "gap_pass",
    "gap_reduce",
    "gap_inner_trial",
    "refit_containers",
    "transaction_commit",
    "transaction_validate",
    "binpack",
    "binpack_trial",
    "route_search",
    "visibility_build",
    "queue_pop",
    "graph_clone",
    "geometry_snapshot",
    "geometry_snapshot_unchecked",
    "placement_cost_snapshot",
    "layout",
    "stage.Prescale",
    "stage.PreprocessSequences",
    "stage.Preprocess",
    "stage.PreprocessTrees",
    "stage.PreprocessHierarchies",
    "stage.PreprocessClusters",
    "stage.PreprocessHubs",
    "stage.NodePlacement",
    "stage.SwapStuff",
    "stage.Transpose",
    "stage.AlignAxes",
    "stage.GapNormalization",
    "stage.OptimizeClusters",
    "stage.BalanceSymmetry",
    "stage.Equidistance",
    "stage.BinPack",
    "stage.CleanupStuff",
    "stage.Rescale",
    "stage.EdgeRouting",
    "stage.Crosshatch",
    "stage.Dejitter",
    "stage.SimplifyEdgeRoutes",
    "stage.SwapEdgePorts",
    "stage.StraightEdgesFallback",
    "stage.BalanceEdgeSegments",
    "stage.FixClusterEdgeBranching",
    "stage.TraceEdgesToShapeBorder",
    "stage.ReorderDuplicates",
    "stage.PlaceLabels",
    "stage.NudgeEdgeChannels",
    "stage.ShortcutEdgeRoutes",
    "stage.Normalize",
];
static COUNTS: [AtomicU64; NAMES.len()] = [const { AtomicU64::new(0) }; NAMES.len()];

#[inline]
pub(crate) fn hit(counter: Counter) {
    if output().is_some() {
        COUNTS[counter as usize].fetch_add(1, Ordering::Relaxed);
    }
}
macro_rules! count {
    ($counter:ident) => {
        $crate::internal::call_counts::hit($crate::internal::call_counts::Counter::$counter)
    };
}
pub(crate) use count;

pub(crate) struct Dump;
impl Drop for Dump {
    fn drop(&mut self) {
        if let Some(path) = output() {
            let _write = WRITE.lock().expect("call counter writer");
            let counts: std::collections::BTreeMap<_, _> = NAMES
                .iter()
                .zip(COUNTS.iter())
                .map(|(name, count)| {
                    (
                        *name,
                        if *name == "transaction_commit" {
                            None
                        } else {
                            Some(count.load(Ordering::Relaxed))
                        },
                    )
                })
                .collect();
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&counts).expect("counter JSON"),
            )
            .expect("write D2RS_TALA_COUNTS");
        }
    }
}
