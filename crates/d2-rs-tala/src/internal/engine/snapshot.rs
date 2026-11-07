// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Diagnostic stage snapshots and complete-candidate seed racing.
//!
//! Snapshot replay observes production boundaries; normal layout runs one full
//! pipeline per seed and selects one complete result under a time budget.

use super::*;
use crate::RaceScoreComponents;
use std::borrow::Cow;
use std::cmp::Ordering as CandidateOrdering;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(any(not(target_arch = "wasm32"), test))]
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Runs one seeded candidate through a requested diagnostic boundary.
///
/// This compatibility entry point is not a stable application API. Use
/// [`crate::Engine::layout`] for production layout.
pub fn layout_snapshot(input: &Graph, seed: i64, stage: LayoutStage) -> LayoutSnapshot {
    let _transactions = limits::AutolayoutTransactionScope::enter();
    #[cfg(feature = "diagnostic-traces")]
    crate::internal::call_counts::count!(Layout);
    let requested_stage = stage;
    let stage = match stage {
        LayoutStage::EdgeRouting
        | LayoutStage::Crosshatch
        | LayoutStage::Dejitter
        | LayoutStage::SecondEdgeRouting
        | LayoutStage::SimplifyEdgeRoutes
        | LayoutStage::SwapEdgePorts
        | LayoutStage::StraightEdgesFallback
        | LayoutStage::BalanceEdgeSegments
        | LayoutStage::FixClusterEdgeBranching
        | LayoutStage::TraceEdgesToShapeBorder
        | LayoutStage::ReorderDuplicates
        | LayoutStage::PlaceLabels
        | LayoutStage::Normalize => LayoutStage::Rescale,
        stage => stage,
    };
    let scope_owned_placement = matches!(
        stage,
        LayoutStage::NodePlacement
            | LayoutStage::SwapFirstPass
            | LayoutStage::SwapSecondPass
            | LayoutStage::SwapThirdPass
            | LayoutStage::SwapStuff
            | LayoutStage::Transpose
            | LayoutStage::AlignAxes
            | LayoutStage::GapNormalization
            | LayoutStage::OptimizeClusters
            | LayoutStage::BalanceSymmetry
            | LayoutStage::Equidistance
            | LayoutStage::FirstBinPack
            | LayoutStage::Rescale
    );
    let mut pipeline = Pipeline::new(input, seed, false, false);
    macro_rules! cancellation_boundary {
        () => {
            if crate::internal::cancellation_requested() {
                return pipeline.snapshot(requested_stage);
            }
        };
    }
    macro_rules! stage_boundary {
        ($name:literal) => {
            #[cfg(feature = "diagnostic-traces")]
            crate::internal::call_counts::hit(match $name {
                "Prescale" => crate::internal::call_counts::Counter::StagePrescale,
                "PreprocessSequences" => {
                    crate::internal::call_counts::Counter::StagePreprocessSequences
                }
                "Preprocess" => crate::internal::call_counts::Counter::StagePreprocess,
                "PreprocessTrees" => crate::internal::call_counts::Counter::StagePreprocessTrees,
                "PreprocessHierarchies" => {
                    crate::internal::call_counts::Counter::StagePreprocessHierarchies
                }
                "PreprocessClusters" => {
                    crate::internal::call_counts::Counter::StagePreprocessClusters
                }
                "PreprocessHubs" => crate::internal::call_counts::Counter::StagePreprocessHubs,
                "NodePlacement" => crate::internal::call_counts::Counter::StageNodePlacement,
                "SwapStuff" => crate::internal::call_counts::Counter::StageSwapStuff,
                "Transpose" => crate::internal::call_counts::Counter::StageTranspose,
                "AlignAxes" => crate::internal::call_counts::Counter::StageAlignAxes,
                "GapNormalization" => crate::internal::call_counts::Counter::StageGapNormalization,
                "OptimizeClusters" => crate::internal::call_counts::Counter::StageOptimizeClusters,
                "BalanceSymmetry" => crate::internal::call_counts::Counter::StageBalanceSymmetry,
                "Equidistance" => crate::internal::call_counts::Counter::StageEquidistance,
                "BinPack" => crate::internal::call_counts::Counter::StageBinPack,
                "CleanupStuff" => crate::internal::call_counts::Counter::StageCleanupStuff,
                "Rescale" => crate::internal::call_counts::Counter::StageRescale,
                "EdgeRouting" => crate::internal::call_counts::Counter::StageEdgeRouting,
                "Crosshatch" => crate::internal::call_counts::Counter::StageCrosshatch,
                "Dejitter" => crate::internal::call_counts::Counter::StageDejitter,
                "SimplifyEdgeRoutes" => {
                    crate::internal::call_counts::Counter::StageSimplifyEdgeRoutes
                }
                "SwapEdgePorts" => crate::internal::call_counts::Counter::StageSwapEdgePorts,
                "StraightEdgesFallback" => {
                    crate::internal::call_counts::Counter::StageStraightEdgesFallback
                }
                "BalanceEdgeSegments" => {
                    crate::internal::call_counts::Counter::StageBalanceEdgeSegments
                }
                "FixClusterEdgeBranching" => {
                    crate::internal::call_counts::Counter::StageFixClusterEdgeBranching
                }
                "TraceEdgesToShapeBorder" => {
                    crate::internal::call_counts::Counter::StageTraceEdgesToShapeBorder
                }
                "ReorderDuplicates" => {
                    crate::internal::call_counts::Counter::StageReorderDuplicates
                }
                "PlaceLabels" => crate::internal::call_counts::Counter::StagePlaceLabels,
                "NudgeEdgeChannels" => {
                    crate::internal::call_counts::Counter::StageNudgeEdgeChannels
                }
                "ShortcutEdgeRoutes" => {
                    crate::internal::call_counts::Counter::StageShortcutEdgeRoutes
                }
                "Normalize" => crate::internal::call_counts::Counter::StageNormalize,
                _ => unreachable!(),
            });
            if let Err(error) = pipeline.check_stage_dimensions($name) {
                let mut snapshot = pipeline.snapshot(requested_stage);
                snapshot.layout_error = Some(error);
                return snapshot;
            }
        };
    }
    macro_rules! checked_compaction {
        ($optimizer:ident, $horizontal:expr) => {
            if let Err(error) = $optimizer.compact($horizontal, 3.0) {
                drop($optimizer);
                let mut snapshot = pipeline.snapshot(requested_stage);
                snapshot.layout_error = Some(LayoutError::PlacementTransaction(error.to_string()));
                return snapshot;
            }
        };
    }
    pipeline.run_prescale();
    cancellation_boundary!();
    pipeline.trace_node_stage("Prescale");
    stage_boundary!("Prescale");
    if stage == LayoutStage::Prescale {
        return pipeline.snapshot(requested_stage);
    }
    pipeline.run_preprocess_sequences();
    cancellation_boundary!();
    pipeline.trace_node_stage("PreprocessSequences");
    stage_boundary!("PreprocessSequences");
    pipeline.run_preprocess();
    cancellation_boundary!();
    pipeline.trace_node_stage("Preprocess");
    stage_boundary!("Preprocess");
    pipeline.run_preprocess_trees();
    cancellation_boundary!();
    pipeline.trace_node_stage("PreprocessTrees");
    stage_boundary!("PreprocessTrees");
    pipeline.run_preprocess_containers();
    pipeline.run_preprocess_hierarchies();
    cancellation_boundary!();
    pipeline.trace_node_stage("PreprocessHierarchies");
    stage_boundary!("PreprocessHierarchies");
    if stage == LayoutStage::PreprocessHierarchies {
        return pipeline.snapshot(requested_stage);
    }
    pipeline.run_preprocess_clusters();
    cancellation_boundary!();
    pipeline.trace_node_stage("PreprocessClusters");
    stage_boundary!("PreprocessClusters");
    pipeline.run_preprocess_hubs();
    cancellation_boundary!();
    pipeline.trace_node_stage("PreprocessHubs");
    stage_boundary!("PreprocessHubs");
    // `PlaceHierarchies` has already established these coordinates. TALA's
    // sizeless/sized optimizers are constructed later for each ordinary
    // `SplitSubgraphs` result inside `Graph.placeNodes`; they never run once
    // over the unified graph containing hierarchy-owned nodes.
    // Do not use `has_materialized_hierarchy` here: before ordinary placement
    // its containment predicate deliberately accepts unpositioned children,
    // so any container would look materialized.  The completed placement
    // records are the direct result of this invocation's `PlaceHierarchies`
    // loop and therefore identify only coordinates owned by that stage.
    let hierarchy_owned_placement = !pipeline.hierarchy_placements.is_empty();
    // Recovered Graph.placeNodes materializes one temporary children graph per
    // container, then a root children graph. The root graph is a distinct
    // transaction even when the owner graph is flat, because sequence and
    // cluster members have already been replaced by temporary vessels.
    let projected_scope_placement = scope_owned_placement;
    if matches!(
        stage,
        LayoutStage::InitializeNodes
            | LayoutStage::SizelessOptimize
            | LayoutStage::SizelessFirstPass
            | LayoutStage::SizelessFirstCompaction
            | LayoutStage::SizelessSecondPass
            | LayoutStage::SizelessSecondCompaction
            | LayoutStage::SizelessAnneal
            | LayoutStage::TransitionCompaction
            | LayoutStage::SizedStart
            | LayoutStage::SizedFirstNode
            | LayoutStage::SizedFirstPass
            | LayoutStage::SizedSecondPass
            | LayoutStage::SizedPreCompaction
            | LayoutStage::SizedBeforeFirstCompaction
            | LayoutStage::SizedFirstCompaction
            | LayoutStage::SizedSecondCompaction
            | LayoutStage::SizedAnneal
            | LayoutStage::SizedZeroOptimize
            | LayoutStage::NodePlacement
            | LayoutStage::SwapFirstPass
            | LayoutStage::SwapSecondPass
            | LayoutStage::SwapThirdPass
            | LayoutStage::SwapStuff
            | LayoutStage::Transpose
            | LayoutStage::AlignAxes
            | LayoutStage::GapNormalization
            | LayoutStage::OptimizeClusters
            | LayoutStage::BalanceSymmetry
            | LayoutStage::Equidistance
            | LayoutStage::FirstBinPack
            | LayoutStage::Rescale
    ) && !hierarchy_owned_placement
        && !projected_scope_placement
    {
        // A direct placement scope computes its cell size before
        // InitializeNodes. Pipeline::new deliberately leaves the owning root
        // at CellSize == 0 in the normal execution path, so stage snapshots
        // must reproduce the temporary scope's sizing boundary before fixed
        // coordinates are converted into cell coordinates.
        if pipeline.graph.cell_size == 0.0 {
            pipeline.graph.compute_cell_size();
        }
        if let Err(error) = pipeline.run_initialize_nodes() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        cancellation_boundary!();
    }
    if stage == LayoutStage::SizelessOptimize && !hierarchy_owned_placement {
        pipeline.run_sizeless_optimize(0.0);
    } else if stage == LayoutStage::SizelessFirstPass && !hierarchy_owned_placement {
        pipeline.run_sizeless_optimize(2.0 * (pipeline.graph.nodes.len() as f64).sqrt());
    } else if stage == LayoutStage::SizelessFirstCompaction && !hierarchy_owned_placement {
        let temperature = 2.0 * (pipeline.graph.nodes.len() as f64).sqrt();
        let mut optimizer = sizeless::SizelessOptimizer::new(&mut pipeline.graph, seed);
        optimizer.optimize(temperature);
        checked_compaction!(optimizer, true);
        pipeline.next_rng_float = Some(optimizer.next_float_probe());
        cancellation_boundary!();
    } else if stage == LayoutStage::SizelessSecondPass && !hierarchy_owned_placement {
        let iterations = (90.0 * (pipeline.graph.nodes.len() as f64).sqrt()) as usize;
        let mut temperature = 2.0 * (pipeline.graph.nodes.len() as f64).sqrt();
        let cooling_factor = (1.0 / iterations as f64 * (0.2 / temperature).ln()).exp();
        let mut optimizer = sizeless::SizelessOptimizer::new(&mut pipeline.graph, seed);
        optimizer.optimize(temperature);
        cancellation_boundary!();
        pipeline.next_rng_float = Some(optimizer.next_float_probe());
        checked_compaction!(optimizer, true);
        temperature *= cooling_factor;
        optimizer.optimize(temperature);
    } else if stage == LayoutStage::SizelessSecondCompaction && !hierarchy_owned_placement {
        let iterations = (90.0 * (pipeline.graph.nodes.len() as f64).sqrt()) as usize;
        let mut temperature = 2.0 * (pipeline.graph.nodes.len() as f64).sqrt();
        let cooling_factor = (1.0 / iterations as f64 * (0.2 / temperature).ln()).exp();
        let mut optimizer = sizeless::SizelessOptimizer::new(&mut pipeline.graph, seed);
        for iteration in 0..=9 {
            optimizer.optimize(temperature);
            cancellation_boundary!();
            if iteration == 0 {
                checked_compaction!(optimizer, true);
            } else if iteration == 9 {
                checked_compaction!(optimizer, false);
            }
            temperature *= cooling_factor;
        }
        pipeline.next_rng_float = Some(optimizer.next_float_probe());
    } else if matches!(
        stage,
        LayoutStage::SizelessAnneal | LayoutStage::TransitionCompaction | LayoutStage::SizedStart
    ) && !hierarchy_owned_placement
    {
        pipeline.run_sizeless_anneal();
        cancellation_boundary!();
        if matches!(
            stage,
            LayoutStage::TransitionCompaction | LayoutStage::SizedStart
        ) {
            pipeline.graph.transition_compact();
        }
        if stage == LayoutStage::SizedStart {
            pipeline.graph.snap_nonfixed_to_cells();
        }
        cancellation_boundary!();
    } else if matches!(
        stage,
        LayoutStage::SizedFirstNode
            | LayoutStage::SizedFirstPass
            | LayoutStage::SizedSecondPass
            | LayoutStage::SizedPreCompaction
            | LayoutStage::SizedBeforeFirstCompaction
            | LayoutStage::SizedFirstCompaction
            | LayoutStage::SizedSecondCompaction
            | LayoutStage::SizedAnneal
            | LayoutStage::SizedZeroOptimize
            | LayoutStage::NodePlacement
            | LayoutStage::SwapFirstPass
            | LayoutStage::SwapSecondPass
            | LayoutStage::SwapThirdPass
            | LayoutStage::SwapStuff
            | LayoutStage::Transpose
            | LayoutStage::AlignAxes
            | LayoutStage::GapNormalization
            | LayoutStage::OptimizeClusters
            | LayoutStage::BalanceSymmetry
            | LayoutStage::Equidistance
            | LayoutStage::FirstBinPack
            | LayoutStage::Rescale
    ) {
        if !hierarchy_owned_placement && !projected_scope_placement {
            let placement_node_count = pipeline.graph.largest_optimizable_component_size();
            let total_iterations = (90.0 * (placement_node_count as f64).sqrt()) as usize;
            let pass_count = match stage {
                LayoutStage::SizedSecondPass => 2,
                LayoutStage::SizedPreCompaction => 6,
                LayoutStage::SizedBeforeFirstCompaction | LayoutStage::SizedFirstCompaction => 7,
                LayoutStage::SizedSecondCompaction => 16,
                LayoutStage::SizedAnneal
                | LayoutStage::SizedZeroOptimize
                | LayoutStage::NodePlacement
                | LayoutStage::SwapFirstPass
                | LayoutStage::SwapSecondPass
                | LayoutStage::SwapThirdPass
                | LayoutStage::SwapStuff
                | LayoutStage::Transpose
                | LayoutStage::AlignAxes
                | LayoutStage::GapNormalization
                | LayoutStage::OptimizeClusters
                | LayoutStage::BalanceSymmetry
                | LayoutStage::Equidistance
                | LayoutStage::FirstBinPack
                | LayoutStage::Rescale => total_iterations.saturating_sub(total_iterations / 2 + 1),
                _ => 1,
            };
            let compact_during_passes = matches!(
                stage,
                LayoutStage::SizedFirstCompaction
                    | LayoutStage::SizedSecondCompaction
                    | LayoutStage::SizedAnneal
                    | LayoutStage::SizedZeroOptimize
                    | LayoutStage::NodePlacement
                    | LayoutStage::SwapFirstPass
                    | LayoutStage::SwapSecondPass
                    | LayoutStage::SwapThirdPass
                    | LayoutStage::SwapStuff
                    | LayoutStage::Transpose
                    | LayoutStage::AlignAxes
                    | LayoutStage::GapNormalization
                    | LayoutStage::OptimizeClusters
                    | LayoutStage::BalanceSymmetry
                    | LayoutStage::Equidistance
                    | LayoutStage::FirstBinPack
                    | LayoutStage::Rescale
            );
            pipeline.run_sized_pass(
                pass_count,
                stage == LayoutStage::SizedFirstNode,
                compact_during_passes,
                matches!(
                    stage,
                    LayoutStage::SizedZeroOptimize
                        | LayoutStage::NodePlacement
                        | LayoutStage::SwapFirstPass
                        | LayoutStage::SwapSecondPass
                        | LayoutStage::SwapThirdPass
                        | LayoutStage::SwapStuff
                        | LayoutStage::Transpose
                        | LayoutStage::AlignAxes
                        | LayoutStage::GapNormalization
                        | LayoutStage::OptimizeClusters
                        | LayoutStage::BalanceSymmetry
                        | LayoutStage::Equidistance
                        | LayoutStage::FirstBinPack
                        | LayoutStage::Rescale
                ),
                // `Graph.abductEdges` initializes and returns a non-nil empty
                // slice even when the flat scope has no abductions. TALA's
                // sized transpose distinguishes that from nil: non-nil uses
                // local edge scoring and cell-rounded rotations.
                Some(BTreeSet::new()),
            );
            cancellation_boundary!();
        }
        let recursively_placed = if projected_scope_placement {
            match pipeline.place_nodes_recursively() {
                Ok(placed) => placed,
                Err(error) => {
                    let mut snapshot = pipeline.snapshot(requested_stage);
                    snapshot.layout_error = Some(error);
                    return snapshot;
                }
            }
        } else {
            false
        };
        cancellation_boundary!();
        // Materialized, edge-isolated containers are transaction carriers for
        // recursive transpose validation.  Once root placement has completed
        // they are ordinary disconnected components again; carrying the
        // validation-only aliases into the later bin-pack stage perturbs the
        // component order (and therefore every tie-sensitive packed offset).
        // Keep this escape hatch diagnostic-only while the boundary is being
        // validated against the v0.9.0 oracle.
        if crate::internal::trace_env_enabled("TALA_TRACE_DROP_MATERIALIZED_EXTERNAL_AFTER_PLACE") {
            let dropped = pipeline
                .graph
                .transaction_external_containers
                .iter()
                .filter(|node| {
                    node.is_container
                        && node.position.is_some()
                        && node.fixed_top_left.is_none()
                        && node.hierarchy.is_none()
                        && node.grid_rows.is_none()
                        && node.grid_columns.is_none()
                })
                .map(|node| node.tala_id)
                .collect::<BTreeSet<_>>();
            pipeline
                .graph
                .transaction_external_containers
                .retain(|node| !dropped.contains(&node.tala_id));
            pipeline
                .graph
                .transaction_external_container_children
                .retain(|parent, _| !dropped.contains(parent));
            if !dropped.is_empty() {
                eprintln!("DROP_MATERIALIZED_EXTERNAL_RUST {:?}", dropped);
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_DROP_ALL_EXTERNAL_AFTER_PLACE") {
                pipeline.graph.transaction_external_containers.clear();
                pipeline
                    .graph
                    .transaction_external_container_children
                    .clear();
                pipeline
                    .graph
                    .transaction_external_aggregate_children
                    .clear();
                pipeline.graph.transaction_external_cluster_layouts.clear();
                pipeline.graph.projected_transaction_nodes.clear();
                eprintln!("DROP_ALL_EXTERNAL_RUST");
            }
        }
        if crate::internal::trace_env_enabled("TALA_TRACE_HIER_ORDER") {
            eprintln!("RECURSIVELY_PLACED_RUST {}", recursively_placed);
        }
        if matches!(
            stage,
            LayoutStage::NodePlacement
                | LayoutStage::SwapFirstPass
                | LayoutStage::SwapSecondPass
                | LayoutStage::SwapThirdPass
                | LayoutStage::SwapStuff
                | LayoutStage::Transpose
                | LayoutStage::AlignAxes
                | LayoutStage::GapNormalization
                | LayoutStage::OptimizeClusters
                | LayoutStage::BalanceSymmetry
                | LayoutStage::Equidistance
                | LayoutStage::FirstBinPack
                | LayoutStage::Rescale
        ) && !recursively_placed
        {
            // `Graph.placeNodes` directs every independently optimized
            // SplitSubgraphs result before `CombineSubgraphs` translates it
            // into the shared coordinate system. The flat root takes this
            // path too; skipping it leaves the annealer's arbitrary axis
            // orientation visible in the final layout.
            pipeline.graph.direct(false);
            if let Err(error) = pipeline
                .graph
                .combine_placement_components(true, false, false)
            {
                let mut snapshot = pipeline.snapshot(requested_stage);
                snapshot.layout_error = Some(container_orientation::layout_error_for_work(
                    error,
                    seed,
                    "CombineSubgraphs",
                    limits::MAX_ENGINE_WORK_UNITS,
                ));
                return snapshot;
            }
        }
        pipeline.refresh_connected_container_child_order_from_live_slices();
        if matches!(
            stage,
            LayoutStage::NodePlacement
                | LayoutStage::SwapFirstPass
                | LayoutStage::SwapSecondPass
                | LayoutStage::SwapThirdPass
                | LayoutStage::SwapStuff
                | LayoutStage::Transpose
                | LayoutStage::AlignAxes
                | LayoutStage::GapNormalization
                | LayoutStage::OptimizeClusters
                | LayoutStage::BalanceSymmetry
                | LayoutStage::Equidistance
                | LayoutStage::FirstBinPack
                | LayoutStage::Rescale
        ) {
            pipeline.graph.activate_root_sequence_vessels();
            pipeline.trace_node_stage("NodePlacement");
            stage_boundary!("NodePlacement");
        }
        let swap_passes = match stage {
            LayoutStage::SwapFirstPass => 1,
            LayoutStage::SwapSecondPass => 2,
            LayoutStage::SwapThirdPass => 3,
            LayoutStage::SwapStuff => 4,
            LayoutStage::Transpose => 4,
            LayoutStage::AlignAxes => 4,
            LayoutStage::GapNormalization => 4,
            LayoutStage::OptimizeClusters => 4,
            LayoutStage::BalanceSymmetry => 4,
            LayoutStage::Equidistance => 4,
            LayoutStage::FirstBinPack => 4,
            LayoutStage::Rescale => 4,
            _ => 0,
        };
        if swap_passes > 0 {
            // TALA's placement subgraphs own independent lazy score caches.
            // The main graph first materializes getTurnCost/getNonCenterPortCost
            // against the combined NodePlacement geometry used by SwapStuff.
            if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_STATE") {
                eprintln!(
                    "SWAP_STATE_RUST before_init node_order={:?} membership_valid={} graph_order={:?} clusters={:?}",
                    pipeline
                        .graph
                        .node_order
                        .iter()
                        .map(|node| pipeline.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    pipeline.graph.node_order_membership_valid,
                    pipeline
                        .graph
                        .graph_node_order()
                        .iter()
                        .map(|node| pipeline.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    pipeline
                        .graph
                        .clusters
                        .iter()
                        .map(|cluster| (
                            cluster.vessel_tala_id,
                            cluster
                                .members
                                .iter()
                                .map(|node| pipeline.graph.nodes[node.0 as usize].tala_id)
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                );
            }
            pipeline.graph.restore_rejoined_scoring_container_identity();
            pipeline.graph.initialize_rejoined_scoring_costs();
            if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_CLUSTER6") {
                let member = pipeline
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == 6)
                    .map(|node| node.input_id);
                eprintln!(
                    "SWAP_CLUSTER6_RUST phase=before-swaps pos={:?} pending={:?}",
                    member.and_then(|node| pipeline.graph.position(node)),
                    pipeline.graph.pending_cluster_vessel_positions
                );
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_STATE") {
                eprintln!(
                    "SWAP_STATE_RUST after_init node_order={:?} membership_valid={} graph_order={:?}",
                    pipeline
                        .graph
                        .node_order
                        .iter()
                        .map(|node| pipeline.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>(),
                    pipeline.graph.node_order_membership_valid,
                    pipeline
                        .graph
                        .graph_node_order()
                        .iter()
                        .map(|node| pipeline.graph.nodes[node.0 as usize].tala_id)
                        .collect::<Vec<_>>()
                );
            }
        }
        for _ in 0..swap_passes {
            cancellation_boundary!();
            if !pipeline.graph.swap_optimize() {
                break;
            }
            if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_CLUSTER6") {
                let member = pipeline
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == 6)
                    .map(|node| node.input_id);
                eprintln!(
                    "SWAP_CLUSTER6_RUST phase=after-swap pos={:?} pending={:?}",
                    member.and_then(|node| pipeline.graph.position(node)),
                    pipeline.graph.pending_cluster_vessel_positions
                );
            }
        }
        cancellation_boundary!();
        if swap_passes > 0 {
            pipeline.graph.direct(true);
            if crate::internal::trace_env_enabled("TALA_TRACE_SWAP_CLUSTER6") {
                let member = pipeline
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.tala_id == 6)
                    .map(|node| node.input_id);
                eprintln!(
                    "SWAP_CLUSTER6_RUST phase=after-direct pos={:?} pending={:?}",
                    member.and_then(|node| pipeline.graph.position(node)),
                    pipeline.graph.pending_cluster_vessel_positions
                );
            }
            pipeline.trace_node_stage("SwapStuff");
            stage_boundary!("SwapStuff");
        }
        if matches!(
            stage,
            LayoutStage::Transpose
                | LayoutStage::AlignAxes
                | LayoutStage::GapNormalization
                | LayoutStage::OptimizeClusters
                | LayoutStage::BalanceSymmetry
                | LayoutStage::Equidistance
                | LayoutStage::FirstBinPack
                | LayoutStage::Rescale
        ) {
            pipeline.graph.transpose_all();
            cancellation_boundary!();
            pipeline.trace_node_stage("Transpose");
            stage_boundary!("Transpose");
        }
        if matches!(
            stage,
            LayoutStage::AlignAxes
                | LayoutStage::GapNormalization
                | LayoutStage::OptimizeClusters
                | LayoutStage::BalanceSymmetry
                | LayoutStage::Equidistance
                | LayoutStage::FirstBinPack
                | LayoutStage::Rescale
        ) {
            pipeline.run_align_axes(false);
            cancellation_boundary!();
            pipeline.trace_node_stage("AlignAxes");
            stage_boundary!("AlignAxes");
        }
        if matches!(
            stage,
            LayoutStage::GapNormalization
                | LayoutStage::OptimizeClusters
                | LayoutStage::BalanceSymmetry
                | LayoutStage::Equidistance
                | LayoutStage::FirstBinPack
                | LayoutStage::Rescale
        ) {
            match pipeline.gap_normalization_stage() {
                Ok(changed) => pipeline.run_align_axis = changed,
                Err(error) => {
                    pipeline.run_align_axis = false;
                    let mut snapshot = pipeline.snapshot(requested_stage);
                    snapshot.layout_error = Some(error);
                    return snapshot;
                }
            }
            cancellation_boundary!();
            pipeline.trace_node_stage("GapNormalization");
            stage_boundary!("GapNormalization");
        }
        if matches!(
            stage,
            LayoutStage::OptimizeClusters
                | LayoutStage::BalanceSymmetry
                | LayoutStage::Equidistance
                | LayoutStage::FirstBinPack
                | LayoutStage::Rescale
        ) {
            pipeline.run_align_axes(false);
            pipeline.trace_node_stage("AlignAxes");
            stage_boundary!("AlignAxes");
            if let Err(error) = pipeline.optimize_clusters_stage() {
                pipeline.run_align_axis = false;
                let mut snapshot = pipeline.snapshot(requested_stage);
                snapshot.layout_error = Some(error);
                return snapshot;
            }
            cancellation_boundary!();
            if crate::internal::trace_env_enabled("TALA_TRACE_CLUSTER_FLIP") {
                eprintln!(
                    "CLUSTER_OPTIMIZE_GATE_RUST changed={} vessel_states={:?}",
                    pipeline.run_align_axis,
                    pipeline
                        .graph
                        .clusters
                        .iter()
                        .map(|cluster| (cluster.vessel_tala_id, cluster.arrangement))
                        .collect::<Vec<_>>()
                );
            }
            pipeline.trace_node_stage("OptimizeClusters");
            stage_boundary!("OptimizeClusters");
        }
        if matches!(
            stage,
            LayoutStage::BalanceSymmetry
                | LayoutStage::Equidistance
                | LayoutStage::FirstBinPack
                | LayoutStage::Rescale
        ) {
            // NewPipeline schedules another AlignAxes stage immediately
            // after OptimizeClusters and before BalanceSymmetry.
            pipeline.run_align_axes(false);
            cancellation_boundary!();
            pipeline.trace_node_stage("AlignAxes");
            stage_boundary!("AlignAxes");
        }
        if matches!(
            stage,
            LayoutStage::BalanceSymmetry
                | LayoutStage::Equidistance
                | LayoutStage::FirstBinPack
                | LayoutStage::Rescale
        ) {
            pipeline.graph.balance_symmetry();
            cancellation_boundary!();
            pipeline.trace_node_stage("BalanceSymmetry");
            stage_boundary!("BalanceSymmetry");
        }
        if matches!(
            stage,
            LayoutStage::Equidistance | LayoutStage::FirstBinPack | LayoutStage::Rescale
        ) {
            pipeline.equidistance_stage();
            cancellation_boundary!();
            pipeline.trace_node_stage("Equidistance");
            stage_boundary!("Equidistance");
        }
        if matches!(stage, LayoutStage::FirstBinPack | LayoutStage::Rescale) {
            pipeline.run_align_axes(true);
            pipeline.trace_node_stage("AlignAxes");
            stage_boundary!("AlignAxes");
            if let Err(error) = pipeline.graph.bin_pack_recovered() {
                let mut snapshot = pipeline.snapshot(requested_stage);
                snapshot.layout_error = Some(container_orientation::layout_error_for_work(
                    error,
                    seed,
                    "BinPack",
                    binpack_guard::MAX_BIN_PACK_WORK_UNITS,
                ));
                return snapshot;
            }
            pipeline.graph.apply_canvas_positions();
            pipeline.graph.publish_aggregate_projection_geometry();
            cancellation_boundary!();
            pipeline.trace_node_stage("BinPack");
            stage_boundary!("BinPack");
        }
        if stage == LayoutStage::Rescale {
            pipeline.graph.restore_aggregate_members_to_node_order();
            pipeline.trace_node_stage("CleanupStuff");
            stage_boundary!("CleanupStuff");
            pipeline.graph.compute_cell_size();
            pipeline.graph.pad();
            cancellation_boundary!();
            pipeline.trace_node_stage("Rescale");
            stage_boundary!("Rescale");
        }
    }
    if matches!(
        requested_stage,
        LayoutStage::EdgeRouting
            | LayoutStage::Crosshatch
            | LayoutStage::Dejitter
            | LayoutStage::SecondEdgeRouting
            | LayoutStage::SimplifyEdgeRoutes
            | LayoutStage::SwapEdgePorts
            | LayoutStage::StraightEdgesFallback
            | LayoutStage::BalanceEdgeSegments
            | LayoutStage::FixClusterEdgeBranching
            | LayoutStage::TraceEdgesToShapeBorder
            | LayoutStage::ReorderDuplicates
            | LayoutStage::PlaceLabels
            | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_edge_routing_checked() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("EdgeRouting");
        cancellation_boundary!();
    }
    if matches!(
        requested_stage,
        LayoutStage::Crosshatch
            | LayoutStage::Dejitter
            | LayoutStage::SecondEdgeRouting
            | LayoutStage::SimplifyEdgeRoutes
            | LayoutStage::SwapEdgePorts
            | LayoutStage::StraightEdgesFallback
            | LayoutStage::BalanceEdgeSegments
            | LayoutStage::FixClusterEdgeBranching
            | LayoutStage::TraceEdgesToShapeBorder
            | LayoutStage::ReorderDuplicates
            | LayoutStage::PlaceLabels
            | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_crosshatch() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("Crosshatch");
        cancellation_boundary!();
    }
    if matches!(
        requested_stage,
        LayoutStage::Dejitter
            | LayoutStage::SecondEdgeRouting
            | LayoutStage::SimplifyEdgeRoutes
            | LayoutStage::SwapEdgePorts
            | LayoutStage::StraightEdgesFallback
            | LayoutStage::BalanceEdgeSegments
            | LayoutStage::FixClusterEdgeBranching
            | LayoutStage::TraceEdgesToShapeBorder
            | LayoutStage::ReorderDuplicates
            | LayoutStage::PlaceLabels
            | LayoutStage::Normalize
    ) {
        pipeline.run_dejitter();
        stage_boundary!("Dejitter");
        cancellation_boundary!();
    }
    if matches!(
        requested_stage,
        LayoutStage::SecondEdgeRouting
            | LayoutStage::SimplifyEdgeRoutes
            | LayoutStage::SwapEdgePorts
            | LayoutStage::StraightEdgesFallback
            | LayoutStage::BalanceEdgeSegments
            | LayoutStage::FixClusterEdgeBranching
            | LayoutStage::TraceEdgesToShapeBorder
            | LayoutStage::ReorderDuplicates
            | LayoutStage::PlaceLabels
            | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_second_edge_routing() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("EdgeRouting");
        cancellation_boundary!();
    }
    if matches!(
        requested_stage,
        LayoutStage::SimplifyEdgeRoutes
            | LayoutStage::SwapEdgePorts
            | LayoutStage::StraightEdgesFallback
            | LayoutStage::BalanceEdgeSegments
            | LayoutStage::FixClusterEdgeBranching
            | LayoutStage::TraceEdgesToShapeBorder
            | LayoutStage::ReorderDuplicates
            | LayoutStage::PlaceLabels
            | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_simplify_edge_routes_checked() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("SimplifyEdgeRoutes");
        cancellation_boundary!();
    }
    if matches!(
        requested_stage,
        LayoutStage::SwapEdgePorts
            | LayoutStage::StraightEdgesFallback
            | LayoutStage::BalanceEdgeSegments
            | LayoutStage::FixClusterEdgeBranching
            | LayoutStage::TraceEdgesToShapeBorder
            | LayoutStage::ReorderDuplicates
            | LayoutStage::PlaceLabels
            | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_swap_edge_ports_checked() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("SwapEdgePorts");
        cancellation_boundary!();
    }
    if matches!(
        requested_stage,
        LayoutStage::StraightEdgesFallback
            | LayoutStage::BalanceEdgeSegments
            | LayoutStage::FixClusterEdgeBranching
            | LayoutStage::TraceEdgesToShapeBorder
            | LayoutStage::ReorderDuplicates
            | LayoutStage::PlaceLabels
            | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_straight_edges_fallback_checked() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("StraightEdgesFallback");
        cancellation_boundary!();
    }
    if matches!(
        requested_stage,
        LayoutStage::BalanceEdgeSegments
            | LayoutStage::FixClusterEdgeBranching
            | LayoutStage::TraceEdgesToShapeBorder
            | LayoutStage::ReorderDuplicates
            | LayoutStage::PlaceLabels
            | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_balance_edge_segments_checked() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("BalanceEdgeSegments");
        cancellation_boundary!();
    }
    if matches!(
        requested_stage,
        LayoutStage::FixClusterEdgeBranching
            | LayoutStage::TraceEdgesToShapeBorder
            | LayoutStage::ReorderDuplicates
            | LayoutStage::PlaceLabels
            | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_fix_cluster_edge_branching_checked() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("FixClusterEdgeBranching");
        cancellation_boundary!();
    }
    if matches!(
        requested_stage,
        LayoutStage::TraceEdgesToShapeBorder
            | LayoutStage::ReorderDuplicates
            | LayoutStage::PlaceLabels
            | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_trace_edges_to_shape_border_checked() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("TraceEdgesToShapeBorder");
        cancellation_boundary!();
    }
    if matches!(
        requested_stage,
        LayoutStage::ReorderDuplicates | LayoutStage::PlaceLabels | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_reorder_duplicates_checked() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        cancellation_boundary!();
        pipeline.trace_node_stage("ReorderDuplicates");
        stage_boundary!("ReorderDuplicates");
    }
    if matches!(
        requested_stage,
        LayoutStage::PlaceLabels | LayoutStage::Normalize
    ) {
        if let Err(error) = pipeline.run_second_bin_pack() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(container_orientation::layout_error_for_work(
                error,
                seed,
                "BinPack",
                binpack_guard::MAX_BIN_PACK_WORK_UNITS,
            ));
            return snapshot;
        }
        pipeline.trace_node_stage("BinPack");
        stage_boundary!("BinPack");
        if let Err(error) = pipeline.run_place_labels() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        pipeline.trace_node_stage("PlaceLabels");
        stage_boundary!("PlaceLabels");
        if let Err(error) = pipeline.run_nudge_edge_channels_checked() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("NudgeEdgeChannels");
        if let Err(error) = pipeline.run_shortcut_edge_routes_checked() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        stage_boundary!("ShortcutEdgeRoutes");
        cancellation_boundary!();
    }
    if requested_stage == LayoutStage::Normalize {
        if let Err(error) = pipeline.run_normalize() {
            let mut snapshot = pipeline.snapshot(requested_stage);
            snapshot.layout_error = Some(error);
            return snapshot;
        }
        cancellation_boundary!();
        pipeline.trace_node_stage("Normalize");
        stage_boundary!("Normalize");
    }
    pipeline.snapshot(requested_stage)
}

pub(super) fn prepare_compound_candidate_state(graph: &mut ArenaGraph) {
    // layoutgraph.Clone creates a fresh cache and clears private fixed-label
    // bookkeeping. Keep the saved incumbent's cache intact on rejection.
    graph.detach_cost_owner();
    for node in &mut graph.nodes {
        node.loop_offsets = None;
        node.label_position_fixed = false;
        node.icon_position_fixed = false;
    }
    // Graph.Clone recreates every reachable source node record, including
    // external nodes retained by temporary projection views. Ordinary
    // geometry-trial clones keep their live maps instead.
    for node in graph
        .transaction_external_containers
        .iter_mut()
        .chain(graph.transaction_validation_external_containers.iter_mut())
    {
        node.loop_offsets = None;
    }
    for nodes in graph
        .transaction_external_container_children
        .values_mut()
        .chain(graph.trace_container_children.values_mut())
        .chain(graph.transaction_external_aggregate_children.values_mut())
    {
        for node in nodes {
            node.loop_offsets = None;
        }
    }
    for node in &mut graph.projected_transaction_nodes {
        node.node.loop_offsets = None;
    }
    for edge_index in graph.member_edge_indices() {
        let edge = &mut graph.edge_records[edge_index];
        edge.label_position_fixed = false;
    }
}

fn reset_compound_route_state(graph: &mut ArenaGraph) {
    // upstream engine.resetFullLayoutRouteState also invalidates placement
    // costs: rigid block movement makes the ordinary candidate's cache stale.
    graph.cost_cache.reset_placement();
    for edge_index in graph.member_edge_indices() {
        let edge = &mut graph.edge_records[edge_index];
        edge.route.replace(RoutePoints::nil());
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
}

fn graph_race_score(graph: &ArenaGraph) -> Result<evaluation::RaceScoreBreakdown, LayoutError> {
    let edge_order = graph
        .edge_order
        .read()
        .iter()
        .map(|edge_id| graph.edge_records[edge_id.0 as usize].input_id)
        .collect::<Vec<_>>();
    let surviving_edges = edge_order.iter().copied().collect::<BTreeSet<_>>();
    let routes = graph
        .member_edges()
        .filter(|edge| surviving_edges.contains(&edge.input_id))
        .map(|edge| (edge.input_id, edge.route.read().clone()))
        .collect::<BTreeMap<_, _>>();
    let clustered_edges = graph
        .member_edges()
        .filter(|edge| surviving_edges.contains(&edge.input_id))
        .filter(|edge| {
            graph.nodes[edge.from.0 as usize].cluster.is_some()
                || graph.nodes[edge.to.0 as usize].cluster.is_some()
        })
        .map(|edge| edge.input_id)
        .collect::<BTreeSet<_>>();
    Ok(evaluation::evaluate_race_seed_layout(
        &routes,
        &clustered_edges,
        labels::score_existing_label_placements_checked(graph)?,
        &edge_order,
    ))
}

fn graph_race_selection_score(
    graph: &ArenaGraph,
) -> Result<evaluation::RaceSelectionScore, LayoutError> {
    let race = graph_race_score(graph)?;
    Ok(evaluation::RaceSelectionScore {
        penalty: race.selection_penalty(),
        area: evaluation::race_seed_graph_area(graph),
    })
}

fn refresh_seed_layout_result(
    input: &Graph,
    seed: i64,
    graph: &ArenaGraph,
    result: &mut LayoutResult,
) -> Result<evaluation::RaceSelectionScore, LayoutError> {
    result.boxes = graph
        .nodes
        .iter()
        .filter(|node| {
            graph.topology_arena_id().is_none()
                || graph.topology_source_input_node(node.input_id).is_some()
        })
        .filter_map(|node| {
            Some((
                node.input_id,
                Rect {
                    // buildObjectPatch copies the selected node's live
                    // TopLeft. Compound placement/normalization updates the
                    // owner point; the arena record may retain its old copy.
                    origin: graph.position(node.input_id)?,
                    size: node.rect.size,
                },
            ))
        })
        .collect();
    result.routes = graph
        .member_edges()
        .map(|edge| (edge.input_id, edge.route.read().clone()))
        .collect();
    result.edge_labels = graph
        .member_edges()
        .filter_map(|edge| edge.label.clone().map(|label| (edge.input_id, label)))
        .collect();
    result.node_labels = graph
        .nodes
        .iter()
        .map(|node| {
            (
                node.input_id,
                NodeLabelState {
                    font_size_committed: true,
                    size: node.label_size,
                    font_size: node.font_size,
                    position: node.label_position,
                    icon_position: node.icon_position,
                },
            )
        })
        .collect();
    let score = evaluation::evaluate_layout(input, &result.boxes, &result.routes);
    let race = graph_race_score(graph)?;
    let selection = evaluation::RaceSelectionScore {
        penalty: race.selection_penalty(),
        area: evaluation::race_seed_graph_area(graph),
    };
    result.report.selected_seed = seed;
    result.report.score = score.clone();
    if let Some(candidate) = result.report.candidates.first_mut() {
        candidate.seed = seed;
        candidate.score = score;
        candidate.accepted = candidate.score.invalidities == 0;
        candidate.race_score = Some(selection.penalty);
        candidate.race_score_components = Some(RaceScoreComponents {
            route_turns: race.route_turns,
            diagonal_segments: race.diagonal_segments,
            crossings: race.crossings,
            label_score: race.label_score,
            label_penalty: race.label_penalty,
            selection_area: selection.area,
        });
        candidate.race_seed_edge_order = Some(
            graph
                .edge_order
                .read()
                .iter()
                .map(|edge_id| graph.edge_records[edge_id.0 as usize].input_id)
                .collect(),
        );
    }
    Ok(selection)
}

fn run_compound_label_stage(
    pipeline: &mut Pipeline,
    cancellation: Option<&CancellationToken>,
) -> Result<(), LayoutError> {
    // compound_candidate.go checks the explicit caller before entering each
    // stage. Compound rerouting runs after the seed worker's TLS scope ends.
    if let Some(error) = cancellation.and_then(CancellationToken::error) {
        return Err(error);
    }
    with_cancellation(cancellation, || pipeline.run_place_labels())
}

/// `evaluateSeedResult` checks authored label/icon positions after each
/// optional refinement. Clone clears the private fixed-position flags, so
/// rerouting can move a label even when the ordinary seed preserved it.
fn refinement_preserves_authored_positions(input: &Graph, graph: &ArenaGraph) -> bool {
    input.nodes().all(|(id, node)| {
        let actual = &graph.nodes[id.0 as usize];
        let fixed_label = actual.label_text_present
            && node.label_position != LabelPosition::Unset
            && !node.external_label.is_some_and(|label| label.automatic);
        let fixed_icon = node.has_icon
            && node
                .icon_position
                .is_some_and(|position| position != LabelPosition::Unset);
        (!fixed_label || actual.label_position == node.label_position)
            && (!fixed_icon || actual.icon_position == node.icon_position)
    })
}

/// Apply D2 v0.9.0's one post-selection rigid-container proposal. The
/// proposal is fully isolated, rerouted, and scored before it can replace the
/// ordinary winning seed. Every `Compound/...` event is diagnostic-only.
fn build_compound_candidate(
    input: &Graph,
    winner: &SeedCandidate<SeedLayout>,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<SeedCandidate<SeedLayout>>, LayoutError> {
    #[cfg(test)]
    compound_cancellation_api_tests::checkpoint(cancellation);
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(cancellation
            .as_ref()
            .and_then(|token| token.error())
            .unwrap_or(LayoutError::Cancelled));
    }
    let mut candidate_graph = winner.value.graph.clone();
    prepare_compound_candidate_state(&mut candidate_graph);
    // TALA constructs the compound-placement RNG before `reroutePlaced`
    // creates its route-only pipeline. Preserve that constructor order.
    if !candidate_graph.apply_compound_flow() {
        return Ok(None);
    }
    #[cfg(test)]
    compound_cancellation_api_tests::checkpoint(cancellation);
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(cancellation
            .as_ref()
            .and_then(|token| token.error())
            .unwrap_or(LayoutError::Cancelled));
    }
    let mut candidate = Pipeline::new(input, 0, true, true);
    candidate.graph = candidate_graph;
    // A proposal with worse perpendicular alignment is rejected before the
    // expensive reroute, matching engine.CompoundCandidate.
    if candidate.graph.compound_cross_axis_detours()
        > winner.value.graph.compound_cross_axis_detours()
    {
        return Ok(None);
    }

    candidate.suppress_route_trace = true;
    reset_compound_route_state(&mut candidate.graph);
    candidate.emit_normalized_trace("Compound/ResetRoutes");
    let stages: [(&str, fn(&mut Pipeline)); 8] = [
        ("EdgeRouting", Pipeline::run_edge_routing),
        ("SimplifyEdgeRoutes", Pipeline::run_simplify_edge_routes),
        ("SwapEdgePorts", Pipeline::run_swap_edge_ports),
        (
            "StraightEdgesFallback",
            Pipeline::run_straight_edges_fallback,
        ),
        ("BalanceEdgeSegments", Pipeline::run_balance_edge_segments),
        (
            "FixClusterEdgeBranching",
            Pipeline::run_fix_cluster_edge_branching,
        ),
        (
            "TraceEdgesToShapeBorder",
            Pipeline::run_trace_edges_to_shape_border,
        ),
        ("ReorderDuplicates", Pipeline::run_reorder_duplicates),
    ];
    for (name, run) in stages {
        #[cfg(test)]
        compound_cancellation_api_tests::checkpoint(cancellation);
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            return Err(cancellation
                .as_ref()
                .and_then(|token| token.error())
                .unwrap_or(LayoutError::Cancelled));
        }
        run(&mut candidate);
        candidate.emit_normalized_trace(&format!("Compound/{name}"));
    }
    run_compound_label_stage(&mut candidate, cancellation)?;
    candidate.emit_normalized_trace("Compound/PlaceLabels");
    if candidate
        .graph
        .nodes
        .iter()
        .all(|node| node.fixed_top_left.is_none())
    {
        candidate.graph.normalize_seed()?;
    }
    candidate.emit_normalized_trace("Compound/Normalize");
    #[cfg(test)]
    compound_cancellation_api_tests::checkpoint(cancellation);
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(cancellation
            .as_ref()
            .and_then(|token| token.error())
            .unwrap_or(LayoutError::Cancelled));
    }
    if candidate
        .stage_graph_bounds()
        .is_some_and(|(tl, br)| br.x - tl.x > 30_000.0 || br.y - tl.y > 30_000.0)
    {
        return Ok(None);
    }

    // result_validate.go rejects a proposal that changes an authored label
    // or icon position, even when its completed geometry scores better.
    if !refinement_preserves_authored_positions(input, &candidate.graph) {
        return Ok(None);
    }

    let candidate_score = graph_race_selection_score(&candidate.graph)?;
    let incumbent_score = winner.score;
    if candidate_score.compare(incumbent_score) != CandidateOrdering::Less {
        return Ok(None);
    }
    let mut result = winner.value.result.clone();
    refresh_seed_layout_result(input, winner.seed, &candidate.graph, &mut result)?;
    Ok(Some(SeedCandidate {
        index: winner.index,
        seed: winner.seed,
        score: candidate_score,
        value: SeedLayout {
            result,
            graph: candidate.graph,
        },
    }))
}

/// Upstream layout.go's PreserveCompoundRoutes + evaluateSeedResult build for
/// refineSeedResult. Unlike the compound proposal, the refined drawing is not
/// compared with the selected score: placement is already chosen.
fn build_preserved_compound_routes(
    input: &Graph,
    ordinary: &SeedCandidate<SeedLayout>,
    selected: &SeedCandidate<SeedLayout>,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<SeedCandidate<SeedLayout>>, LayoutError> {
    let graph = compound_routes::preserve_compound_routes(
        input,
        &ordinary.value.graph,
        &selected.value.graph,
        cancellation,
    )?;
    if !refinement_preserves_authored_positions(input, &graph) {
        return Ok(None);
    }
    let mut result = selected.value.result.clone();
    let score = refresh_seed_layout_result(input, selected.seed, &graph, &mut result)?;
    // evaluateSeedResult rejects a non-finite score.
    if !score.penalty.is_finite() {
        return Err(LayoutError::SeedInvariant);
    }
    Ok(Some(SeedCandidate {
        index: selected.index,
        seed: selected.seed,
        score,
        value: SeedLayout { result, graph },
    }))
}

// Original layout.go considerSeedCandidate: an optional failure/panic keeps the
// incumbent; a context failure takes precedence even after a successful build.
// Build against a read-only incumbent and publish only a completed proposal.
fn consider_optional_proposal<T>(
    cancellation: Option<&CancellationToken>,
    build: impl FnOnce() -> Result<Option<T>, LayoutError>,
) -> Result<Option<T>, LayoutError> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(build));
    if let Some(error) = cancellation.and_then(CancellationToken::error) {
        return Err(error);
    }
    fn is_context(error: &LayoutError) -> bool {
        match error {
            LayoutError::Cancelled | LayoutError::DeadlineExceeded => true,
            LayoutError::Context { source, .. } => is_context(source),
            _ => false,
        }
    }
    match result {
        Ok(Ok(proposal)) => Ok(proposal),
        Ok(Err(error)) if is_context(&error) => Err(error),
        Ok(Err(_)) | Err(_) => Ok(None),
    }
}

/// Runs the recovered pipeline as an ordinary layout result.
pub(crate) fn layout_result(
    input: &Graph,
    options: &LayoutOptions,
) -> Result<LayoutResult, LayoutError> {
    layout_result_with_cancellation(input, options, None)
}

/// Runs the recovered pipeline while observing a caller-owned cancellation
/// token at seed, stage, routing, and work-guard boundaries.
pub(crate) fn layout_result_with_cancellation(
    input: &Graph,
    options: &LayoutOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<LayoutResult, LayoutError> {
    #[cfg(feature = "diagnostic-traces")]
    let _count_dump = crate::internal::call_counts::Dump;
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(cancellation
            .as_ref()
            .and_then(|token| token.error())
            .unwrap_or(LayoutError::Cancelled));
    }
    // d2talalayout.Layout checks cancellation before layoutPlan, and checks
    // that plan before constructing/validating the measured seed input.
    // Source Graph.Data overrides caller seeds even in extracted scopes.
    let configured_options;
    let options = if let Some(data) = &input.tala_seed_data {
        configured_options = LayoutOptions {
            seeds: data.clone().map_err(LayoutError::InvalidSeedData)?,
            max_concurrency: options.max_concurrency,
        };
        &configured_options
    } else {
        options
    };
    let seeds = options.normalized_seeds()?;
    let max_concurrency = options.effective_max_concurrency(seeds.len())?;
    validation::validate(input)?;
    // adapter.go constructs source-only edges as Dst -> Src before any
    // placement, scoring or routing. Publish in the caller's declaration
    // orientation only after the complete candidate/compound seed race.
    let canonical = canonical_tala_input(input);
    let mut result = race_seed_layouts(&canonical, &seeds, max_concurrency, cancellation.cloned())?;
    restore_declared_edge_orientation(input, &mut result.routes, &mut result.edge_labels);
    Ok(result)
}

struct SeedCandidate<T> {
    index: usize,
    seed: i64,
    score: evaluation::RaceSelectionScore,
    value: T,
}

/// The completed ordinary candidate and its arena state. The arena is kept
/// only inside the seed race so the adapter can run one post-selection
/// `CompoundCandidate` trial without replaying layout or changing tracing.
struct SeedLayout {
    result: LayoutResult,
    graph: ArenaGraph,
}

struct SeedRace<T> {
    winner: SeedCandidate<T>,
    received: Vec<SeedCandidate<T>>,
}

fn layout_result_for_seed(
    input: &Graph,
    seed: i64,
    cancellation: Option<&CancellationToken>,
) -> Result<SeedCandidate<SeedLayout>, LayoutError> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(cancellation
            .as_ref()
            .and_then(|token| token.error())
            .unwrap_or(LayoutError::Cancelled));
    }
    let mut snapshot = with_cancellation(cancellation, || {
        layout_snapshot(input, seed, LayoutStage::Normalize)
    });
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(cancellation
            .as_ref()
            .and_then(|token| token.error())
            .unwrap_or(LayoutError::Cancelled));
    }
    if let Some(error) = snapshot.layout_error.take() {
        return Err(error);
    }
    let boxes = snapshot
        .nodes
        .iter()
        .map(|node| (node.node, node.rect))
        .collect::<BTreeMap<_, _>>();
    let routes = snapshot
        .edges
        .iter()
        .map(|edge| (edge.edge, edge.points.clone()))
        .collect::<BTreeMap<_, _>>();
    // d2talalayout.buildLayoutPatch (patch.go:215–239) replaces Graph.Edges
    // with the surviving input edges. BuildSequence has permanently removed
    // the defining step connections; a missing route is not an exported edge.
    let removed_edges = input
        .edges()
        .map(|(id, _)| id)
        .filter(|id| !routes.contains_key(id))
        .collect();
    let edge_labels = snapshot
        .edges
        .iter()
        .filter_map(|edge| edge.label.clone().map(|label| (edge.edge, label)))
        .collect();
    let node_labels = snapshot
        .nodes
        .iter()
        .map(|node| {
            (
                node.node,
                NodeLabelState {
                    font_size_committed: true,
                    size: node.label_size,
                    font_size: node.font_size,
                    position: node.label_position,
                    icon_position: node.icon_position,
                },
            )
        })
        .collect();
    let score = evaluation::evaluate_layout(input, &boxes, &routes);
    let graph = snapshot
        .graph
        .take()
        .ok_or(LayoutError::TimedOut("missing completed TALA graph"))?;
    let race_breakdown = graph_race_score(&graph)?;
    let selection_score = evaluation::RaceSelectionScore {
        penalty: race_breakdown.selection_penalty(),
        area: evaluation::race_seed_graph_area(&graph),
    };
    let race_score = selection_score.penalty;
    let race_seed_edge_order = graph
        .edge_order
        .read()
        .iter()
        .map(|edge_id| graph.edge_records[edge_id.0 as usize].input_id)
        .collect();
    Ok(SeedCandidate {
        index: 0,
        seed,
        score: selection_score,
        value: SeedLayout {
            graph,
            result: LayoutResult {
                removed_edges,
                source_edge_storage: d2_rs_core::SourceEdgeStorage::Replaced,
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                edge_label_percentage_commits: Default::default(),
                boxes,
                routes,
                edge_labels,
                node_labels,
                report: LayoutReport {
                    selected_seed: seed,
                    selected_pass: 0,
                    score: score.clone(),
                    improvement_passes: 1,
                    candidates: vec![CandidateReport {
                        seed,
                        pass: 0,
                        accepted: score.invalidities == 0,
                        score,
                        race_score: Some(race_score),
                        race_score_components: Some(RaceScoreComponents {
                            route_turns: race_breakdown.route_turns,
                            diagonal_segments: race_breakdown.diagonal_segments,
                            crossings: race_breakdown.crossings,
                            label_score: race_breakdown.label_score,
                            label_penalty: race_breakdown.label_penalty,
                            selection_area: selection_score.area,
                        }),
                        race_seed_edge_order: Some(race_seed_edge_order),
                    }],
                    warnings: Vec::new(),
                },
            },
        },
    })
}

fn candidate_is_better<T>(candidate: &SeedCandidate<T>, incumbent: &SeedCandidate<T>) -> bool {
    match candidate.score.compare(incumbent.score) {
        CandidateOrdering::Less => true,
        CandidateOrdering::Equal => candidate.index > incumbent.index,
        CandidateOrdering::Greater => false,
    }
}

fn race_seed_layouts(
    input: &Graph,
    seeds: &[i64],
    max_concurrency: usize,
    cancellation: Option<CancellationToken>,
) -> Result<LayoutResult, LayoutError> {
    let input = std::sync::Arc::new(input.clone());
    let worker_input = std::sync::Arc::clone(&input);
    let worker_cancellation = cancellation.clone();
    let race = race_seed_candidates_with_cancellation(
        seeds,
        max_concurrency,
        cancellation.clone(),
        move |index, seed| {
            let input = std::sync::Arc::clone(&worker_input);
            let mut candidate = layout_result_for_seed(&input, seed, worker_cancellation.as_ref())?;
            candidate.index = index;
            Ok(candidate)
        },
    )?;

    let mut winner = race.winner;
    let mut candidates = race
        .received
        .into_iter()
        .map(|candidate| {
            let mut report = candidate.value.result.report.candidates[0].clone();
            report.seed = candidate.seed;
            (candidate.index, report)
        })
        .collect::<Vec<_>>();
    candidates.push((winner.index, {
        let mut report = winner.value.result.report.candidates[0].clone();
        report.seed = winner.seed;
        report
    }));
    candidates.sort_by_key(|(index, _)| *index);
    let candidates = candidates
        .into_iter()
        .map(|(_, report)| report)
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    if candidates.len() < seeds.len() {
        warnings.push(format!(
            "RaceSeeds completed with {}/{} successful candidates",
            candidates.len(),
            seeds.len()
        ));
    }
    // D2 performs one deterministic rigid-container trial only after the
    // ordinary seed race has selected its incumbent. It is deliberately
    // outside the worker closure: running it per seed would consume a new
    // stream and would make completion order observable.
    if let Some(proposal) = consider_optional_proposal(cancellation.as_ref(), || {
        build_compound_candidate(&input, &winner, cancellation.as_ref())
    })? {
        let ordinary = std::mem::replace(&mut winner, proposal);
        // Upstream layout.go (b34d2e936): once the compound proposal is
        // selected, refineSeedResult runs PreserveCompoundRoutes against the
        // ordinary graph and accepts it unconditionally unless it fails.
        if let Some(refined) = consider_optional_proposal(cancellation.as_ref(), || {
            build_preserved_compound_routes(&input, &ordinary, &winner, cancellation.as_ref())
        })? {
            winner = refined;
        }
    }
    winner.value.result.report = LayoutReport {
        selected_seed: winner.seed,
        selected_pass: 0,
        score: winner.value.result.report.score.clone(),
        improvement_passes: candidates.len(),
        candidates,
        warnings,
    };
    winner.value.result.edge_label_percentage_commits =
        winner.value.result.edge_labels.keys().copied().collect();
    #[cfg(feature = "diagnostic-traces")]
    Pipeline::trace_selected_result(&input, winner.seed, winner.value.graph);
    #[cfg(feature = "diagnostic-traces")]
    trace_label_commit(
        &input,
        &winner.value.result.edge_labels,
        &winner.value.result.edge_label_percentage_commits,
        "layout",
    );
    Ok(winner.value.result)
}

#[cfg(not(target_arch = "wasm32"))]
fn race_seed_candidates_with_cancellation<T, F>(
    seeds: &[i64],
    max_concurrency: usize,
    cancellation: Option<CancellationToken>,
    run_candidate: F,
) -> Result<SeedRace<T>, LayoutError>
where
    T: Send + 'static,
    F: Fn(usize, i64) -> Result<SeedCandidate<T>, LayoutError> + Send + Sync + 'static,
{
    if seeds.is_empty() {
        return Err(LayoutError::NoSeeds);
    }

    let (sender, receiver) = mpsc::sync_channel(seeds.len());
    let run_candidate = std::sync::Arc::new(run_candidate);

    // D2 bounds simultaneous local seed attempts. Keep the input order in a
    // shared FIFO while allowing each worker to finish independently; the
    // selection reduction below still uses the configured index, so completion
    // order cannot change the winner.
    let jobs = std::sync::Arc::new(std::sync::Mutex::new(
        seeds.iter().copied().enumerate().collect::<VecDeque<_>>(),
    ));
    for _ in 0..max_concurrency.min(seeds.len()).max(1) {
        let sender = sender.clone();
        let run_candidate = std::sync::Arc::clone(&run_candidate);
        let jobs = std::sync::Arc::clone(&jobs);
        thread::spawn(move || {
            loop {
                let job = jobs.lock().ok().and_then(|mut jobs| jobs.pop_front());
                let Some((index, seed)) = job else {
                    break;
                };
                // Original runLocalSeed recovers each attempt independently. A
                // failed attempt must still be sent, and this worker continues
                // draining its queued seeds.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_candidate(index, seed)
                }))
                .unwrap_or(Err(LayoutError::SeedInvariant));
                if sender.send((index, seed, result)).is_err() {
                    break;
                }
            }
        });
    }
    drop(sender);

    let mut best_position = None;
    let mut received = Vec::with_capacity(seeds.len());
    let mut attempt_errors = vec![None; seeds.len()];
    let mut completed = 0;
    let mut cancellation_observed = false;

    // D2 v0.9.0's coordinateLocalSeeds drains every configured attempt before
    // reducing the completed results. Do not let worker completion timing
    // select a partial seed set; a caller deadline is conveyed through the
    // cancellation token and checked by each candidate's work guards.
    while completed < seeds.len() {
        cancellation_observed |= cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled);
        let result = if cancellation.is_some() {
            receiver.recv_timeout(CANCELLATION_POLL_INTERVAL)
        } else {
            receiver
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };

        match result {
            Ok((_, _, Ok(candidate))) => {
                if best_position
                    .is_none_or(|position| candidate_is_better(&candidate, &received[position]))
                {
                    best_position = Some(received.len());
                }
                received.push(candidate);
                completed += 1;
            }
            Ok((index, seed, Err(error))) => {
                attempt_errors[index] = Some((seed, error));
                completed += 1;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    cancellation_observed |= cancellation
        .as_ref()
        .is_some_and(CancellationToken::is_cancelled);
    if cancellation_observed {
        return Err(LayoutError::Context {
            context: "tala layout stopped before all seeds completed",
            source: Box::new(
                cancellation
                    .as_ref()
                    .and_then(|token| token.error())
                    .unwrap_or(LayoutError::Cancelled),
            ),
        });
    }
    let Some(best_position) = best_position else {
        let attempts = attempt_errors.into_iter().flatten().collect::<Vec<_>>();
        return Err(if attempts.is_empty() {
            LayoutError::TimedOut("Autolayout")
        } else {
            LayoutError::AllSeedAttemptsFailed { attempts }
        });
    };
    let winner = received.swap_remove(best_position);
    Ok(SeedRace { winner, received })
}

#[cfg(not(target_arch = "wasm32"))]
fn race_seed_candidates<T, F>(
    seeds: &[i64],
    max_concurrency: usize,
    run_candidate: F,
) -> Result<SeedRace<T>, LayoutError>
where
    T: Send + 'static,
    F: Fn(usize, i64) -> Result<SeedCandidate<T>, LayoutError> + Send + Sync + 'static,
{
    race_seed_candidates_with_cancellation(seeds, max_concurrency, None, run_candidate)
}

/// Browsers do not provide the blocking threads used by the native seed
/// coordinator. Evaluate every candidate in request order and use the same
/// score/tie reduction instead. This keeps the WASM API synchronous and usable
/// without cross-origin-isolated shared memory.
#[cfg(target_arch = "wasm32")]
fn race_seed_candidates_with_cancellation<T, F>(
    seeds: &[i64],
    max_concurrency: usize,
    cancellation: Option<CancellationToken>,
    run_candidate: F,
) -> Result<SeedRace<T>, LayoutError>
where
    F: Fn(usize, i64) -> Result<SeedCandidate<T>, LayoutError>,
{
    if seeds.is_empty() {
        return Err(LayoutError::NoSeeds);
    }

    let mut best_position = None;
    let mut received = Vec::with_capacity(seeds.len());
    let mut attempt_errors = vec![None; seeds.len()];

    let _ = max_concurrency;
    for (index, seed) in seeds.iter().copied().enumerate() {
        match run_candidate(index, seed) {
            Ok(candidate) => {
                if best_position
                    .is_none_or(|position| candidate_is_better(&candidate, &received[position]))
                {
                    best_position = Some(received.len());
                }
                received.push(candidate);
            }
            Err(error) => attempt_errors[index] = Some((seed, error)),
        }
    }

    if cancellation
        .as_ref()
        .is_some_and(CancellationToken::is_cancelled)
    {
        return Err(LayoutError::Context {
            context: "tala layout stopped before all seeds completed",
            source: Box::new(
                cancellation
                    .as_ref()
                    .and_then(|token| token.error())
                    .unwrap_or(LayoutError::Cancelled),
            ),
        });
    }
    let Some(best_position) = best_position else {
        let attempts = attempt_errors.into_iter().flatten().collect::<Vec<_>>();
        return Err(if attempts.is_empty() {
            LayoutError::TimedOut("Autolayout")
        } else {
            LayoutError::AllSeedAttemptsFailed { attempts }
        });
    };
    let winner = received.swap_remove(best_position);
    Ok(SeedRace { winner, received })
}

#[cfg(target_arch = "wasm32")]
fn race_seed_candidates<T, F>(
    seeds: &[i64],
    max_concurrency: usize,
    run_candidate: F,
) -> Result<SeedRace<T>, LayoutError>
where
    F: Fn(usize, i64) -> Result<SeedCandidate<T>, LayoutError>,
{
    race_seed_candidates_with_cancellation(seeds, max_concurrency, None, run_candidate)
}

pub(crate) fn route_existing(
    input: &Graph,
    options: &LayoutOptions,
    requested: &[EdgeId],
) -> Result<LayoutResult, LayoutError> {
    route_existing_with_cancellation(input, options, requested, None)
}

pub(crate) fn route_existing_with_cancellation(
    input: &Graph,
    options: &LayoutOptions,
    requested: &[EdgeId],
    cancellation: Option<&CancellationToken>,
) -> Result<LayoutResult, LayoutError> {
    with_cancellation(cancellation, || {
        route_existing_impl(input, options, requested, cancellation)
    })
}

// adapter.go translates a source-only arrow as Dst -> Src. Keep the
// caller's IDs/topology intact and restore its route orientation on output.
fn canonical_tala_input(input: &Graph) -> Cow<'_, Graph> {
    if !input.edges().any(|(id, _)| {
        let arrows = input.edge_arrows(id);
        arrows.source && !arrows.target
    }) {
        return Cow::Borrowed(input);
    }
    let mut canonical = input.clone();
    for (id, edge) in input.edges() {
        let arrows = input.edge_arrows(id);
        if !arrows.source || arrows.target {
            continue;
        }
        canonical.edges[id.0 as usize] = crate::Edge {
            source: edge.target,
            target: edge.source,
        };
        let arrowheads = input.edge_arrowheads(id);
        let target = arrowheads.source.unwrap_or_else(|| "triangle".to_owned());
        canonical.set_edge_arrows(
            id,
            crate::EdgeArrows {
                source: false,
                target: target != "none",
            },
        );
        canonical.set_edge_arrowheads(
            id,
            crate::EdgeArrowheads {
                source: Some("none".to_owned()),
                target: Some(target),
            },
        );
        let labels = input.edge_arrowhead_labels(id);
        canonical.set_edge_arrowhead_labels(
            id,
            crate::EdgeArrowheadLabels {
                source: labels.target,
                target: labels.source,
            },
        );
        let columns = input.edge_table_columns(id);
        canonical.set_edge_table_columns(
            id,
            crate::EdgeTableColumns {
                source: columns.target,
                target: columns.source,
            },
        );
        let mut route = input.edge_route(id).to_vec();
        route.reverse();
        canonical.set_edge_route(id, route);
        if let Some(mut label) = input.edge_label(id).cloned() {
            label.position = label.position.mirrored();
            if label.position != LabelPosition::Unset
                && (input.edge_label_percentage_present(id)
                    || matches!(
                        label.position,
                        LabelPosition::UnlockedTop
                            | LabelPosition::UnlockedMiddle
                            | LabelPosition::UnlockedBottom
                    ))
            {
                label.percentage = 1.0 - label.percentage;
            }
            canonical.set_edge_label(id, Some(label));
        }
    }
    Cow::Owned(canonical)
}

fn restore_declared_edge_orientation(
    input: &Graph,
    routes: &mut BTreeMap<EdgeId, Vec<Point>>,
    labels: &mut BTreeMap<EdgeId, EdgeLabel>,
) {
    for (id, _) in input.edges() {
        let arrows = input.edge_arrows(id);
        if !arrows.source || arrows.target {
            continue;
        }
        if let Some(route) = routes.get_mut(&id) {
            route.reverse();
        }
        if let Some(label) = labels.get_mut(&id) {
            label.position = label.position.mirrored();
            label.percentage = 1.0 - label.percentage;
        }
    }
}

#[cfg(feature = "diagnostic-traces")]
pub(super) fn trace_standalone_stage(graph: &ArenaGraph, stage: &str) {
    if !std::env::var_os("D2_TRACE_ROUTER_STAGE").is_some_and(|value| value == "1") {
        return;
    }
    eprintln!("ROUTER_STAGE begin stage={stage}");
    for id in &graph.edge_order.read() {
        let edge = &graph.edge_records[id.0 as usize];
        let points = edge
            .route
            .read()
            .iter()
            .map(|p| format!("{:016x},{:016x}", p.x.to_bits(), p.y.to_bits()))
            .collect::<Vec<_>>()
            .join(";");
        eprintln!("ROUTER_STAGE edge={} points={points}", edge.tala_id);
    }
    eprintln!("ROUTER_STAGE end");
}

fn initialize_standalone_node_state(graph: &mut ArenaGraph, input: &Graph) {
    // translateGraph(loadExistingLayout=true) creates a fresh Graph and does
    // not preprocess placement trees. Constructor-inferred fringe ownership
    // must not suppress tunnels or trigger tree routing for this API.
    graph.tree_routing_nodes.clear();
    graph.tree_sentinels.clear();
    graph.preprocessed_tree_children.clear();
    graph.restored_tree_edges.clear();
    graph.published_tree_node_edge_order.clear();
    // translateGraph constructs fresh Go nodes. Standalone RouteEdges does
    // not run labeling initialization or ComputeNodeSpacing: an existing
    // position is not a private fixed-position bit, nor precomputed padding.
    for node in &mut graph.nodes {
        // adapter.go copies the existing Box.TopLeft independently of
        // FixedTopLeft. Routing must not apply authored placement constraints.
        if let Some(position) = input
            .initial_positions
            .get(node.input_id.0 as usize)
            .copied()
            .flatten()
            .or_else(|| {
                input
                    .node(node.input_id)
                    .and_then(|node| node.locked_position)
            })
        {
            node.position = Some(position);
            node.rect.origin = position;
        }
        node.loop_offsets = None;
        node.label_position_fixed = false;
        node.icon_position_fixed = false;
        node.node_padding = Insets::uniform(0.0);
        node.layout_margins = Insets::uniform(0.0);
    }
}

fn route_existing_impl(
    input: &Graph,
    options: &LayoutOptions,
    requested: &[EdgeId],
    cancellation: Option<&CancellationToken>,
) -> Result<LayoutResult, LayoutError> {
    validation::validate_requested_edges(input, requested)?;
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(cancellation
            .as_ref()
            .and_then(|token| token.error())
            .unwrap_or(LayoutError::Cancelled));
    }
    validation::validate_existing(input)?;
    // The source RouteEdges API has no seed plan. This is only the shared
    // report/work-error seed tag; routing does not consume a seed RNG.
    let seed = options.seeds.first().copied().unwrap_or(1);
    let canonical = canonical_tala_input(input);
    let mut pipeline =
        with_cancellation(cancellation, || Pipeline::new(&canonical, seed, true, true));
    initialize_standalone_node_state(&mut pipeline.graph, &canonical);
    // RouteEdges receives the already-built D2 hierarchy. Preserve the
    // translated parent/container links exactly as the upstream adapter does;
    // inferring containment from overlapping boxes can clip a child and change
    // the route endpoint candidates even though its serialized geometry is
    // fixed.
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(cancellation
            .as_ref()
            .and_then(|token| token.error())
            .unwrap_or(LayoutError::Cancelled));
    }
    #[cfg(feature = "diagnostic-traces")]
    if std::env::var_os("D2_TRACE_ROUTER_INPUT").is_some_and(|value| value == "1") {
        eprintln!(
            "ROUTER_INPUT begin nodes={} edges={} selected={}",
            pipeline.graph.nodes.len(),
            pipeline.graph.edge_records.len(),
            requested.len()
        );
        for id in &pipeline.graph.node_order {
            let node = &pipeline.graph.nodes[id.0 as usize];
            let shape = match node.shape {
                ShapeKind::Rectangle | ShapeKind::Square => "Square".to_owned(),
                ShapeKind::SqlTable => "Table".to_owned(),
                shape => format!("{shape:?}"),
            };
            eprintln!(
                "ROUTER_SHAPE node={} shape={} columns={} 3d={} multiple={} icon={} iconfixed={} labelfixed={}",
                node.tala_id,
                shape,
                node.table_column_count.unwrap_or_default(),
                node.is_3d,
                node.is_multiple,
                if node.has_icon {
                    node.icon_position.unwrap_or_default().d2_name()
                } else {
                    "nil"
                },
                node.icon_position_fixed,
                node.label_position_fixed
            );
            let p = node.position.unwrap_or_default();
            let label = node.label_size.unwrap_or_default();
            eprintln!(
                "ROUTER_NODE node={} parent={} rect={:016x},{:016x},{:016x},{:016x} label={:016x},{:016x} position={} font={} invisible={}",
                node.tala_id,
                node.container
                    .map_or(0, |id| pipeline.graph.nodes[id.0 as usize].tala_id),
                p.x.to_bits(),
                p.y.to_bits(),
                node.rect.size.width.to_bits(),
                node.rect.size.height.to_bits(),
                label.width.to_bits(),
                label.height.to_bits(),
                node.label_position.d2_name(),
                node.font_size.unwrap_or_default(),
                node.is_invisible
            );
        }
        for id in &pipeline.graph.edge_order.read() {
            let edge = &pipeline.graph.edge_records[id.0 as usize];
            let label = edge.label.as_ref();
            let size = label.map_or(Size::default(), |label| label.size);
            eprintln!(
                "ROUTER_EDGE edge={} label={:016x},{:016x} position={} percentage={:016x} fixed={} invisible={} curve={} ports={},{} arrows={},{}",
                edge.tala_id,
                size.width.to_bits(),
                size.height.to_bits(),
                label.map_or("", |label| label.position.d2_name()),
                label.map_or(0.0, |label| label.percentage).to_bits(),
                edge.label_position_fixed,
                edge.invisible,
                edge.is_curve,
                edge.source_table_column.map_or(-1, |value| value as i64),
                edge.target_table_column.map_or(-1, |value| value as i64),
                edge.source_arrowhead
                    .as_deref()
                    .unwrap_or(if edge.source_arrow {
                        "triangle"
                    } else {
                        "none"
                    }),
                edge.target_arrowhead
                    .as_deref()
                    .unwrap_or(if edge.target_arrow {
                        "triangle"
                    } else {
                        "none"
                    })
            );

            for (endpoint, label) in [
                edge.source_arrowhead_label.as_ref(),
                edge.target_arrowhead_label.as_ref(),
            ]
            .into_iter()
            .enumerate()
            {
                let size = label.map_or(Size::default(), |label| label.size);
                eprintln!(
                    "ROUTER_END_LABEL edge={} endpoint={} present={} size={:016x},{:016x}",
                    edge.tala_id,
                    endpoint,
                    label.is_some(),
                    size.width.to_bits(),
                    size.height.to_bits()
                );
            }

            eprintln!(
                "ROUTER_INPUT edge={} from={} to={} selected={} min={},{} points={}",
                edge.tala_id,
                pipeline.graph.nodes[edge.from.0 as usize].tala_id,
                pipeline.graph.nodes[edge.to.0 as usize].tala_id,
                u8::from(requested.contains(id)),
                edge.min_width as i64,
                edge.min_height as i64,
                edge.route
                    .read()
                    .iter()
                    .map(|p| format!("{:016x},{:016x}", p.x.to_bits(), p.y.to_bits()))
                    .collect::<Vec<_>>()
                    .join(";")
            );
        }
        eprintln!("ROUTER_INPUT end");
    }
    let requested_set = requested
        .iter()
        .map(|edge| edge.0 as usize)
        .collect::<BTreeSet<_>>();
    routing::route_existing_owner(&mut pipeline.graph, requested, seed, cancellation)?;
    let boxes = pipeline
        .graph
        .nodes
        .iter()
        .filter(|node| {
            pipeline.graph.topology_arena_id().is_none()
                || pipeline
                    .graph
                    .topology_source_input_node(node.input_id)
                    .is_some()
        })
        .filter_map(|node| {
            Some((
                node.input_id,
                Rect {
                    origin: node.position?,
                    size: node.rect.size,
                },
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let mut routes = pipeline
        .graph
        .member_edges()
        .map(|edge| (edge.input_id, edge.route.read().clone()))
        .collect::<BTreeMap<_, _>>();
    let mut edge_labels: BTreeMap<_, _> = pipeline
        .graph
        .member_edges()
        .filter_map(|edge| edge.label.clone().map(|label| (edge.input_id, label)))
        .collect();
    restore_declared_edge_orientation(input, &mut routes, &mut edge_labels);
    // buildRoutePatch/commitRoutePatch publish only selected edges. Internal
    // simplification of locked routes must not change caller-visible geometry.
    for (id, _) in input.edges() {
        if !requested_set.contains(&(id.0 as usize)) {
            routes.insert(id, input.edge_route(id).to_vec());
            if let Some(label) = input.edge_label(id) {
                edge_labels.insert(id, label.clone());
            } else {
                edge_labels.remove(&id);
            }
        }
    }
    let edge_label_percentage_commits = requested
        .iter()
        .copied()
        .filter(|id| edge_labels.contains_key(id))
        .collect();
    #[cfg(feature = "diagnostic-traces")]
    trace_label_commit(input, &edge_labels, &edge_label_percentage_commits, "route");
    let node_labels = pipeline
        .graph
        .nodes
        .iter()
        .map(|node| {
            (
                node.input_id,
                NodeLabelState {
                    font_size_committed: false,
                    size: node.label_size,
                    font_size: node.font_size,
                    position: node.label_position,
                    icon_position: node.icon_position,
                },
            )
        })
        .collect();
    let score = evaluation::evaluate_layout(input, &boxes, &routes);

    Ok(LayoutResult {
        removed_edges: Default::default(),
        source_edge_storage: d2_rs_core::SourceEdgeStorage::Replaced,
        synthetic_root: None,
        route_curves: Default::default(),
        sequence_lifelines: Vec::new(),
        edge_label_percentage_commits,
        boxes,
        routes,
        edge_labels,
        node_labels,
        report: LayoutReport {
            selected_seed: seed,
            selected_pass: 0,
            score: score.clone(),
            improvement_passes: 1,
            candidates: vec![CandidateReport {
                seed,
                pass: 0,
                accepted: score.invalidities == 0,
                score,
                race_score: None,
                race_score_components: None,
                race_seed_edge_order: None,
            }],
            warnings: Vec::new(),
        },
    })
}

pub(super) fn prescale_input(input: &Graph) -> Graph {
    let mut pipeline = Pipeline::new(input, 1, false, false);
    pipeline.run_prescale();
    let mut output = input.clone();
    for arena_node in pipeline.graph.nodes {
        output.nodes[arena_node.input_id.0 as usize].size = arena_node.rect.size;
    }
    output
}

#[cfg(test)]
mod equidistance_prelude_tests {
    use super::*;

    // Pinned original-Go actual-wrapper witness: automatic grouping creates
    // Row220x101, actual OptimizeClusters computes cell220 and flips it into
    // Column100x222. Equidistance must recompute222 before its own work.
    fn after_actual_cluster_flip() -> Pipeline {
        let mut input = Graph::default();
        let first = input.add_node(crate::Node::new(
            "first",
            Size {
                width: 100.0,
                height: 101.0,
            },
        ));
        let second = input.add_node(crate::Node::new(
            "second",
            Size {
                width: 100.0,
                height: 101.0,
            },
        ));
        let peer = input.add_node(crate::Node::new(
            "peer",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        input.tala_id_overrides = vec![Some(1), Some(2), Some(3)];
        for member in [first, second] {
            input.add_edge(crate::Edge {
                source: member,
                target: peer,
            });
        }
        let mut pipeline = Pipeline::new_untraced(&input, 1, true, true);
        for (id, position) in [
            (
                first,
                Point {
                    x: 1000.0,
                    y: 1000.0,
                },
            ),
            (
                second,
                Point {
                    x: 1120.0,
                    y: 1000.0,
                },
            ),
            (
                peer,
                Point {
                    x: 1800.0,
                    y: 1000.0,
                },
            ),
        ] {
            pipeline.graph.set_position(id, position);
        }
        pipeline.graph.assign_clusters(1, false);
        assert_eq!(pipeline.graph.clusters.len(), 1);
        assert_eq!(pipeline.graph.clusters[0].members, [first, second]);
        assert_eq!(
            pipeline.graph.clusters[0].arrangement,
            ClusterArrangement::Row
        );
        assert_eq!(
            pipeline.graph.clusters[0].padding.to_bits(),
            20.0_f64.to_bits()
        );
        assert_eq!(
            pipeline.graph.cluster_vessel_size(0),
            Size {
                width: 220.0,
                height: 101.0
            }
        );
        // The live production OptimizeClusters wrapper owns this recompute;
        // do not force the arrangement, cell or final vessel dimensions.
        pipeline.optimize_clusters_stage().unwrap();
        assert!(pipeline.run_align_axis);
        assert_eq!(
            pipeline.graph.clusters[0].arrangement,
            ClusterArrangement::Column
        );
        assert_eq!(
            pipeline.graph.clusters[0].padding.to_bits(),
            20.0_f64.to_bits()
        );
        assert_eq!(
            pipeline.graph.cluster_vessel_size(0),
            Size {
                width: 100.0,
                height: 222.0
            }
        );
        assert_eq!(pipeline.graph.cell_size.to_bits(), 220.0_f64.to_bits());
        pipeline
    }

    #[test]
    fn equidistance_wrapper_recomputes_after_actual_cluster_flip() {
        let mut pipeline = after_actual_cluster_flip();
        pipeline.equidistance_stage();
        assert_eq!(pipeline.graph.cell_size.to_bits(), 222.0_f64.to_bits());
    }

    #[test]
    fn equidistance_wrapper_keeps_already_current_flipped_cell() {
        let mut pipeline = after_actual_cluster_flip();
        pipeline.graph.compute_cell_size();
        assert_eq!(pipeline.graph.cell_size.to_bits(), 222.0_f64.to_bits());
        pipeline.equidistance_stage();
        assert_eq!(pipeline.graph.cell_size.to_bits(), 222.0_f64.to_bits());
    }

    fn simple_pipeline() -> (Pipeline, NodeId) {
        let mut input = Graph::default();
        let node = input.add_node(crate::Node::new(
            "node",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let mut pipeline = Pipeline::new_untraced(&input, 1, true, true);
        pipeline.graph.set_position(
            node,
            Point {
                x: 1000.0,
                y: 1000.0,
            },
        );
        pipeline.graph.compute_cell_size();
        assert_eq!(pipeline.graph.cell_size.to_bits(), 100.0_f64.to_bits());
        (pipeline, node)
    }

    #[test]
    fn equidistance_wrapper_recomputes_after_dimension_change() {
        let (mut pipeline, node) = simple_pipeline();
        pipeline.graph.nodes[node.0 as usize].rect.size.width = 13.0;
        pipeline.equidistance_stage();
        assert_eq!(pipeline.graph.cell_size.to_bits(), 20.0_f64.to_bits());
    }

    #[test]
    fn equidistance_wrapper_keeps_unchanged_dimensions() {
        let (mut pipeline, _) = simple_pipeline();
        pipeline.equidistance_stage();
        assert_eq!(pipeline.graph.cell_size.to_bits(), 100.0_f64.to_bits());
    }

    #[test]
    fn equidistance_wrapper_recomputes_empty_graph() {
        // A source-equivalent fresh owning Graph has CellSize0; do not use
        // the ready-lattice direct-helper constructor to hide that prelude.
        let mut pipeline = Pipeline::new_untraced(&Graph::default(), 1, false, false);
        assert_eq!(pipeline.graph.cell_size.to_bits(), 0.0_f64.to_bits());
        pipeline.equidistance_stage();
        assert_eq!(pipeline.graph.cell_size.to_bits(), 10.0_f64.to_bits());
    }
}

#[cfg(test)]
mod optimize_clusters_prelude_tests {
    use super::*;

    // Original Go v0.9.0 engine.optimizeClustersStage controls: derive a
    // stale cell from dimensions, mutate geometry, and discover the real
    // cluster before invoking the shared production wrapper. These target
    // the wrapper's cell contract, not full authored NodePlacement or caches.
    fn derived_cluster_prelude_pipeline(
        x: f64,
        initial_blocker_width: f64,
        final_blocker_width: f64,
    ) -> Pipeline {
        let mut input = Graph::default();
        let first = input.add_node(crate::Node::new(
            "first",
            Size {
                width: 100.0,
                height: 101.0,
            },
        ));
        let second = input.add_node(crate::Node::new(
            "second",
            Size {
                width: 100.0,
                height: 101.0,
            },
        ));
        let peer = input.add_node(crate::Node::new(
            "peer",
            Size {
                width: 29_900.0,
                height: 100.0,
            },
        ));
        let blocker = input.add_node(crate::Node::new(
            "blocker",
            Size {
                width: initial_blocker_width,
                height: 101.0,
            },
        ));
        let mut anchor_input = crate::Node::new(
            "anchor",
            Size {
                width: 100.0,
                height: 100.0,
            },
        );
        anchor_input.locked_position = Some(Point { x: 0.0, y: 2000.0 });
        let anchor = input.add_node(anchor_input);
        input.tala_id_overrides = vec![Some(1), Some(2), Some(3), Some(4), Some(5)];
        for member in [first, second] {
            for _ in 0..2 {
                input.add_edge(crate::Edge {
                    source: member,
                    target: peer,
                });
            }
        }
        let mut pipeline = Pipeline::new_untraced(&input, 1, true, true);
        for (id, position) in [
            (first, Point { x, y: 1000.0 }),
            (
                second,
                Point {
                    x: x + 120.0,
                    y: 1000.0,
                },
            ),
            (
                peer,
                Point {
                    x: 10_000.0,
                    y: 0.0,
                },
            ),
            (
                blocker,
                Point {
                    x: 24_840.0,
                    y: 1000.0,
                },
            ),
            (
                anchor,
                Point {
                    x: 10_000.0,
                    y: 2000.0,
                },
            ),
        ] {
            pipeline.graph.set_position(id, position);
        }
        pipeline.graph.compute_cell_size();
        assert_eq!(
            pipeline.graph.cell_size.to_bits(),
            (if initial_blocker_width == 13.0 {
                20.0_f64
            } else {
                10.0_f64
            })
            .to_bits(),
            "prior cell must come from actual dimensions, not a forced assignment",
        );
        pipeline.graph.nodes[blocker.0 as usize].rect.size.width = final_blocker_width;
        pipeline.graph.assign_clusters(1, false);
        assert_eq!(pipeline.graph.clusters.len(), 1);
        assert_eq!(pipeline.graph.clusters[0].members, [first, second]);
        assert_eq!(
            pipeline.graph.clusters[0].arrangement,
            crate::internal::ClusterArrangement::Row,
        );
        assert_eq!(
            pipeline.graph.cluster_vessel_size(0),
            Size {
                width: 220.0,
                height: 101.0
            },
        );
        pipeline
    }

    fn cluster_prelude_cell_contract(x: f64, initial_width: f64, final_width: f64) {
        let mut pipeline = derived_cluster_prelude_pipeline(x, initial_width, final_width);
        // Same method invoked by the production snapshot/full-layout path.
        // BEFORE extraction changes no semantics and must fail the stale-cell
        // controls. Do not replace this call with a literal direct helper.
        pipeline.optimize_clusters_stage().unwrap();
        let expected = if final_width == 13.0 {
            20.0_f64
        } else {
            10.0_f64
        };
        assert_eq!(
            pipeline.graph.cell_size.to_bits(),
            expected.to_bits(),
            "original Go engine wrapper recomputes before OptimizeClusters work",
        );
    }

    #[test]
    fn optimize_clusters_wrapper_recomputes_derived_cell_below_size_limit() {
        cluster_prelude_cell_contract(24_939.0, 13.0, 1.0);
    }

    #[test]
    fn optimize_clusters_wrapper_recomputes_derived_cell_equal_size_limit() {
        cluster_prelude_cell_contract(24_940.0, 13.0, 1.0);
    }

    #[test]
    fn optimize_clusters_wrapper_recomputes_derived_cell_above_size_limit() {
        cluster_prelude_cell_contract(24_941.0, 13.0, 1.0);
    }

    #[test]
    fn optimize_clusters_wrapper_keeps_unchanged_derived_cell() {
        cluster_prelude_cell_contract(24_940.0, 13.0, 13.0);
    }

    #[test]
    fn optimize_clusters_wrapper_keeps_already_current_cell() {
        cluster_prelude_cell_contract(24_940.0, 1.0, 1.0);
    }

    #[test]
    fn optimize_clusters_wrapper_recomputes_even_without_clusters() {
        let mut input = Graph::default();
        let node = input.add_node(crate::Node::new(
            "node",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let mut pipeline = Pipeline::new_untraced(&input, 1, true, true);
        pipeline.graph.set_position(
            node,
            Point {
                x: 1000.0,
                y: 1000.0,
            },
        );
        pipeline.graph.compute_cell_size();
        assert_eq!(pipeline.graph.cell_size.to_bits(), 100.0_f64.to_bits());
        pipeline.graph.nodes[node.0 as usize].rect.size.width = 13.0;
        assert!(pipeline.graph.clusters.is_empty());
        pipeline.optimize_clusters_stage().unwrap();
        assert_eq!(pipeline.graph.cell_size.to_bits(), 20.0_f64.to_bits());
        assert!(!pipeline.run_align_axis);
        assert_eq!(
            pipeline.graph.position(node),
            Some(Point {
                x: 1000.0,
                y: 1000.0
            })
        );
    }
}
#[cfg(test)]
mod race_seed_tests {
    use super::*;
    // Pinned Go v0.9.0 prepared SwapStuff contracts. These start from valid
    // positioned graphs; authored NodePlacement/CLI reachability is separate.
    fn prepared_swap_max_size_fixture(x: f64) -> (Pipeline, [NodeId; 3]) {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 29_900.0,
                height: 100.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let c = input.add_node(crate::Node::new(
            "c",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        for _ in 0..2 {
            input.add_edge(crate::Edge {
                source: a,
                target: c,
            });
        }
        let mut pipeline = Pipeline::new_untraced(&input, 1, true, true);
        for (id, position) in [
            (a, Point { x: 0.0, y: 0.0 }),
            (b, Point { x, y: 300.0 }),
            (
                c,
                Point {
                    x: x + 14_900.0,
                    y: 600.0,
                },
            ),
        ] {
            pipeline.graph.set_position(id, position);
        }
        pipeline.graph.compute_cell_size();
        assert_eq!(pipeline.graph.cell_size, 150.0);
        assert!(pipeline.graph.tree_nodes_in_extraction_order().is_empty());
        for id in [a, b, c] {
            assert!(pipeline.graph.nodes[id.0 as usize].fixed_top_left.is_none());
            assert!(pipeline.graph.nodes[id.0 as usize].hierarchy.is_none());
        }
        (pipeline, [a, b, c])
    }

    fn prepared_swap_max_size_boundary_contract(x: f64, smart_global_bits: u64) {
        let (mut pipeline, ids @ [a, b, c]) = prepared_swap_max_size_fixture(x);
        let geometry_bits = |graph: &ArenaGraph| {
            ids.map(|id| {
                let position = graph.active_node_position(id).unwrap();
                let size = graph.active_node_size(id);
                [
                    position.x.to_bits(),
                    position.y.to_bits(),
                    size.width.to_bits(),
                    size.height.to_bits(),
                ]
            })
        };
        let before = geometry_bits(&pipeline.graph);
        let (tl, br) = pipeline.stage_graph_bounds().unwrap();
        assert_eq!((br.x - tl.x, br.y - tl.y), (29_900.0, 700.0));
        assert!(
            pipeline
                .check_stage_dimensions("PreparedSwapBaseline")
                .is_ok()
        );
        let overlaps = pipeline.graph.existing_overlap_pairs();
        let exact_overlaps = pipeline.graph.exact_overlap_pairs();
        assert!(overlaps.is_empty());
        assert!(exact_overlaps.is_empty());

        // Original Go source admits both real AffectContainers trial variants,
        // including an ordinary trial of width 30_001, then rolls them back.
        let regular = pipeline
            .graph
            .evaluate_swap_trial(a, b, false, &overlaps, &exact_overlaps)
            .expect("source admits ordinary trial before the completed-stage size invariant");
        assert!(regular.0.is_finite() && regular.1.is_finite());
        assert_eq!(regular.1.to_bits(), 800.0_f64.to_bits());
        assert_eq!(regular.2, 0);
        assert_eq!(
            geometry_bits(&pipeline.graph),
            before,
            "ordinary rollback retains exact bits"
        );
        let smart = pipeline
            .graph
            .evaluate_swap_trial(a, b, true, &overlaps, &exact_overlaps)
            .expect("paired smart trial remains admitted");
        assert!(smart.0.is_finite() && smart.1.is_finite());
        assert_eq!(smart.1.to_bits(), smart_global_bits);
        assert_eq!(smart.2, 0);
        assert_eq!(
            geometry_bits(&pipeline.graph),
            before,
            "smart rollback retains exact bits"
        );

        // Reproduce the complete source Swap stage: up to four optimizer passes,
        // followed by direct(checkEdgeLength=true), then its actual boundary.
        let mut changed = false;
        for _ in 0..4 {
            if !pipeline.graph.swap_optimize() {
                break;
            }
            changed = true;
        }
        assert!(changed);
        pipeline.graph.direct(true);
        assert_eq!(pipeline.graph.position(a), Some(Point { x, y: 300.0 }));
        assert_eq!(pipeline.graph.position(b), Some(Point { x: 0.0, y: 0.0 }));
        assert_eq!(
            pipeline.graph.position(c),
            Some(Point {
                x: x + 14_900.0,
                y: 600.0
            })
        );
        let (tl, br) = pipeline.stage_graph_bounds().unwrap();
        assert_eq!((br.x - tl.x, br.y - tl.y), (29_900.0 + x, 700.0));
        if x <= 100.0 {
            assert!(pipeline.check_stage_dimensions("SwapStuff").is_ok());
        } else {
            let error = pipeline.check_stage_dimensions("SwapStuff").unwrap_err();
            assert!(matches!(
                error,
                LayoutError::StageDimensions {
                    stage: "SwapStuff",
                    width: 30_001.0,
                    height: 700.0
                }
            ));
            assert_eq!(
                error.to_string(),
                "layout invariant violated: Dimensions w:30001, h:700 reached after stage SwapStuff"
            );
        }
    }

    #[test]
    fn swap_trial_below_max_size_matches_source_boundary() {
        prepared_swap_max_size_boundary_contract(99.0, 0x4089_0001_5a07_b353);
    }

    #[test]
    fn swap_trial_equal_max_size_matches_source_boundary() {
        prepared_swap_max_size_boundary_contract(100.0, 0x4089_0001_5d86_7c3f);
    }

    #[test]
    fn swap_trial_above_max_size_matches_source_boundary() {
        prepared_swap_max_size_boundary_contract(101.0, 0x4089_0001_6105_452b);
    }

    // Pinned Go v0.9.0 prepared GapNormalization admission and stage bounds.
    // This models a prepared ordinary source graph with explicit Go identities;
    // it does not assert that parsing/NodePlacement yields these positions.
    fn prepared_gap_max_size_fixture(x: f64) -> (Pipeline, [NodeId; 4]) {
        let mut input = Graph::default();
        let ids = ["a", "b", "c", "d"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 100.0,
                    height: 100.0,
                },
            ))
        });
        input.add_edge(crate::Edge {
            source: ids[0],
            target: ids[1],
        });
        input.add_edge(crate::Edge {
            source: ids[1],
            target: ids[2],
        });
        let mut pipeline = Pipeline::new_untraced(&input, 1, true, true);
        for (index, position) in [
            Point { x: 0.0, y: 0.0 },
            Point { x, y: 0.0 },
            Point { x: 0.0, y: 300.0 },
            Point {
                x: 29_800.0,
                y: 600.0,
            },
        ]
        .into_iter()
        .enumerate()
        {
            let id = ids[index];
            pipeline.graph.nodes[id.0 as usize].tala_id = index as i64 + 1;
            pipeline.graph.set_position(id, position);
            assert!(pipeline.graph.nodes[id.0 as usize].fixed_top_left.is_none());
            assert!(pipeline.graph.nodes[id.0 as usize].hierarchy.is_none());
        }
        assert!(pipeline.graph.clusters.is_empty());
        assert!(pipeline.graph.sequences.is_empty());
        assert!(pipeline.graph.tree_nodes_in_extraction_order().is_empty());
        assert!(pipeline.graph.existing_overlap_pairs().is_empty());
        pipeline.graph.compute_cell_size();
        assert_eq!(pipeline.graph.cell_size, 100.0);
        // The original main graph lazily materializes full routing costs on
        // its first sized EdgeLength score. Prepared native graphs bypass
        // NodePlacement, so initialize that existing main-graph cost surface.
        pipeline.graph.initialize_combined_scoring_costs();
        assert!(
            pipeline
                .check_stage_dimensions("PreparedGapBaseline")
                .is_ok()
        );
        let (tl, br) = pipeline.stage_graph_bounds().unwrap();
        assert_eq!((br.x - tl.x, br.y - tl.y), (29_900.0, 700.0));
        (pipeline, ids)
    }

    fn prepared_gap_max_size_boundary_contract(
        x: f64,
        old_global_bits: u64,
        direct_global_bits: u64,
    ) {
        let (mut direct, ids) = prepared_gap_max_size_fixture(x);
        let geometry_bits = |graph: &ArenaGraph| {
            ids.map(|id| {
                let position = graph.active_node_position(id).unwrap();
                let size = graph.active_node_size(id);
                [
                    position.x.to_bits(),
                    position.y.to_bits(),
                    size.width.to_bits(),
                    size.height.to_bits(),
                ]
            })
        };
        let original = geometry_bits(&direct.graph);
        assert_eq!(
            direct
                .graph
                .global_sized_edge_length_after_tree_restoration(false)
                .to_bits(),
            old_global_bits
        );
        let changed = direct
            .graph
            .reduce_gap_to_neighbors(ids[0], true, true, false);
        if !changed {
            assert_eq!(
                geometry_bits(&direct.graph),
                original,
                "a rejected native trial preserves exact prior geometry"
            );
        }
        assert!(
            changed,
            "actual source direct gap trial is admitted before completed-stage dimension check"
        );
        assert_eq!(
            direct
                .graph
                .global_sized_edge_length_after_tree_restoration(false)
                .to_bits(),
            direct_global_bits
        );
        let expected_direct = [
            Point { x: 0.0, y: 0.0 },
            Point { x: 250.0, y: 0.0 },
            Point {
                x: 250.0 - x,
                y: 300.0,
            },
            Point {
                x: 29_800.0,
                y: 600.0,
            },
        ];
        for (id, expected) in ids.into_iter().zip(expected_direct) {
            let actual = direct.graph.active_node_position(id).unwrap();
            assert_eq!(
                (actual.x.to_bits(), actual.y.to_bits()),
                (expected.x.to_bits(), expected.y.to_bits())
            );
        }

        // Fresh baseline; invoke the actual native Pipeline wrapper's complete
        // four directional passes, matching the full source NormalizeGaps stage.
        let (mut full, ids) = prepared_gap_max_size_fixture(x);
        assert!(full.gap_normalization_stage().unwrap());
        let expected_full = [
            Point {
                x: 250.0 - x,
                y: 0.0,
            },
            Point {
                x: 500.0 - x,
                y: 0.0,
            },
            Point {
                x: 250.0 - x,
                y: 250.0,
            },
            Point {
                x: 29_800.0,
                y: 600.0,
            },
        ];
        for (id, expected) in ids.into_iter().zip(expected_full) {
            let actual = full.graph.active_node_position(id).unwrap();
            assert_eq!(
                (actual.x.to_bits(), actual.y.to_bits()),
                (expected.x.to_bits(), expected.y.to_bits())
            );
        }
        assert_eq!(
            full.graph
                .global_sized_edge_length_after_tree_restoration(false)
                .to_bits(),
            0x4092_d843_9a01_d1a1
        );
        let (tl, br) = full.stage_graph_bounds().unwrap();
        assert_eq!((br.x - tl.x, br.y - tl.y), (29_900.0 + x - 250.0, 700.0));
        if x <= 350.0 {
            assert!(full.check_stage_dimensions("GapNormalization").is_ok());
        } else {
            let error = full.check_stage_dimensions("GapNormalization").unwrap_err();
            assert!(matches!(
                error,
                LayoutError::StageDimensions {
                    stage: "GapNormalization",
                    width: 30_001.0,
                    height: 700.0
                }
            ));
            assert_eq!(
                error.to_string(),
                "layout invariant violated: Dimensions w:30001, h:700 reached after stage GapNormalization"
            );
        }
    }

    #[test]
    fn gap_trial_below_max_size_matches_source_boundary() {
        prepared_gap_max_size_boundary_contract(
            349.0,
            0x409b_6ec0_7d0b_4638,
            0x4098_56c0_7d0b_4639,
        );
    }
    #[test]
    fn gap_trial_equal_max_size_matches_source_boundary() {
        prepared_gap_max_size_boundary_contract(
            350.0,
            0x409b_804f_fb00_9fe8,
            0x4098_604f_fb00_9fe7,
        );
    }
    #[test]
    fn gap_trial_above_max_size_matches_source_boundary() {
        prepared_gap_max_size_boundary_contract(
            351.0,
            0x409b_91e0_18ba_40a2,
            0x4098_69e0_18ba_40a2,
        );
    }

    fn stage_nan_bbox_fixture(width: f64) -> (Pipeline, EdgeId) {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width,
                height: 80.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 80.0,
                height: 80.0,
            },
        ));
        let edge = input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        let mut pipeline = Pipeline::new(&input, 1, true, true);
        pipeline.graph.set_position(a, Point { x: 0.0, y: 0.0 });
        pipeline.graph.set_position(b, Point { x: 100.0, y: 0.0 });
        (pipeline, edge)
    }

    #[test]
    fn stage_dimensions_nan_route_preserves_source_bounds_and_gate() {
        for width in [80.0, 30_001.0] {
            let (pipeline, edge) = stage_nan_bbox_fixture(width);
            pipeline.graph.edge_records[edge.0 as usize]
                .route
                .replace(RoutePoints::literal(&[
                    Point {
                        x: f64::from_bits(0x7ff8_0000_0000_0001),
                        y: 0.0,
                    },
                    Point { x: 100.0, y: 0.0 },
                ]));
            let (tl, br) = pipeline.stage_graph_bounds().expect("placed nodes");
            assert_eq!(
                [
                    tl.x.to_bits(),
                    tl.y.to_bits(),
                    br.x.to_bits(),
                    br.y.to_bits()
                ],
                [
                    0x7ff8_0000_0000_0001,
                    0,
                    0x7ff8_0000_0000_0001,
                    80.0_f64.to_bits()
                ]
            );
            assert!(
                pipeline.check_stage_dimensions("EdgeRouting").is_ok(),
                "source comparisons with NaN do not trigger an overflow error"
            );
        }
        // The source's first-point fold quiets a signaling first point before
        // merging the second point. A quiet second payload then wins; a
        // signaling second point takes precedence and retains its payload.
        for (a, b, expected) in [
            (
                0x7ff8_0000_0000_0000,
                0x7ff8_0000_0000_0001,
                0x7ff8_0000_0000_0001,
            ),
            (
                0x7ff8_0000_0000_0001,
                0x7ff8_0000_0000_0000,
                0x7ff8_0000_0000_0000,
            ),
            (
                0xfff8_0000_0000_0007,
                0x7ff8_0000_0000_0003,
                0x7ff8_0000_0000_0003,
            ),
            (
                0x7ff8_0000_0000_0003,
                0xfff8_0000_0000_0007,
                0xfff8_0000_0000_0007,
            ),
            (
                0x7ff0_0000_0000_0005,
                0x7ff8_0000_0000_0003,
                0x7ff8_0000_0000_0003,
            ),
            (
                0x7ff8_0000_0000_0003,
                0x7ff0_0000_0000_0005,
                0x7ff8_0000_0000_0005,
            ),
        ] {
            let (pipeline, edge) = stage_nan_bbox_fixture(80.0);
            pipeline.graph.edge_records[edge.0 as usize]
                .route
                .replace(RoutePoints::literal(&[
                    Point {
                        x: f64::from_bits(a),
                        y: 0.0,
                    },
                    Point {
                        x: f64::from_bits(b),
                        y: 0.0,
                    },
                ]));
            let (tl, br) = pipeline.stage_graph_bounds().unwrap();
            assert_eq!([tl.x.to_bits(), br.x.to_bits()], [expected, expected]);
        }
        // Direct primitive observations from Go 1.27 darwin/arm64 dim_arm64.s;
        // the public graph merge above has its own point-fold ordering.
        for (a, b, minimum, maximum) in [
            (
                0x7ff8_0000_0000_0000,
                0x7ff8_0000_0000_0001,
                0x7ff8_0000_0000_0001,
                0x7ff8_0000_0000_0001,
            ),
            (
                0x7ff8_0000_0000_0001,
                0x7ff8_0000_0000_0000,
                0x7ff8_0000_0000_0000,
                0x7ff8_0000_0000_0000,
            ),
            (
                0xfff8_0000_0000_0007,
                0x7ff8_0000_0000_0003,
                0x7ff8_0000_0000_0003,
                0x7ff8_0000_0000_0003,
            ),
            (
                0x7ff8_0000_0000_0003,
                0xfff8_0000_0000_0007,
                0xfff8_0000_0000_0007,
                0xfff8_0000_0000_0007,
            ),
            (
                0x7ff0_0000_0000_0005,
                0x7ff8_0000_0000_0003,
                0x7ff8_0000_0000_0005,
                0x7ff8_0000_0000_0005,
            ),
            (
                0x7ff8_0000_0000_0003,
                0x7ff0_0000_0000_0005,
                0x7ff8_0000_0000_0005,
                0x7ff8_0000_0000_0005,
            ),
            (
                0x7ff0_0000_0000_0005,
                0xfff0_0000_0000_0007,
                0xfff8_0000_0000_0007,
                0xfff8_0000_0000_0007,
            ),
            (
                0xfff0_0000_0000_0007,
                0x7ff0_0000_0000_0005,
                0x7ff8_0000_0000_0005,
                0x7ff8_0000_0000_0005,
            ),
            (
                0xfff0_0000_0000_0000,
                0x7ff8_0000_0000_0000,
                0xfff0_0000_0000_0000,
                0x7ff8_0000_0000_0000,
            ),
            (
                0x7ff0_0000_0000_0000,
                0x7ff8_0000_0000_0000,
                0x7ff8_0000_0000_0000,
                0x7ff0_0000_0000_0000,
            ),
            (0x8000_0000_0000_0000, 0, 0x8000_0000_0000_0000, 0),
            (0, 0x8000_0000_0000_0000, 0x8000_0000_0000_0000, 0),
            (
                0x8000_0000_0000_0000,
                0x8000_0000_0000_0000,
                0x8000_0000_0000_0000,
                0x8000_0000_0000_0000,
            ),
        ] {
            let (a, b) = (f64::from_bits(a), f64::from_bits(b));
            assert_eq!(pipeline::stage_minimum(a, b).to_bits(), minimum);
            assert_eq!(pipeline::stage_maximum(a, b).to_bits(), maximum);
        }
    }

    #[test]
    fn stage_dimensions_coincident_arrowhead_label_preserves_source_bounds_and_gate() {
        for width in [80.0, 30_001.0] {
            let (mut pipeline, edge) = stage_nan_bbox_fixture(width);
            pipeline.graph.edge_records[edge.0 as usize]
                .route
                .replace(RoutePoints::literal(&[Point { x: 0.0, y: 0.0 }; 2]));
            pipeline.graph.edge_records[edge.0 as usize].target_arrowhead_label =
                Some(crate::ArrowheadLabel {
                    text: "port".into(),
                    size: Size {
                        width: 20.0,
                        height: 10.0,
                    },
                    color: None,
                });
            let (tl, br) = pipeline.stage_graph_bounds().expect("placed nodes");
            // The default quiet NaN's sign differs between x86-64 and arm64,
            // and constant folding can change it, so only the payload is fixed.
            assert_eq!(
                [tl.x, tl.y, br.x, br.y].map(|value| value.to_bits() & !(1 << 63)),
                [0x7ff8_0000_0000_0000; 4]
            );
            assert!(
                pipeline.check_stage_dimensions("PlaceLabels").is_ok(),
                "source comparisons with NaN do not trigger an overflow error"
            );
        }
    }

    #[test]
    fn stage_dimensions_entrypoint_boundary_matches_source() {
        for width in [29_999.0, 30_000.0, 30_001.0] {
            let mut input = Graph::default();
            let mut node = crate::Node::new(
                "wide",
                Size {
                    width,
                    height: 80.0,
                },
            );
            node.fixed_width = true;
            node.fixed_height = true;
            input.add_node(node);
            let snapshot = layout_snapshot(&input, 1, LayoutStage::Normalize);
            if width <= 30_000.0 {
                assert!(
                    snapshot.layout_error.is_none(),
                    "width {width}: {:?}",
                    snapshot.layout_error
                );
                assert!(
                    layout_result(
                        &input,
                        &LayoutOptions {
                            seeds: vec![1],
                            max_concurrency: 1
                        }
                    )
                    .is_ok()
                );
            } else {
                assert!(matches!(
                    snapshot.layout_error,
                    Some(LayoutError::StageDimensions {
                        stage: "NodePlacement",
                        width: 30_001.0,
                        height: 80.0
                    })
                ));
                let error = layout_result(
                    &input,
                    &LayoutOptions {
                        seeds: vec![1],
                        max_concurrency: 1,
                    },
                )
                .expect_err("oversize seed must fail");
                assert_eq!(
                    error.to_string(),
                    "all TALA seed attempts failed: seed 1: layout invariant violated: Dimensions w:30001, h:80 reached after stage NodePlacement"
                );
            }
        }
    }

    #[test]
    fn stage_dimensions_bbox_rounding_and_unplaced_nodes_match_source() {
        let mut input = Graph::default();
        let node = input.add_node(crate::Node::new(
            "wide",
            Size {
                width: 29_999.5,
                height: 80.0,
            },
        ));
        let mut pipeline = Pipeline::new(&input, 1, true, true);
        pipeline
            .graph
            .set_position(node, Point { x: 0.25, y: -0.25 });
        assert_eq!(
            pipeline.stage_graph_bounds(),
            Some((
                Point { x: 0.0, y: -0.0 },
                Point {
                    x: 30_000.0,
                    y: 80.0
                }
            ))
        );
        assert!(pipeline.check_stage_dimensions("NodePlacement").is_ok());
        pipeline.graph.nodes[node.0 as usize].rect.size.width = 30_000.5;
        assert!(matches!(
            pipeline.check_stage_dimensions("NodePlacement"),
            Err(LayoutError::StageDimensions {
                width: 30_001.0,
                height: 80.0,
                ..
            })
        ));
        let unplaced = input.add_node(crate::Node::new("unplaced", Size::default()));
        let mut pipeline = Pipeline::new(&input, 1, true, true);
        pipeline.graph.set_position(node, Point { x: 0.0, y: 0.0 });
        assert!(pipeline.graph.position(unplaced).is_none());
        assert!(pipeline.stage_graph_bounds().is_none());
        assert!(pipeline.check_stage_dimensions("NodePlacement").is_ok());
    }

    #[test]
    fn stage_dimensions_bbox_uses_shallow_fixed_origin() {
        let mut input = Graph::default();
        let root = input.add_node(crate::Node::new(
            "root",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        let mut child = crate::Node::new(
            "child",
            Size {
                width: 80.0,
                height: 80.0,
            },
        );
        child.parent = Some(root);
        child.locked_position = Some(Point { x: 20.0, y: 20.0 });
        let child = input.add_node(child);
        let mut pipeline = Pipeline::new(&input, 1, true, true);
        pipeline
            .graph
            .set_position(root, Point { x: 100.0, y: 100.0 });
        pipeline
            .graph
            .set_position(child, Point { x: 150.0, y: 150.0 });
        assert_eq!(
            pipeline.stage_graph_bounds(),
            Some((Point { x: 100.0, y: 100.0 }, Point { x: 230.0, y: 230.0 }))
        );
        pipeline.graph.nodes[root.0 as usize].fixed_top_left = Some(Point { x: 90.0, y: 90.0 });
        assert_eq!(
            pipeline.stage_graph_bounds(),
            Some((Point { x: 10.0, y: 10.0 }, Point { x: 230.0, y: 230.0 }))
        );
    }

    #[test]
    fn stage_dimensions_bbox_includes_routes_labels_and_modifiers() {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 80.0,
                height: 80.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 80.0,
                height: 80.0,
            },
        ));
        let edge = input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        let mut pipeline = Pipeline::new(&input, 1, true, true);
        pipeline.graph.set_position(a, Point { x: 0.0, y: 0.0 });
        pipeline.graph.set_position(b, Point { x: 100.0, y: 100.0 });
        pipeline.graph.edge_records[edge.0 as usize]
            .route
            .replace(RoutePoints::literal(&[
                Point { x: 0.0, y: 0.0 },
                Point {
                    x: 30_000.6,
                    y: 0.0,
                },
            ]));
        assert!(matches!(
            pipeline.check_stage_dimensions("EdgeRouting"),
            Err(LayoutError::StageDimensions {
                width: 30_001.0,
                height: 180.0,
                ..
            })
        ));
        pipeline.graph.edge_records[edge.0 as usize]
            .route
            .replace(RoutePoints::nil());
        pipeline.graph.nodes[b.0 as usize].label_position = LabelPosition::OutsideRightMiddle;
        pipeline.graph.nodes[b.0 as usize].label_size = Some(Size {
            width: 30_000.0,
            height: 20.0,
        });
        assert!(matches!(
            pipeline.check_stage_dimensions("PlaceLabels"),
            Err(LayoutError::StageDimensions {
                width: 30_190.0,
                height: 180.0,
                ..
            })
        ));
        pipeline.graph.nodes[b.0 as usize].label_size = None;
        pipeline.graph.nodes[b.0 as usize].is_multiple = true;
        assert_eq!(
            pipeline.stage_graph_bounds(),
            Some((Point { x: 0.0, y: 0.0 }, Point { x: 190.0, y: 180.0 }))
        );
    }

    #[test]
    fn stage_dimensions_bbox_excludes_retained_vessel_carrier_state() {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 80.0,
                height: 60.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 80.0,
                height: 60.0,
            },
        ));
        let peer = input.add_node(crate::Node::new(
            "peer",
            Size {
                width: 100.0,
                height: 100.0,
            },
        ));
        for source in [a, b] {
            for _ in 0..2 {
                input.add_edge(crate::Edge {
                    source,
                    target: peer,
                });
            }
        }
        let mut pipeline = Pipeline::new(&input, 1, true, true);
        pipeline.graph.set_position(a, Point { x: 100.0, y: 100.0 });
        pipeline.graph.set_position(b, Point { x: 100.0, y: 180.0 });
        pipeline
            .graph
            .set_position(peer, Point { x: 300.0, y: 300.0 });
        pipeline.graph.assign_clusters(1, false);
        assert_eq!(pipeline.graph.clusters.len(), 1);
        let carrier = pipeline.graph.clusters[0].members[0];
        let hidden = pipeline.graph.clusters[0].members[1];
        pipeline
            .graph
            .pending_cluster_vessel_positions
            .insert(0, Point { x: 100.0, y: 100.0 });
        pipeline.graph.nodes[carrier.0 as usize].fixed_top_left = Some(Point {
            x: -100_000.0,
            y: -100_000.0,
        });
        pipeline.graph.nodes[carrier.0 as usize].label_position = LabelPosition::OutsideRightMiddle;
        pipeline.graph.nodes[carrier.0 as usize].label_size = Some(Size {
            width: 100_000.0,
            height: 20.0,
        });
        pipeline.graph.set_position(
            hidden,
            Point {
                x: 100_000.0,
                y: 100_000.0,
            },
        );
        assert_eq!(
            pipeline.stage_graph_bounds(),
            Some((Point { x: 100.0, y: 100.0 }, Point { x: 400.0, y: 400.0 }))
        );
        assert!(pipeline.check_stage_dimensions("OptimizeClusters").is_ok());
    }

    #[test]
    fn stage_dimensions_cancellation_precedes_overflow() {
        let mut input = Graph::default();
        let node = input.add_node(crate::Node::new(
            "wide",
            Size {
                width: 30_001.0,
                height: 80.0,
            },
        ));
        let mut pipeline = Pipeline::new(&input, 1, true, true);
        pipeline.graph.set_position(node, Point { x: 0.0, y: 0.0 });
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            with_cancellation(Some(&token), || pipeline
                .check_stage_dimensions("NodePlacement")),
            Err(LayoutError::Cancelled)
        ));
        assert!(matches!(
            layout_result_with_cancellation(
                &input,
                &LayoutOptions {
                    seeds: vec![1],
                    max_concurrency: 1
                },
                Some(&token)
            ),
            Err(LayoutError::Cancelled)
        ));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn stage_dimensions_failed_seed_race_joins_in_request_order() {
        let result = race_seed_candidates::<(), _>(&[17, -17, 0], 3, |index, _seed| {
            thread::sleep(Duration::from_millis((3 - index) as u64 * 5));
            Err(LayoutError::StageDimensions {
                stage: "NodePlacement",
                width: 30_001.0 + index as f64,
                height: 80.0,
            })
        });
        let error = result.err().expect("every attempt failed");
        assert_eq!(
            error.to_string(),
            "all TALA seed attempts failed: seed 17: layout invariant violated: Dimensions w:30001, h:80 reached after stage NodePlacement\nseed -17: layout invariant violated: Dimensions w:30002, h:80 reached after stage NodePlacement\nseed 0: layout invariant violated: Dimensions w:30003, h:80 reached after stage NodePlacement"
        );
        assert!(
            matches!(error, LayoutError::AllSeedAttemptsFailed { attempts } if attempts.iter().map(|(seed, _)| *seed).collect::<Vec<_>>() == vec![17, -17, 0])
        );
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn recovered_seed_panics_are_ordered_failures_and_workers_keep_draining() {
        for concurrency in [1, 2, 3] {
            let visited = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed = visited.clone();
            let result =
                race_seed_candidates::<(), _>(&[1, 2, 3], concurrency, move |index, seed| {
                    observed.lock().unwrap().push((index, seed));
                    panic!("source seed invariant");
                });
            let mut visits = visited.lock().unwrap().clone();
            visits.sort();
            assert_eq!(visits, vec![(0, 1), (1, 2), (2, 3)]);
            assert_eq!(
                result.err().unwrap().to_string(),
                "all TALA seed attempts failed: seed 1: TALA seed layout failed due to an internal invariant\nseed 2: TALA seed layout failed due to an internal invariant\nseed 3: TALA seed layout failed due to an internal invariant"
            );
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn stage_dimensions_failed_race_cancellation_wins() {
        let token = CancellationToken::new();
        let worker_token = token.clone();
        let result = race_seed_candidates_with_cancellation::<(), _>(
            &[1, 17],
            2,
            Some(token),
            move |_, _| {
                worker_token.cancel();
                Err(LayoutError::StageDimensions {
                    stage: "NodePlacement",
                    width: 30_001.0,
                    height: 80.0,
                })
            },
        );
        let error = result
            .err()
            .expect("cancellation wins over failed candidates");
        assert_eq!(
            error.to_string(),
            "tala layout stopped before all seeds completed: context canceled"
        );
        assert!(
            matches!(error, LayoutError::Context { source, .. } if matches!(*source, LayoutError::Cancelled))
        );
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn cancelled_seed_coordinator_drains_all_attempts_and_preserves_cause() {
        for deadline in [false, true] {
            let token = CancellationToken::new();
            if deadline {
                token.expire();
            } else {
                token.cancel();
            }
            let completed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed = completed.clone();
            let result = race_seed_candidates_with_cancellation::<(), _>(
                &[7, -7, 0, 42],
                2,
                Some(token),
                move |index, seed| {
                    observed.lock().unwrap().push((index, seed));
                    Err(LayoutError::TimedOut("test candidate"))
                },
            );
            let mut attempts = completed.lock().unwrap().clone();
            attempts.sort();
            assert_eq!(attempts, vec![(0, 7), (1, -7), (2, 0), (3, 42)]);
            let error = result
                .err()
                .expect("cancelled coordinator rejects completed attempts");
            let cause = if deadline {
                "context deadline exceeded"
            } else {
                "context canceled"
            };
            assert_eq!(
                error.to_string(),
                format!("tala layout stopped before all seeds completed: {cause}")
            );
        }
    }

    #[test]
    fn full_layout_canonicalizes_source_only_edges_and_restores_declared_routes() {
        let mut declared = Graph::default();
        let a = declared.add_node(crate::Node::new(
            "a",
            Size {
                width: 80.0,
                height: 80.0,
            },
        ));
        let b = declared.add_node(crate::Node::new(
            "b",
            Size {
                width: 80.0,
                height: 80.0,
            },
        ));
        for text in ["one", "two", "three"] {
            let id = declared.add_edge(crate::Edge {
                source: a,
                target: b,
            });
            declared.set_edge_arrows(
                id,
                crate::EdgeArrows {
                    source: true,
                    target: false,
                },
            );
            declared.set_edge_label(
                id,
                Some(EdgeLabel {
                    text: text.into(),
                    size: Size {
                        width: 40.0,
                        height: 20.0,
                    },
                    position: LabelPosition::UnlockedTop,
                    percentage: 0.5,
                }),
            );
        }
        // The independently declared b -> a graph has the same physical
        // direction. Its routes/labels must differ only in publication
        // orientation; the borrowed a <- b graph keeps its declaration.
        let mut forward = declared.clone();
        for id in declared.edges().map(|(id, _)| id) {
            forward.edges[id.0 as usize] = crate::Edge {
                source: b,
                target: a,
            };
            forward.set_edge_arrows(
                id,
                crate::EdgeArrows {
                    source: false,
                    target: true,
                },
            );
            let mut label = forward.edge_label(id).unwrap().clone();
            label.position = LabelPosition::UnlockedBottom;
            forward.set_edge_label(id, Some(label));
        }
        let options = LayoutOptions {
            seeds: vec![1],
            max_concurrency: 1,
        };
        let expected = layout_result(&forward, &options).unwrap();
        let actual = layout_result(&declared, &options).unwrap();
        assert_eq!(actual.boxes, expected.boxes);
        assert_eq!(actual.report.selected_seed, expected.report.selected_seed);
        for (id, edge) in declared.edges() {
            assert_eq!(edge.source, a);
            assert_eq!(edge.target, b);
            let mut route = expected.routes[&id].clone();
            route.reverse();
            assert_eq!(actual.routes[&id], route);
            let mut label = expected.edge_labels[&id].clone();
            label.position = label.position.mirrored();
            label.percentage = 1.0 - label.percentage;
            assert_eq!(actual.edge_labels[&id], label);
        }
    }

    #[test]
    fn compound_flow_keeps_the_ordinary_layout_when_rerouting_moves_an_authored_label() {
        // X16 job 687: Check PIN's local direction and a small, explicitly
        // centered start circle admit an outer-flow proposal whose relabeling
        // violates result_validate.go's fixed-label position contract.
        let mut input = Graph::default();
        for (index, (name, width, height, label, label_width, label_height)) in [
            ("Check PIN", 161.0, 81.0, "Check PIN", 116.0, 36.0),
            ("Check PIN.start", 10.0, 10.0, "⬤", 9.0, 21.0),
            ("Check PIN.Enter PIN", 111.0, 66.0, "Enter PIN", 66.0, 21.0),
            ("Check PIN.choice", 20.0, 20.0, "choice", 46.0, 21.0),
            ("Check PIN.end", 10.0, 10.0, "⬤", 9.0, 21.0),
            ("Search Network", 159.0, 66.0, "Search Network", 114.0, 21.0),
            ("Ready", 89.0, 66.0, "Ready", 44.0, 21.0),
            ("Off", 68.0, 66.0, "Off", 23.0, 21.0),
        ]
        .into_iter()
        .enumerate()
        {
            let mut node = crate::Node::new(name, Size { width, height });
            node.label_size = Some(Size {
                width: label_width,
                height: label_height,
            });
            node.font_size = Some(if index == 0 { 28 } else { 16 });
            if (1..=4).contains(&index) {
                node.parent = Some(NodeId(0));
            }
            if [1, 3, 4].contains(&index) {
                node.fixed_width = true;
                node.fixed_height = true;
                node.shape = if index == 3 {
                    crate::ShapeKind::Diamond
                } else {
                    crate::ShapeKind::Circle
                };
            }
            if index == 0 {
                node.direction = Some(crate::Direction::Right);
            }
            if index == 1 {
                node.label_position = LabelPosition::InsideMiddleCenter;
            }
            let id = input.add_node(node);
            input.set_node_label(id, label);
            input.tala_id_overrides[index] = Some(i64::from(
                name.bytes().fold(2_166_136_261_u32, |hash, byte| {
                    (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
                }),
            ));
        }
        input.container_flags[0] = true;
        for (from, to, tala_id, text, width) in [
            (1, 2, 676415191, "", 0.0),
            (2, 3, 1833161418, "/check PIN", 72.0),
            (3, 2, 1172586854, "[pin invalid]", 78.0),
            (3, 4, 359648842, "[pin OK]", 53.0),
            (0, 5, 2848456926, "[pin OK]", 53.0),
            (5, 6, 947159636, "network found", 96.0),
            (0, 7, 886741779, "power off", 66.0),
            (5, 7, 605853718, "power off", 66.0),
        ] {
            let id = input.add_edge(crate::Edge {
                source: NodeId(from),
                target: NodeId(to),
            });
            input.tala_edge_ids[id.0 as usize] = Some(tala_id);
            input.set_edge_arrows(
                id,
                crate::EdgeArrows {
                    source: false,
                    target: true,
                },
            );
            if !text.is_empty() {
                input.set_edge_label(
                    id,
                    Some(EdgeLabel {
                        text: text.into(),
                        size: Size {
                            width,
                            height: 21.0,
                        },
                        position: LabelPosition::Unset,
                        percentage: 0.0,
                    }),
                );
            }
        }
        let result = layout_result(
            &input,
            &LayoutOptions {
                seeds: vec![1],
                max_concurrency: 1,
            },
        )
        .unwrap();
        assert_eq!(
            result.node_labels[&NodeId(1)].position,
            LabelPosition::InsideMiddleCenter
        );
        // Pinned Go keeps the ordinary seed: its outer Ready block is to the
        // right of Search Network, rather than below it in the rejected plan.
        assert_eq!(
            result.boxes[&NodeId(6)].origin,
            Point { x: 654.0, y: 396.0 }
        );
    }

    #[test]
    fn refinement_checks_authored_label_and_icon_positions_after_cloning() {
        let mut input = Graph::default();
        let mut node = crate::Node::new(
            "fixed",
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        node.label_size = Some(Size {
            width: 9.0,
            height: 21.0,
        });
        node.label_position = LabelPosition::InsideMiddleCenter;
        node.has_icon = true;
        node.icon_position = Some(LabelPosition::InsideTopLeft);
        input.add_node(node);
        let mut candidate = ArenaGraph::from_input(&input);
        prepare_compound_candidate_state(&mut candidate);
        assert!(refinement_preserves_authored_positions(&input, &candidate));
        candidate.nodes[0].label_position = LabelPosition::OutsideTopCenter;
        assert!(!refinement_preserves_authored_positions(&input, &candidate));
        candidate.nodes[0].label_position = LabelPosition::InsideMiddleCenter;
        candidate.nodes[0].icon_position = Some(LabelPosition::OutsideLeftTop);
        assert!(!refinement_preserves_authored_positions(&input, &candidate));
        input.nodes[0].label_position = LabelPosition::Unset;
        input.nodes[0].icon_position = Some(LabelPosition::Unset);
        assert!(refinement_preserves_authored_positions(&input, &candidate));
    }

    #[test]
    fn compound_label_stage_installs_caller_context_and_preserves_its_cause() {
        let mut input = Graph::default();
        let mut value = crate::Node::new(
            "node",
            Size {
                width: 40.0,
                height: 30.0,
            },
        );
        value.label_size = Some(Size {
            width: 14.0,
            height: 8.0,
        });
        let node = input.add_node(value);
        let mut pipeline = Pipeline::new(&input, 1, true, true);
        pipeline.graph.set_position(node, Point { x: 0.0, y: 0.0 });
        let before = pipeline.graph.nodes[node.0 as usize].label_position;
        for deadline in [false, true] {
            let token = CancellationToken::new();
            if deadline {
                token.expire();
            } else {
                token.cancel();
            }
            let result = run_compound_label_stage(&mut pipeline, Some(&token));
            if deadline {
                assert!(matches!(result, Err(LayoutError::DeadlineExceeded)));
            } else {
                assert!(matches!(result, Err(LayoutError::Cancelled)));
            }
            assert_eq!(pipeline.graph.nodes[node.0 as usize].label_position, before);
        }
        let unrelated = CancellationToken::new();
        unrelated.cancel();
        let caller = CancellationToken::new();
        with_cancellation(Some(&unrelated), || {
            assert!(matches!(
                pipeline.run_place_labels(),
                Err(LayoutError::Cancelled)
            ));
            run_compound_label_stage(&mut pipeline, Some(&caller)).unwrap();
            assert!(matches!(
                crate::internal::cancellation_error(),
                Some(crate::internal::limits::ContextError::Cancelled)
            ));
        });
    }

    #[test]
    fn checked_route_boundary_observes_active_cancellation() {
        let graph = Graph::default();
        let token = CancellationToken::new();
        token.cancel();
        let result = with_cancellation(Some(&token), || {
            routing::route_edges_with_cancellation(&ArenaGraph::from_input(&graph), 0)
        });
        assert!(matches!(result, Err(LayoutError::Cancelled)));
    }

    #[test]
    fn compound_candidate_cache_does_not_mutate_incumbent() {
        let mut input = Graph::default();
        let node = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        input.add_edge(crate::Edge {
            source: node,
            target: node,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[0].label_position_fixed = true;
        graph.nodes[0].icon_position_fixed = true;
        graph.edge_records[0].label_position_fixed = true;
        graph.cost_cache.store_edge_length(7, 11.0);
        graph.nodes[0].loop_offsets = Some(LoopOffsetsMap::from([(0, 31.75), (8, -0.0)]));
        graph
            .transaction_external_containers
            .push(graph.nodes[0].clone());
        graph
            .projected_transaction_nodes
            .push(graph.projected_transaction_node_from_current(node));
        let incumbent = graph.clone();
        prepare_compound_candidate_state(&mut graph);
        assert!(graph.nodes[0].loop_offsets.is_none());
        assert!(
            graph.transaction_external_containers[0]
                .loop_offsets
                .is_none()
        );
        assert!(
            graph.projected_transaction_nodes[0]
                .node
                .loop_offsets
                .is_none()
        );
        assert_eq!(incumbent.nodes[0].loop_offsets.as_ref().unwrap()[&0], 31.75);
        assert_eq!(
            incumbent.nodes[0].loop_offsets.as_ref().unwrap()[&8].to_bits(),
            (-0.0_f64).to_bits()
        );
        assert!(
            incumbent.transaction_external_containers[0]
                .loop_offsets
                .is_some()
        );
        assert!(
            incumbent.projected_transaction_nodes[0]
                .node
                .loop_offsets
                .is_some()
        );
        assert!(!graph.nodes[0].label_position_fixed);
        assert!(!graph.nodes[0].icon_position_fixed);
        assert!(!graph.edge_records[0].label_position_fixed);
        assert!(incumbent.nodes[0].label_position_fixed);
        assert!(incumbent.nodes[0].icon_position_fixed);
        assert!(incumbent.edge_records[0].label_position_fixed);
        assert!(!Arc::ptr_eq(&graph.cost_cache, &incumbent.cost_cache));
        assert!((graph.cost_cache.edge_length_entries() == 0));
        graph.cost_cache.store_edge_length(13, 17.0);
        reset_compound_route_state(&mut graph);
        assert_eq!(incumbent.cost_cache.edge_length_entries(), 1);
        assert_eq!(incumbent.cost_cache.lookup_edge_length(7), Some(11.0));
    }

    #[test]
    fn route_commit_tracks_selected_labels_and_survives_repeated_reversal() {
        let mut input = Graph::default();
        let mut a = crate::Node::new(
            "a",
            Size {
                width: 80.0,
                height: 60.0,
            },
        );
        a.locked_position = Some(Point { x: 0.0, y: 0.0 });
        let a = input.add_node(a);
        let mut b = crate::Node::new(
            "b",
            Size {
                width: 80.0,
                height: 60.0,
            },
        );
        b.locked_position = Some(Point { x: 300.0, y: 0.0 });
        let b = input.add_node(b);
        let selected = input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        let untouched = input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        input.set_edge_arrows(
            selected,
            crate::EdgeArrows {
                source: true,
                target: false,
            },
        );
        for id in [selected, untouched] {
            input.set_edge_label(
                id,
                Some(EdgeLabel {
                    text: "fixed".into(),
                    size: Size {
                        width: 40.0,
                        height: 20.0,
                    },
                    position: LabelPosition::OutsideTopCenter,
                    percentage: 0.0,
                }),
            );
            input.set_edge_label_percentage_present(id, false);
        }
        input.set_edge_route(
            untouched,
            vec![Point { x: 80.0, y: 30.0 }, Point { x: 300.0, y: 30.0 }],
        );
        let options = LayoutOptions {
            seeds: vec![1],
            max_concurrency: 1,
        };
        let first = route_existing_impl(&input, &options, &[selected], None).unwrap();
        assert_eq!(
            first
                .edge_label_percentage_commits
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![selected]
        );
        assert_eq!(first.edge_labels[&selected].percentage, 1.0);
        // The adapter commits only selected edges, even though results also
        // include unchanged labels belonging to fixed obstacle edges.
        input.set_edge_label(selected, Some(first.edge_labels[&selected].clone()));
        input.set_edge_label_percentage_present(
            selected,
            first.edge_label_percentage_commits.contains(&selected),
        );
        input.set_edge_route(selected, first.routes[&selected].clone());
        let second = route_existing_impl(&input, &options, &[selected], None).unwrap();
        assert_eq!(second.edge_labels[&selected].percentage, 1.0);
        assert_eq!(
            second.edge_labels[&selected].position,
            first.edge_labels[&selected].position
        );
        assert!(!input.edge_label_percentage_present(untouched));
        assert!(!second.edge_label_percentage_commits.contains(&untouched));
    }

    #[test]
    fn reversed_fixed_label_distinguishes_absent_and_explicit_zero_percentage() {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new("a", Size::default()));
        let b = input.add_node(crate::Node::new("b", Size::default()));
        let edge = input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        input.set_edge_arrows(
            edge,
            crate::EdgeArrows {
                source: true,
                target: false,
            },
        );
        for (position, present, expected) in [
            (LabelPosition::InsideMiddleCenter, false, 0.0),
            (LabelPosition::InsideMiddleCenter, true, 1.0),
            (LabelPosition::UnlockedTop, false, 1.0),
            (LabelPosition::Unset, false, 0.0),
        ] {
            input.set_edge_label(
                edge,
                Some(EdgeLabel {
                    text: "label".into(),
                    size: Size::default(),
                    position,
                    percentage: 0.0,
                }),
            );
            input.set_edge_label_percentage_present(edge, present);
            assert_eq!(
                canonical_tala_input(&input)
                    .edge_label(edge)
                    .unwrap()
                    .percentage,
                expected
            );
            assert_eq!(input.edge_label(edge).unwrap().percentage, 0.0);
        }
    }

    #[test]
    fn route_adapter_reverses_source_only_arrow_metadata_without_changing_ids() {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new("a", Size::default()));
        let b = input.add_node(crate::Node::new("b", Size::default()));
        let edge = input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        input.tala_edge_ids[0] = Some(123);
        input.set_edge_arrows(
            edge,
            crate::EdgeArrows {
                source: true,
                target: false,
            },
        );
        input.set_edge_table_columns(
            edge,
            crate::EdgeTableColumns {
                source: Some(1),
                target: Some(2),
            },
        );
        input.set_edge_route(
            edge,
            vec![Point { x: 1.0, y: 2.0 }, Point { x: 3.0, y: 4.0 }],
        );
        input.set_edge_label(
            edge,
            Some(EdgeLabel {
                text: "label".into(),
                size: Size::default(),
                position: LabelPosition::OutsideTopLeft,
                percentage: 0.25,
            }),
        );
        let canonical = canonical_tala_input(&input);
        assert_eq!(canonical.edges[0].source, b);
        assert_eq!(canonical.edges[0].target, a);
        assert_eq!(canonical.tala_edge_ids[0], Some(123));
        assert_eq!(canonical.edge_route(edge)[0], Point { x: 3.0, y: 4.0 });
        assert_eq!(canonical.edge_table_columns(edge).source, Some(2));
        assert_eq!(
            canonical.edge_arrowheads(edge).source.as_deref(),
            Some("none")
        );
        assert_eq!(
            canonical.edge_arrowheads(edge).target.as_deref(),
            Some("triangle")
        );
        assert_eq!(
            canonical.edge_label(edge).unwrap().position,
            LabelPosition::OutsideBottomRight
        );
        assert_eq!(canonical.edge_label(edge).unwrap().percentage, 0.75);
        assert_eq!(input.edges[0].source, a);
        let second = canonical_tala_input(&canonical);
        assert_eq!(second.edge_route(edge), canonical.edge_route(edge));
    }

    #[test]
    fn route_reset_preserves_fixed_label_even_with_unlocked_position() {
        let mut input = Graph::default();
        let node = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let edge = input.add_edge(crate::Edge {
            source: node,
            target: node,
        });
        let label = EdgeLabel {
            text: "fixed".into(),
            size: Size {
                width: 30.0,
                height: 12.0,
            },
            position: LabelPosition::UnlockedTop,
            percentage: 0.9,
        };
        input.set_edge_label(edge, Some(label.clone()));
        input.tala_edge_label_fixed[0] = true;
        let mut graph = ArenaGraph::from_input(&input);
        graph.edge_records[0]
            .route
            .insert_fresh(0, Point { x: 1.0, y: 2.0 });
        reset_compound_route_state(&mut graph);
        assert!(graph.edge_records[0].route.is_empty());
        assert_eq!(graph.edge_records[0].label, Some(label));
        graph.edge_records[0].label_position_fixed = false;
        reset_compound_route_state(&mut graph);
        let label = graph.edge_records[0].label.as_ref().unwrap();
        assert_eq!(label.position, LabelPosition::Unset);
        assert_eq!(label.percentage, 0.0);
    }

    #[test]
    fn route_reset_clears_placement_costs_without_replacing_cache() {
        let mut graph = ArenaGraph::from_input(&Graph::default());
        let cache = graph.cost_cache.clone();
        cache.store_edge_length(109, 113.0);
        graph.set_crossing_cost(101.0);
        graph.set_turn_cost(103.0);
        graph.set_non_center_port_cost(107.0);

        reset_compound_route_state(&mut graph);

        assert_eq!(graph.raw_crossing_cost(), 0.0);
        assert_eq!(graph.raw_turn_cost(), 0.0);
        assert_eq!(graph.raw_non_center_port_cost(), 0.0);
        assert!(Arc::ptr_eq(&cache, &graph.cost_cache));
        assert_eq!(cache.edge_length_entries(), 0);
        graph.cost_cache.store_edge_length(127, 131.0);
        assert_eq!(cache.lookup_edge_length(127), Some(131.0));
    }

    #[test]
    fn standalone_routing_does_not_inherit_inferred_placement_trees() {
        let mut input = Graph::default();
        let root = input.add_node(crate::Node::new(
            "root",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let c = input.add_node(crate::Node::new(
            "c",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        input.container_flags[root.0 as usize] = true;
        input.add_edge(crate::Edge {
            source: root,
            target: a,
        });
        input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        input.add_edge(crate::Edge {
            source: a,
            target: c,
        });
        for (id, x) in [(root, 0.0), (a, 100.0), (b, 200.0), (c, 300.0)] {
            input.set_initial_position(id, Some(Point { x, y: 0.0 }));
        }
        let mut graph = ArenaGraph::from_input(&input);
        assert!(!graph.tree_routing_nodes.is_empty());
        let positions = graph
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>();
        let order = graph.node_order.clone();
        initialize_standalone_node_state(&mut graph, &input);
        assert!(graph.tree_routing_nodes.is_empty() && graph.tree_sentinels.is_empty());
        assert!(
            graph.preprocessed_tree_children.is_empty() && graph.restored_tree_edges.is_empty()
        );
        assert!(graph.published_tree_node_edge_order.is_empty());
        assert_eq!(graph.node_order, order);
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| node.position)
                .collect::<Vec<_>>(),
            positions
        );
    }

    #[test]
    fn standalone_nodes_retain_positions_without_layout_label_initialization() {
        let mut input = Graph::default();
        let mut node = crate::Node::new(
            "labelled",
            Size {
                width: 80.0,
                height: 60.0,
            },
        );
        node.locked_position = Some(Point { x: 10.0, y: 20.0 });
        node.label_size = Some(Size {
            width: 30.0,
            height: 12.0,
        });
        node.label_position = LabelPosition::InsideTopCenter;
        node.has_icon = true;
        node.icon_position = Some(LabelPosition::OutsideRightMiddle);
        let id = input.add_node(node);
        let mut graph = ArenaGraph::from_input(&input);
        assert!(graph.nodes[id.0 as usize].label_position_fixed);
        assert!(graph.nodes[id.0 as usize].icon_position_fixed);
        initialize_standalone_node_state(&mut graph, &input);
        let node = &graph.nodes[id.0 as usize];
        assert!(!node.label_position_fixed);
        assert!(!node.icon_position_fixed);
        assert_eq!(node.node_padding, Insets::uniform(0.0));
        assert_eq!(node.layout_margins, Insets::uniform(0.0));
        assert_eq!(node.label_position, LabelPosition::InsideTopCenter);
        assert_eq!(node.icon_position, Some(LabelPosition::OutsideRightMiddle));
        assert_eq!(node.position, Some(Point { x: 10.0, y: 20.0 }));
    }

    #[test]
    fn standalone_owner_preserves_unselected_header_and_point_allocations() {
        let mut input = Graph::default();
        let a = input.add_node(crate::Node::new(
            "a",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let b = input.add_node(crate::Node::new(
            "b",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let c = input.add_node(crate::Node::new(
            "c",
            Size {
                width: 40.0,
                height: 40.0,
            },
        ));
        let selected = input.add_edge(crate::Edge {
            source: a,
            target: b,
        });
        let fixed = input.add_edge(crate::Edge {
            source: a,
            target: c,
        });
        input.set_edge_route(
            fixed,
            vec![
                Point { x: 40.0, y: 20.0 },
                Point { x: 70.0, y: 20.0 },
                Point { x: 70.0, y: 140.0 },
                Point { x: 100.0, y: 140.0 },
            ],
        );
        let mut graph = ArenaGraph::from_input(&input);
        for (id, point) in [
            (a, Point { x: 0.0, y: 0.0 }),
            (b, Point { x: 200.0, y: 0.0 }),
            (c, Point { x: 100.0, y: 120.0 }),
        ] {
            graph.set_position(id, point);
        }
        let old = graph.edge_records[fixed.0 as usize].route.clone();
        let values = old.read();
        let slots = old.header().iter_slots().collect::<Vec<_>>();
        routing::route_existing_owner(&mut graph, &[selected], 1, None).unwrap();
        let route = &graph.edge_records[fixed.0 as usize].route;
        assert!(route.header().same_backing_allocation(&old.header()));
        assert_eq!(route.len(), old.len());
        assert_eq!(route.capacity(), old.capacity());
        assert_eq!(route.read(), values.clone());
        for (i, slot) in slots.into_iter().enumerate() {
            assert!(route.slot(i).unwrap().same_allocation(&slot.unwrap()));
        }
        assert!(graph.edge_records[selected.0 as usize].route.len() >= 2);
    }

    #[test]
    fn route_existing_preserves_declared_nested_geometry() {
        let mut input = Graph::default();
        let frame = input.add_node({
            let mut node = crate::Node::new(
                "frame",
                Size {
                    width: 12.0,
                    height: 120.0,
                },
            );
            node.locked_position = Some(Point { x: 0.0, y: 0.0 });
            node
        });
        let target = input.add_node({
            let mut node = crate::Node::new(
                "frame.target",
                Size {
                    width: 20.0,
                    height: 20.0,
                },
            );
            node.parent = Some(frame);
            node.locked_position = Some(Point { x: -4.0, y: 40.0 });
            node
        });
        let source = input.add_node({
            let mut node = crate::Node::new(
                "source",
                Size {
                    width: 50.0,
                    height: 50.0,
                },
            );
            node.locked_position = Some(Point { x: 200.0, y: 40.0 });
            node
        });
        let edge = input.add_edge(crate::Edge { source, target });
        input.set_edge_route(
            edge,
            vec![Point { x: 250.0, y: 65.0 }, Point { x: -4.0, y: 50.0 }],
        );

        let result = route_existing(
            &input,
            &LayoutOptions {
                seeds: vec![1],
                max_concurrency: 0,
            },
            &[edge],
        )
        .expect("nested route should complete");

        assert_eq!(result.boxes[&target].size.width, 20.0);
        assert_eq!(result.boxes[&target].size.height, 20.0);
        assert_eq!(result.boxes[&target].origin, Point { x: -4.0, y: 40.0 });
    }

    fn candidate(index: usize, seed: i64, penalty: f64) -> SeedCandidate<usize> {
        candidate_with_area(index, seed, penalty, 1.0)
    }

    fn candidate_with_area(
        index: usize,
        seed: i64,
        penalty: f64,
        area: f64,
    ) -> SeedCandidate<usize> {
        SeedCandidate {
            index,
            seed,
            score: evaluation::RaceSelectionScore { penalty, area },
            value: seed as usize,
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn race_seed_ties_choose_the_highest_requested_index() {
        let race = race_seed_candidates(&[11, 22, 33], 3, |index, seed| {
            // Deliberately deliver the candidates out of order. The
            // published choice must remain independent of completion
            // order when TALA treats the scores as equal.
            thread::sleep(Duration::from_millis((3 - index) as u64 * 10));
            Ok(candidate(index, seed, 10.0))
        })
        .expect("all test candidates should finish");

        assert_eq!(race.winner.index, 2);
        assert_eq!(race.winner.seed, 33);
        assert_eq!(race.received.len(), 2);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn race_seed_respects_max_concurrency_bound() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let active_for_run = Arc::clone(&active);
        let maximum_for_run = Arc::clone(&maximum);
        let race = race_seed_candidates(&[1, 2, 3, 4, 5, 6], 2, move |index, seed| {
            let current = active_for_run.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            maximum_for_run.fetch_max(current, AtomicOrdering::SeqCst);
            thread::sleep(Duration::from_millis(10));
            active_for_run.fetch_sub(1, AtomicOrdering::SeqCst);
            Ok(candidate(index, seed, seed as f64))
        })
        .expect("bounded seed attempts should finish");

        assert_eq!(race.winner.seed, 1);
        assert!(maximum.load(AtomicOrdering::SeqCst) <= 2);
        assert_eq!(race.received.len(), 5);
    }

    #[test]
    fn race_seed_uses_exact_penalty_then_unrounded_area() {
        let incumbent = candidate_with_area(0, 11, 1.0, 1.0);
        let lower_penalty_with_larger_area = candidate_with_area(1, 22, 1.0 - 1e-8, 300.0);
        assert!(candidate_is_better(
            &lower_penalty_with_larger_area,
            &incumbent
        ));

        let larger_area = candidate_with_area(0, 11, 1.0, 300.0);
        let smaller_area = candidate_with_area(1, 22, 1.0, 100.0);
        assert!(candidate_is_better(&smaller_area, &larger_area));

        let exact_tie = candidate_with_area(2, 33, 1.0, 100.0);
        assert!(candidate_is_better(&exact_tie, &smaller_area));
    }

    #[test]
    fn race_seed_selector_prefers_lower_score_before_request_index() {
        let lower_score = candidate(0, 11, 9.0);
        let higher_score = candidate(2, 33, 10.0);
        assert!(candidate_is_better(&lower_score, &higher_score));
        assert!(!candidate_is_better(&higher_score, &lower_score));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn race_seed_waits_for_every_candidate_before_selecting() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

        let slow_candidate_finished = Arc::new(AtomicBool::new(false));
        let slow_candidate_finished_by_run = Arc::clone(&slow_candidate_finished);
        let race = race_seed_candidates(&[11, 22], 2, move |index, seed| {
            if seed == 11 {
                thread::sleep(Duration::from_millis(50));
                slow_candidate_finished_by_run.store(true, AtomicOrdering::SeqCst);
                Ok(candidate(index, seed, 1.0))
            } else {
                Ok(candidate(index, seed, 2.0))
            }
        })
        .expect("all configured seed attempts should finish");

        assert_eq!(race.winner.seed, 11);
        assert_eq!(race.received.len(), 1);
        assert!(slow_candidate_finished.load(AtomicOrdering::SeqCst));
    }

    #[test]
    fn race_seed_rejects_empty_requests() {
        let result = race_seed_candidates(&[], 1, |index, seed| Ok(candidate(index, seed, 0.0)));

        assert!(matches!(result, Err(LayoutError::NoSeeds)));
    }

    #[test]
    fn single_seed_layout_publishes_that_seed() {
        let result = layout_result(
            &Graph::default(),
            &LayoutOptions {
                seeds: vec![73],
                max_concurrency: 0,
            },
        )
        .expect("an empty graph with one seed should layout");

        assert_eq!(result.report.selected_seed, 73);
        assert_eq!(result.report.candidates.len(), 1);
        assert_eq!(result.report.candidates[0].seed, 73);
    }

    #[test]
    fn prescale_snapshot_stops_before_hierarchy_recognition() {
        let snapshot = layout_snapshot(&Graph::default(), 1, LayoutStage::Prescale);

        assert!(snapshot.hierarchy_assignments.is_empty());
        assert!(snapshot.hierarchy_placements.is_empty());
        assert!(snapshot.hierarchy_rng_shuffle_lengths.is_empty());
    }
}

// Observe the public result handoff, preserving graph iteration and IEEE bits.
#[cfg(feature = "diagnostic-traces")]
fn trace_label_commit(
    input: &Graph,
    labels: &BTreeMap<EdgeId, EdgeLabel>,
    commits: &std::collections::BTreeSet<EdgeId>,
    stage: &str,
) {
    if std::env::var("D2_TRACE_LABEL_COMMIT").as_deref() != Ok("1") {
        return;
    }
    for (id, edge) in input.edges() {
        if commits.contains(&id)
            && let Some(label) = labels.get(&id)
        {
            let identity = input.tala_edge_ids.get(id.0 as usize).copied().flatten();
            eprintln!(
                "LABEL_COMMIT stage={stage} edge={} from={} to={} position={} percentage={:016x}",
                identity.unwrap_or(i64::from(id.0)),
                input
                    .node(edge.source)
                    .map_or("", |node| node.external_id.as_str()),
                input
                    .node(edge.target)
                    .map_or("", |node| node.external_id.as_str()),
                label.position.d2_name(),
                label.percentage.to_bits(),
            );
        }
    }
}
#[cfg(test)]
mod normalize_gaps_turn_cache_tests {
    use super::*;

    fn fixed_gap_fixture(with_edge: bool) -> (Pipeline, [NodeId; 2]) {
        let mut input = Graph::default();
        let ids = ["a", "b"].map(|name| {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 100.0,
                    height: 100.0,
                },
            ))
        });
        if with_edge {
            input.add_edge(crate::Edge {
                source: ids[0],
                target: ids[1],
            });
        }
        let mut pipeline = Pipeline::new_untraced(&input, 1, true, true);
        for (index, id) in ids.into_iter().enumerate() {
            let position = Point {
                x: 1000.0 + 1000.0 * index as f64,
                y: 1000.0,
            };
            pipeline.graph.nodes[id.0 as usize].tala_id = index as i64 + 1;
            pipeline.graph.nodes[id.0 as usize].fixed_top_left = Some(position);
            pipeline.graph.set_position(id, position);
        }
        pipeline.graph.compute_cell_size();
        assert_eq!(pipeline.graph.cell_size, 100.0);
        (pipeline, ids)
    }

    fn geometry_bits(graph: &ArenaGraph, ids: [NodeId; 2]) -> [[u64; 4]; 2] {
        ids.map(|id| {
            let position = graph.active_node_position(id).unwrap();
            let size = graph.active_node_size(id);
            [
                position.x.to_bits(),
                position.y.to_bits(),
                size.width.to_bits(),
                size.height.to_bits(),
            ]
        })
    }

    fn actual_gap_wrapper_returns_cleared_turn(with_edge: bool) {
        let (mut pipeline, ids) = fixed_gap_fixture(with_edge);
        let before = geometry_bits(&pipeline.graph, ids);
        let changed = pipeline.gap_normalization_stage().unwrap();
        let returned = geometry_bits(&pipeline.graph, ids);
        println!(
            "GAP_TURN_NATIVE {}",
            serde_json::json!({
                "with_edge": with_edge, "changed": changed, "geometry_before": before,
                "geometry_returned": returned, "raw_turn_bits": pipeline.graph.raw_turn_cost().to_bits(),
                "crossing_bits": pipeline.graph.raw_crossing_cost().to_bits(),
                "edge_length_cache_entries": pipeline.graph.cost_cache.edge_length_entries(),
            })
        );
        assert_eq!(
            returned, before,
            "actual fixed-node wrapper preserves geometry"
        );
        assert_eq!(
            pipeline.graph.raw_turn_cost().to_bits(),
            0.0_f64.to_bits(),
            "original Go NormalizeGaps returns raw turn zero before any subsequent reader"
        );
        assert!(changed, "Go reports successful no-op commits as changed");
        let expected = if with_edge { 112.5_f64 } else { 0.0_f64 };
        let crossing = pipeline.graph.raw_crossing_cost().to_bits();
        let first = pipeline.graph.turn_cost_value();
        let subsequent = pipeline.graph.turn_cost_value();
        assert_eq!(first.to_bits(), expected.to_bits());
        assert_eq!(subsequent.to_bits(), expected.to_bits());
        assert_eq!(pipeline.graph.raw_crossing_cost().to_bits(), crossing);
        assert_eq!(geometry_bits(&pipeline.graph, ids), before);
    }

    #[test]
    fn normalize_gaps_wrapper_returns_lazy_turn_with_edge() {
        actual_gap_wrapper_returns_cleared_turn(true);
    }

    #[test]
    fn normalize_gaps_wrapper_returns_lazy_turn_without_edges() {
        actual_gap_wrapper_returns_cleared_turn(false);
    }

    #[test]
    fn normalize_gaps_wrapper_resets_current_and_stale_cache() {
        for stale in [false, true] {
            let (mut pipeline, ids) = fixed_gap_fixture(true);
            if stale {
                pipeline.graph.set_position(
                    ids[1],
                    Point {
                        x: 1900.0,
                        y: 1000.0,
                    },
                );
            }
            let setup = pipeline.graph.turn_cost_value();
            assert_eq!(setup, if stale { 100.0 } else { 112.5 });
            pipeline.graph.set_position(
                ids[1],
                Point {
                    x: 2000.0,
                    y: 1000.0,
                },
            );
            assert_eq!(pipeline.graph.raw_turn_cost(), setup);
            let before = geometry_bits(&pipeline.graph, ids);
            assert!(pipeline.gap_normalization_stage().unwrap());
            assert_eq!(pipeline.graph.raw_turn_cost().to_bits(), 0);
            assert_eq!(geometry_bits(&pipeline.graph, ids), before);
            assert_eq!(pipeline.graph.turn_cost_value(), 112.5);
        }
    }

    #[test]
    fn normalize_gaps_turn_first_reader_uses_candidate_geometry_and_survives_rejection() {
        let (pipeline, ids) = fixed_gap_fixture(true);
        let mut candidate = pipeline.graph.clone();
        candidate.set_position(
            ids[1],
            Point {
                x: 2700.0,
                y: 1000.0,
            },
        );
        assert_eq!(pipeline.graph.raw_turn_cost().to_bits(), 0);
        assert_eq!(candidate.turn_cost_value(), 200.0);
        drop(candidate); // Ordinary speculative rejection restores geometry only.
        assert_eq!(pipeline.graph.raw_turn_cost(), 200.0);
        assert_eq!(
            pipeline.graph.active_node_position(ids[1]).unwrap().x,
            2000.0
        );
    }

    #[test]
    fn normalize_gaps_turn_independent_owners_and_snapshot_do_not_alias() {
        let (pipeline, _) = fixed_gap_fixture(true);
        assert_eq!(pipeline.graph.turn_cost_value(), 112.5);
        let mut independent = pipeline.graph.clone();
        prepare_compound_candidate_state(&mut independent);
        independent.reset_turn_cost();
        assert_eq!(pipeline.graph.raw_turn_cost(), 112.5);
        assert_eq!(independent.raw_turn_cost().to_bits(), 0);
        let retained = pipeline.snapshot(LayoutStage::GapNormalization);
        pipeline.graph.reset_turn_cost();
        assert_eq!(retained.turn_cost, 112.5);
        assert_eq!(retained.graph.unwrap().raw_turn_cost(), 112.5);
        independent.fresh_cost_owner();
        independent.set_turn_cost(7.0);
        assert_eq!(pipeline.graph.raw_turn_cost().to_bits(), 0);
    }

    #[test]
    fn normalize_gaps_turn_raw_snapshot_does_not_materialize() {
        let (pipeline, _) = fixed_gap_fixture(true);
        let retained = pipeline.snapshot(LayoutStage::GapNormalization);
        assert_eq!(retained.turn_cost.to_bits(), 0);
        assert_eq!(retained.graph.unwrap().raw_turn_cost().to_bits(), 0);
        assert_eq!(pipeline.graph.raw_turn_cost().to_bits(), 0);
    }
}

#[cfg(test)]
#[path = "refinement_api_tests.rs"]
mod refinement_api_tests;

#[cfg(test)]
mod optional_proposal_contract {
    use super::*;
    #[test]
    fn source_optional_errors_and_panics_preserve_incumbent_and_context() {
        let incumbent = 17usize;
        let error =
            consider_optional_proposal::<usize>(None, || Err(LayoutError::SeedInvariant)).unwrap();
        assert_eq!(error, None);
        assert_eq!(incumbent, 17);
        assert_eq!(
            consider_optional_proposal::<usize>(None, || panic!("optional invariant")).unwrap(),
            None
        );
        assert_eq!(
            consider_optional_proposal(None, || Ok(Some(19usize))).unwrap(),
            Some(19)
        );
        assert!(matches!(
            consider_optional_proposal::<usize>(None, || Err(LayoutError::Cancelled)),
            Err(LayoutError::Cancelled)
        ));
        let token = CancellationToken::new();
        assert!(matches!(
            consider_optional_proposal::<usize>(Some(&token), || {
                token.cancel();
                panic!("interrupted proposal")
            }),
            Err(LayoutError::Cancelled)
        ));
        let token = CancellationToken::new();
        assert!(matches!(
            consider_optional_proposal(Some(&token), || {
                token.expire();
                Ok(Some(19usize))
            }),
            Err(LayoutError::DeadlineExceeded)
        ));
    }
}

#[cfg(test)]
#[path = "compound_cancellation_api_tests.rs"]
mod compound_cancellation_api_tests;
