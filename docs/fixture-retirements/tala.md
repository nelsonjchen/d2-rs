# Retired TALA oracle fixtures (`crates/d2-tala-rs`)

Retired per AGENTS.md ("Retire fixtures that pin unreachable internal states (adversarial callbacks, poll accounting) with a written reason"). Each fixture and every test that read it were deleted. The data remains at base commit `9b706ac1` (`git show 9b706ac1:<path>`) and in the private `nelsonjchen/d2-rs-history` repository. The captures were made at v0.9.0 (d5a51743) with untracked probes under `.reference/tala-*-2026101x/` and `2026102x/`; their harnesses were not carried into the repository.

Recipe'd instead (tools/fixtures/tala-oracle.toml): `label-obstacles-source-v090.json`, `table-port-member-obstacles-source-v090.json`, `tree-containers-source-v090.json`, `arrowhead-label-codec-source-v090.json`, `tree-publication-source-v090.json`, `route-validation-source-v090.json`, `tala-labelled-graph-bounds-v090.json` (complete layouts of upstream TALA inputs, codec round trips, stage graphs, public-API route validation and edge-label bounds, all reachable from D2 input or public APIs).

The first pass left the small companion fixtures (under 256 KB) whose only readers it had deleted; the second pass deleted them as orphans (see "Second pass" below). It also retired `tala-scorer-nil-adjacent-v090.json`, the one file the first pass kept for `placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer`.

## Failing witness retired with its family

`placement/transaction_container_refit_source_tests.rs::original_container_refit_and_pre_sync_predicate_match_owned_geometry_and_work` failed at the base commit (one of the three known default-tier failures). It compares `container-refit-source-v090.json`: the native result differs from the capture on all 30 `empty-oval-*` rows (every limit/cancel offset, including no limit and no cancellation); the Go capture records no error and no panic for a container-flagged oval whose child map is empty. That arena state (container flag without children) is constructed directly by the harness; D2 input does not produce it. The divergence is recorded here rather than fixed; `layoutgraph` is unchanged between v0.9.0 and the pin, so a master capture would not change it.

## Transaction, packing-geometry and compaction callback matrices

32 files, 141,007,714 bytes.

Reason: Go `internal/layoutgraph` Transaction lifecycle, packing geometry capture/clone/cost/queue/slice, hierarchy normalize stages, cluster optimization and compaction, each run on a hand-built arena under a `context.Err` callback that fires at every poll offset and cancels, panics, rebinds `Vessel`/`Graph.Nodes`, duplicates entries or writes through shared point aliases, crossed with work limits. Rows pin error/panic text, poll and work counters, pointer/allocation sharing flags and per-phase point bits. A D2 caller (or a public `d2talalayout` API caller) can only cancel a context or hit a deadline; it cannot run code inside an internal poll, so these states are unreachable. The paired `*-inputs-v090.json` files are the request grids of these harnesses and have no other use.

| Fixture | Size | Tests deleted (or trimmed) |
| --- | ---: | --- |
| `captured-overlap-inputs-v090.json` | 317,379 | `placement/captured_overlap_source_tests.rs::original_overlap_ownership_survives_poll_rebinding_clones_updates_and_rollback` |
| `captured-overlap-source-v090.json` | 8,018,474 | `placement/captured_overlap_source_tests.rs::original_overlap_ownership_survives_poll_rebinding_clones_updates_and_rollback` |
| `compaction-live-candidates-source-v090.json` | 794,251 | `placement/compaction_live_candidate_source_tests.rs::original_compaction_live_candidates_match_every_first_mutation_poll` |
| `compaction-moves-source-v090.json` | 947,666 | `placement/compaction_moves_source_tests.rs::original_compaction_moves_match_context_work_and_live_points` |
| `compaction-whole-source-v090.json` | 6,585,196 | `placement/compaction_whole_source_tests.rs::original_whole_compaction_matches_failures_and_exact_shared_move_limit` |
| `container-map-fields-source-v090.json` | 1,885,381 | `layoutgraph/owner_container_fields_tests.rs::original_go_container_map_field_lifetimes_and_rollback` |
| `container-refit-inputs-v090.json` | 444,315 | `placement/transaction_container_refit_source_tests.rs::original_container_refit_and_pre_sync_predicate_match_owned_geometry_and_work` |
| `container-refit-source-v090.json` | 5,379,170 | `placement/transaction_container_refit_source_tests.rs::original_container_refit_and_pre_sync_predicate_match_owned_geometry_and_work` |
| `edge-sweep-polls-inputs-v090.json` | 1,273,706 | `placement/edge_sweep_poll_source_tests.rs::original_edge_sweep_matches_live_ranges_fields_and_topology_restore` |
| `edge-sweep-polls-source-v090.json` | 12,508,697 | `placement/edge_sweep_poll_source_tests.rs::original_edge_sweep_matches_live_ranges_fields_and_topology_restore` |
| `gap-checkpoint-source-v090.json` | 538,338 | `placement/gap_normalization.rs::original_gap_reducer_matches_nested_checkpoint_results_and_owner_state` |
| `geometry-checkpoint-cow-source-v090.json` | 6,536,006 | `placement/gap_normalization.rs::inherited_gap_checkpoint_refresh_failures_match_original_transaction`<br>`placement/geometry_checkpoint_source_tests.rs::geometry_checkpoint_matches_original_shared_refresh_and_failure_atomicity` |
| `metered-geometry-capture-source-v090.json` | 1,853,286 | `placement/metered_geometry_capture_source_tests.rs::metered_geometry_capture_matches_original_poll_mutation_and_rollback` |
| `multicluster-cache-source-v090.json` | 762,186 | `placement/multicluster_cache_source_tests.rs::original_multicluster_cache_and_translated_vessels_match` |
| `normalize-recovery-source-v090.json` | 14,729,828 | `hierarchy/normalize_stage_source_tests.rs::normalize_recovery_under_controlled_go_map_order_matches` |
| `normalize-stage-inputs-v090.json` | 308,326 | `hierarchy/normalize_stage_source_tests.rs::original_normalize_stage_cost_aliases_contexts_and_atomic_failure_match` |
| `normalize-stage-source-v090.json` | 3,749,943 | `hierarchy/normalize_stage_source_tests.rs::original_normalize_stage_cost_aliases_contexts_and_atomic_failure_match` |
| `optimize-cluster-atomic-source-v090.json` | 2,856,566 | `grouping/optimize_cluster_source_tests.rs::original_optimize_cluster_atomic_failure_and_reentry_match`<br>`grouping/optimize_cluster_source_tests.rs::successful_optimize_clusters_preserve_held_geometry_and_cost_owners`<br>`grouping/optimize_cluster_source_tests.rs::empty_optimize_clusters_checks_shared_guard_before_return`<br>`grouping/optimize_cluster_source_tests.rs::optimize_cluster_entry_faults_precede_policy_geometry_and_cost_capture` |
| `optimize-cluster-container-source-v090.json` | 5,301,673 | `grouping/optimize_cluster_source_tests.rs::controlled_optimize_clusters_container_ownership_and_reentry_match` |
| `overlap-sweep-polls-inputs-v090.json` | 734,355 | `placement/overlap_sweep_poll_source_tests.rs::original_overlap_sweep_matches_live_poll_pointer_and_geometry_loads` |
| `overlap-sweep-polls-source-v090.json` | 2,688,880 | `placement/overlap_sweep_poll_source_tests.rs::original_overlap_sweep_matches_live_poll_pointer_and_geometry_loads` |
| `packing-clone-source-v090.json` | 5,718,587 | `packing/packing_clone_source_tests.rs::original_packing_geometry_clone_copies_ops_shares_prior_and_refreshes_atomically` |
| `packing-cost-source-v090.json` | 9,690,951 | `packing/packing_cost_source_tests.rs::original_request_cost_capture_attach_clone_consume_restore_preserves_maps_and_bits` |
| `packing-geometry-inputs-v090.json` | 275,330 | `packing/packing_geometry_source_tests.rs::packing_geometry_capture_matches_original_members_groups_tree_and_route_option` |
| `packing-geometry-source-v090.json` | 7,921,916 | `packing/packing_geometry_source_tests.rs::packing_geometry_capture_matches_original_members_groups_tree_and_route_option` |
| `packing-prior-inputs-v090.json` | 544,898 | `packing/packing_prior_source_tests.rs::original_packing_commit_keeps_constructor_captured_geometry_as_its_prior` |
| `packing-prior-source-v090.json` | 13,709,038 | `packing/packing_prior_source_tests.rs::original_packing_commit_keeps_constructor_captured_geometry_as_its_prior` |
| `packing-queue-source-v090.json` | 3,157,554 | `packing/packing_queue_source_tests.rs::original_packing_queue_range_clear_replay_and_atomic_commit_match` |
| `packing-slice-source-v090.json` | 4,954,857 | `packing/packing_slice_source_tests.rs::original_operation_slice_views_nil_assignment_and_clone_ownership_match` |
| `route-capture-inputs-v090.json` | 623,138 | `packing/route_capture_source_tests.rs::original_routed_geometry_capture_preserves_live_headers_points_and_restore` |
| `route-capture-source-v090.json` | 10,221,578 | `packing/route_capture_source_tests.rs::original_routed_geometry_capture_preserves_live_headers_points_and_restore` |
| `transaction-options-source-v090.json` | 5,976,245 | `packing/transaction_options_source_tests.rs::original_transaction_options_refit_containment_clone_and_refresh_match` |

## Bin-packing group-work and sequence-walk matrices

15 files, 64,280,588 bytes.

Reason: Go `internal/packing` and `internal/grouping` bin-pack traversal (`binPackSyncSequences`/`Clusters`, `SyncGeometryWithWork`, commit tail, root aliases, routed containers, sequence owner walks) on synthetic owner graphs: mutation callbacks at every poll (`mutate_at`, `mutation`, `alias_mode`), legacy mirrors, nil/duplicate IDs, retired members and receiver/vessel rebinding, crossed with cancellation offsets and work limits. Same unreachability as above: they qualify Go aliasing and poll accounting, not layout behavior D2 input can select.

| Fixture | Size | Tests deleted (or trimmed) |
| --- | ---: | --- |
| `compact-sequence-walk-source-v090.json` | 341,034 | `grouping/sequence_compact_sync_tests.rs::compact_sequence_walk_matches_source_reachability_and_postorder` |
| `packing-commit-tail-source-v090.json` | 2,942,701 | `packing/binpack_commit_source_tests.rs::original_packing_commit_tail_matches_validation_work_aliases_and_atomic_rejection` |
| `packing-group-live-headers-source-v090.json` | 8,340,582 | `packing/binpack_group_work_source_tests.rs::original_guarded_group_live_headers_policy_and_vessels_match` |
| `packing-group-map-source-v090.json` | 1,066,012 | `packing/binpack_group_work_source_tests.rs::original_guarded_group_maps_override_compact_mirrors` |
| `packing-group-shared-points-source-v090.json` | 3,297,966 | `packing/binpack_group_work_source_tests.rs::original_guarded_group_shared_points_and_mutating_callback_allocations_match` |
| `packing-group-work-source-v090.json` | 2,335,000 | `packing/binpack_group_work_source_tests.rs::original_guarded_group_walk_geometry_partial_state_and_aliases_match` |
| `packing-live-graph-nodes-source-v090.json` | 4,223,368 | `packing/binpack_group_work_source_tests.rs::original_guarded_live_graph_node_headers_match` |
| `packing-live-group-map-fields-source-v090.json` | 3,067,202 | `packing/binpack_group_work_source_tests.rs::original_guarded_live_graph_group_map_fields_match` |
| `packing-live-node-graph-source-v090.json` | 2,897,774 | `packing/binpack_group_work_source_tests.rs::original_guarded_live_node_graph_fields_match` |
| `packing-receiver-vessel-source-v090.json` | 2,500,548 | `packing/binpack_group_work_source_tests.rs::original_guarded_receivers_and_bound_vessels_match` |
| `packing-root-alias-source-v090.json` | 4,372,429 | `packing/binpack_root_alias_source_tests.rs::original_root_point_aliases_survive_wrapping_constraints_and_rejection` |
| `packing-root-range-source-v090.json` | 14,284,639 | `packing/binpack_group_work_source_tests.rs::original_guarded_root_ranges_match` |
| `packing-wide-sequence-source-v090.json` | 13,238,134 | `packing/binpack_group_work_source_tests.rs::original_guarded_wide_sequences_match` |
| `routed-container-source-v090.json` | 934,980 | `packing/binpack_routed_container_source_tests.rs::original_routed_container_decisions_segments_and_work_match` |
| `sequence-walk-source-v090.json` | 438,219 | `grouping/sequence_owner_sync_tests.rs::original_sequence_walk_matches_graph_owners_postorder_revisits_and_panic_prefix`<br>`grouping/sequence_compact_sync_tests.rs::compact_sequence_walk_matches_source_reachability_and_postorder` |

## Sized-optimizer and placement-cost poll matrices

28 files, 83,031,737 bytes.

Reason: Go `internal/placement` sized-optimizer internals (spatial index, candidate shuffle/movement/scoring context, generation, entry/setup/median, scoring work, neighbor requirements, hub retry) and `internal/placementcost` labeled scorers, evaluated on two- or three-node synthetic frames at every cancellation offset `at` and work limit, many with mid-evaluation mutation (`mutate_at`, replacement records, aliased/resized/foreign owners, constructor re-entry after editing edge records). Rows record `polls`, `used` and the error text at each offset. That is poll accounting of internal guards, unreachable from D2 input; the success-path values feed every TALA layout and are covered by the upstream TALA e2e goldens, which all match.

| Fixture | Size | Tests deleted (or trimmed) |
| --- | ---: | --- |
| `tala-candidate-movement-v090.json` | 350,924 | `placement/sized.rs::guarded_candidate_search_matches_source_movement_and_rollback` |
| `tala-candidate-scoring-context-deep-v090.json` | 4,396,122 | `placement/sized.rs::candidate_search_same_context_matches_source` |
| `tala-candidate-scoring-context-v090.json` | 350,674 | `placement/sized.rs::candidate_search_same_context_matches_source` |
| `tala-generation-v090.json` | 2,320,737 | `placement/sized_generation.rs::canonical_generation_matches_source_points_scratch_capacity_work_and_cancellation` |
| `tala-index-overlap-v090.json` | 2,212,360 | `placement/optimizer_spatial_index_tests.rs::owner_spacing_matches_source_integer_rules_and_nil_geometry`<br>`placement/optimizer_spatial_index_tests.rs::indexed_and_fallback_overlap_match_source_guard_and_spacing`<br>`placement/optimizer_spatial_index_tests.rs::guarded_ring_search_matches_source_distance_checked_offsets_and_errors` |
| `tala-indexed-move-v090.json` | 483,761 | `placement/optimizer_spatial_index_tests.rs::indexed_can_move_matches_source_live_fallback_errors_and_candidate_headers` |
| `tala-labeled-scorer-alias-v090.json` | 1,740,987 | `placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer` (trimmed) |
| `tala-labeled-scorer-v090.json` | 533,611 | `placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer` (trimmed) |
| `tala-neighbor-requirements-v090.json` | 356,323 | `placement/sized_neighbor_requirement_tests.rs::full_neighbor_requirements_and_candidate_order_match_source_constructor_reentry` |
| `tala-owner-hub-retry-v090.json` | 328,976 | `placement/owner_hub_spokes_tests.rs::original_hub_suppression_headers_work_errors_and_panic_controls`<br>`placement/sized.rs::hub_retry_reads_foreign_owner_missing_from_workspace_and_restores_on_cancel` |
| `tala-owner-labels-v090.json` | 4,460,970 | `placementcost/labeled_scorer_tests.rs::full_owner_label_terms_match_source` |
| `tala-owner-obstructions-v090.json` | 1,354,435 | `placementcost/labeled_scorer_tests.rs::selected_owner_obstruction_scores_match_source` |
| `tala-owner-tables-v090.json` | 1,893,507 | `placementcost/labeled_scorer_tests.rs::full_owner_table_terms_match_source` |
| `tala-owner-tail-v090.json` | 1,596,477 | `placementcost/labeled_scorer_tests.rs::canonical_sized_tail_terms_match_source` |
| `tala-point-shuffle-v090.json` | 8,115,873 | `placement/sized_point_shuffle.rs::candidate_point_shuffle_matches_source_partial_state_work_and_rng` |
| `tala-scorer-cluster-receiver-v090.json` | 1,014,603 | `placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer` (trimmed) |
| `tala-scorer-foreign-v090.json` | 4,516,515 | `placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer` (trimmed) |
| `tala-scorer-member-v090.json` | 1,290,027 | `placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer` (trimmed) |
| `tala-scorer-receiver-v090.json` | 4,967,523 | `placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer` (trimmed) |
| `tala-scorer-resize-alias-v090.json` | 1,896,507 | `placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer` (trimmed) |
| `tala-scorer-resize-v090.json` | 585,451 | `placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer` (trimmed) |
| `tala-scorer-retained-resize-v090.json` | 2,069,099 | `placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer` (trimmed) |
| `tala-scoring-work-v090.json` | 1,044,676 | `placement/sized_scoring_work.rs::scoring_charge_matches_source_actual_graph_counts_repeated_work_and_errors` |
| `tala-sized-entry-v090.json` | 2,034,743 | `placement/sized.rs::sized_entry_guards_and_fixed_worklist_match_source` |
| `tala-sized-median-v090.json` | 719,059 | `placement/sized.rs::sized_candidate_median_matches_source_live_children_and_guard_order` |
| `tala-sized-setup-v090.json` | 1,163,977 | `placement/sized_setup.rs::canonical_sized_setup_matches_source_partial_maps_work_errors_and_origin` |
| `tala-spatial-index-v090.json` | 17,829,169 | `placement/optimizer_spatial_index_tests.rs::spatial_rebuild_query_occupancy_and_reuse_match_source` |
| `tala-table-incidence-v090.json` | 13,404,651 | `placementcost/labeled_scorer_tests.rs::table_incidence_poll_writes_match_source` |

## Owner geometry aliasing matrices

5 files, 20,583,461 bytes.

Reason: Herd synchronization, numerical herd members, live owner bounds, nested descendant receivers and view tree-map copies under replaced or held records, raw orientation values outside the enum, shared Go slice/map identity flags and cancellation offsets. These pin Go aliasing of internal records, not behavior reachable from D2 input or a public API.

| Fixture | Size | Tests deleted (or trimmed) |
| --- | ---: | --- |
| `tala-live-herd-sync-v090.json` | 11,364,337 | `placement/sized_herd_sync_tests.rs::original_sync_uses_live_owner_fields_and_preserves_aliases` |
| `tala-live-owner-bounds-v090.json` | 584,097 | `placement/sized_herd_sync_tests.rs::original_sync_reads_live_owner_geometry_and_members` |
| `tala-nested-descendants-v090.json` | 398,523 | `layoutgraph/owner_swap_motion_tests.rs::nested_descendant_owner_receivers_match_source` |
| `tala-numerical-herd-members-v090.json` | 7,858,665 | `proximity/numerical_herd_members_tests.rs::original_numerical_sync_uses_live_members_and_raw_fields` |
| `tala-view-tree-copy-v090.json` | 377,839 | `layoutgraph/owner_view_tree_copy_tests.rs::prepared_component_tree_map_copy_matches_original_source` |

Total: 80 files, 308,903,500 bytes, all under `crates/d2-tala-rs/tests/fixtures/`.

## Second pass: small fixtures (`d2-tala-rs`, `d2-rs-layout`, `d2-rs-scenebuild`)

The second pass covered every remaining tracked data file under these crates' test and fixture areas (120 files, 7.2 MB). As above, deleted data remains at `9b706ac1` and in `nelsonjchen/d2-rs-history`.

### Recipe'd

All regenerated at the pin and compared with the `9b706ac1` bytes on arm64: every artifact's data is identical; only metadata keys changed (`source_commit` is now the pin, added `upstream`/`input_sha256`, dropped unread `source_path`/`groups`/`sources` keys), and the scene-build captures differ only in the canonicalized paths, ports and Date headers listed below.

| Recipe (file) | Artifacts | Generator |
| --- | --- | --- |
| `tala-engine-graphjson-owners`, `tala-engine-serialize`, `tala-engine-layout-errors` (`tala-oracle.toml`) | `cluster-hub-owner`, `container-direct-owner`, `container-refit-owner`, `cluster-owner-restore`, `packing-member-obstacles`, `cluster-policy`, `crossing-cache`, `hub-incidence`, `tree-mirror`, `live-graphjson`, `seeded-cluster-sequence-publication`, `graphjson-layout-errors` (`*-source-v090.json`) | `tala_oracle_fixtures.py layout`: complete seeded `engine.Layout` of upstream TALA testdata; seven hand-built graphs in `tools/fixtures/seeds/tala-live-graphjson.json`; dimension-limit graphs generated in the script |
| `tala-stages-cluster-creation`, `tala-stages-preprocess`, `tala-stages-nested-cluster-initialization` | `cluster-creation/*` (9; `input.json` is the upstream file itself), `preprocess-stage-serialization-source-v090.json`, `nested-cluster-initialization-source-v090.json` | `tala_oracle_fixtures.py stage-sets`: the existing stage hook, graph after each named stage |
| `tala-route-geometry` | `requested-routing-source-v090.json`, `route-geometry-source-v090.json` | new `tools/tala-route-geometry-oracle` (public `d2talalayout.RouteEdges`); object variants in `tools/fixtures/seeds/tala-route-geometry.json`, the rest generated |
| `tala-authored-geometry` | `authored-geometry-source-v090.json` | new hook `tools/fixtures/hooks/tala-authored-geometry` (`validateD2Objects` on D2 width/height/top/left attribute bytes); requests generated |
| `tala-tree-sentinel-hierarchy` | `tree-sentinel-hierarchy-eligibility-v090.json` | `layout_oracle_fixtures.py tala-tree-sentinel` (d2lib + TALA seed 1); D2 source and native graph in `tools/fixtures/seeds/tala-tree-sentinel-hierarchy.json` |
| `tala-go-rng-int63n` (`tala-runtime.toml`) | `src/internal/runtime/fixtures/go127-int63n.json` | `tools/fixtures/programs/tala-go-rng` (Go `math/rand`), Go toolchain key only |
| `layout-sequence`, `layout-grid-fixed-placement`, `layout-elk-nested-sequence-grid` (`layout-oracle.toml`) | `sequence-group-label-adjustment`, `sequence-note-source-order`, `sequence-lifeline-source-order`, `sequence-invalid-positions`, `grid-fixed-placement`, `elk-nested-sequence-grid` (`*-source-v090.json`) | `layout_oracle_fixtures.py` + new `tools/d2-layout-oracle` (d2lib with dagre/ELK/TALA; `-overlay` observer on `d2layouts.validateObjectPositions`); D2 sources in `tools/fixtures/seeds/layout-sequence-sources.json`, grid sources generated, ELK native graph in `tools/fixtures/seeds/layout-elk-nested-sequence-grid.json` |
| `layout-grid-capacity`, `layout-grid-search` | `grid-capacity-master.json`, `grid-search-master.json` | the scoreboard hooks in `tools/grid-{capacity,search}-scoreboard`, applied with `-overlay` |
| `scenebuild-fallback`, `scenebuild-assets`, `scenebuild-imageasset-*` (`scenebuild-oracle.toml`) | `bundled-fallback-master.json`, `image-assets-master.json`, `latex-master.json`, the eight `imageasset-*-master.jsonl` | `scenebuild_oracle_fixtures.py`: the scoreboard rewrites and hooks applied with `-overlay`; upstream-tree paths, `t.TempDir()` roots, httptest ports and HTTP `Date` headers canonicalized in requests and results alike |

The native adapter graphs in the two `tree-sentinel`/`elk-nested-sequence-grid` seeds are d2-rs lowering output (test inputs, not oracle output); the D2 sources beside them are what the recipes compile in Go.

### Orphans deleted

The twelve `*-inputs-v090.json` request grids that the first pass left behind, 1,686,556 bytes: `gap-checkpoint`, `geometry-checkpoint-cow`, `metered-geometry-capture`, `multicluster-cache`, `normalize-recovery`, `optimize-cluster-atomic`, `optimize-cluster-container`, `packing-clone`, `packing-cost`, `packing-queue`, `packing-slice`, `transaction-options` (`crates/d2-tala-rs/tests/fixtures/<name>-inputs-v090.json`). No test or script read them.

### Retired TALA internals

#### Poll, cancellation and mutation-callback matrices

45 files, 2,536,357 bytes, all under `crates/d2-tala-rs/tests/fixtures/`; tests under `crates/d2-tala-rs/src/internal/`.

Reason: Go `internal/placement`, `packing`, `grouping`, `hierarchy`, `layoutgraph` and `routing` (OVG) internals run on hand-built arenas under a `context.Err` callback that fires at every poll offset `at`/`cancel_at` and cancels, panics, rebinds owners, replaces records or writes through shared aliases, crossed with work limits and deadlines. Rows pin error and panic text, `polls`/`used` counters, pointer/header sharing flags and rollback point bits. A D2 caller (or a public `d2talalayout` caller) can only cancel a context or hit a deadline; it cannot run code inside an internal poll. These are the same harness families the first pass retired for their large files, here their small members.

| Fixture | Size | Tests deleted |
| --- | ---: | --- |
| `aggregate-refit-overlaps-v090.json` | 15,492 | placement/transaction_container_refit_source_tests.rs::original_aggregate_prior_overlaps_use_canonical_vessel_allocations (file deleted; its second input, `cluster-policy-source-v090.json`, is now recipe'd) |
| `alignment-cache-inputs-v090.json` | 176,450 | placement/alignment_cache_key_source_tests.rs::alignment_cache_keys_and_source_seeded_hits_match (file deleted) |
| `alignment-cache-source-v090.json` | 96,770 | (same test) |
| `compaction-candidates-source-v090.json` | 118,318 | placement/compaction_candidate_source_tests.rs::original_compaction_candidates_enforce_range_and_context_contracts (file deleted) |
| `compaction-range-rollback-source-v090.json` | 7,370 | placement/compaction_candidate_source_tests.rs::original_compaction_range_failure_restores_points_children_and_routing |
| `compaction-context-source-v090.json` | 9,486 | placement/compaction_context_source_tests.rs::original_transition_compaction_restores_subtrees_on_cancellation_and_panic (file deleted) |
| `compaction-outer-source-v090.json` | 24,890 | placement/compaction_outer_source_tests.rs::original_sized_and_sizeless_compaction_entry_matches_source (file deleted) |
| `compaction-projection-source-v090.json` | 8,271 | placement/compaction_moves_source_tests.rs::original_compaction_projected_endpoint_matches_real_owner_and_slice_presence (file deleted) |
| `compaction-shift-positions-source-v090.json` | 70,586 | placement/compaction_shift_source_tests.rs::original_sizeless_shift_restores_exact_points_before_final_move (file deleted) |
| `gap-point-rollback-source-v090.json` | 7,708 | placement/gap_normalization_tests.rs::gap_rollback_matches_original_point_identity_and_values |
| `gap-cluster-rollback-source-v090.json` | 23,306 | placement/gap_normalization_tests.rs::gap_rollback_matches_original_cluster_policy_and_vessel_geometry<br>placement/optimization.rs::swap_rollback_matches_original_cluster_geometry<br>placement/alignment.rs::alignment_rollback_matches_original_cluster_geometry |
| `gap-selected-geometry-source-v090.json` | 76,214 | placement/gap_normalization_tests.rs::gap_rollback_matches_original_selected_geometry_and_live_fields<br>placement/optimization.rs::swap_rollback_matches_original_selected_geometry<br>placement/alignment.rs::alignment_rollback_matches_original_selected_geometry |
| `geometry-header-trees-source-v090.json` | 198,140 | placement/trial_geometry_tests.rs (shared helper, file deleted) and its callers gap_normalization_tests.rs::gap_rollback_matches_original_header_and_trees, optimization.rs::swap_rollback_matches_original_header_and_trees, alignment.rs::alignment_rollback_matches_original_header_and_trees |
| `geometry-update-work-inputs-v090.json` | 131,338 | placement/geometry_update_work_source_tests.rs::geometry_update_work_matches_original_object_identity_headers_maps_limits_and_cancellation (file deleted) |
| `geometry-update-work-source-v090.json` | 93,285 | (same test) |
| `normalize-cold-inputs-v090.json` | 2,213 | hierarchy/normalize_stage_source_tests.rs::original_normalize_cold_repeated_and_nested_stage_programs_match (file deleted) |
| `normalize-cold-source-v090.json` | 61,295 | (same test) |
| `ovg-edge-header-source-v090.json` | 723 | routing/ovg_edge_header.rs::retained_header_states_match_original_go |
| `ovg-endpoint-rewrite-v090.json` | 6,514 | routing/ovg_ports.rs::retained_rewritten_endpoints_keep_their_foreign_incident_objects |
| `ovg-foreign-edge-source-v090.json` | 24,207 | routing/ovg_ports.rs::merged_incident_edges_keep_original_objects_and_endpoint_lifetimes |
| `ovg-nil-point-priority-source-v090.json` | 4,369 | routing/ovg_ports.rs::nil_point_priority_and_partial_edge_publication_match_original_go |
| `ovg-merge-alias-source-v090.json` | 15,155 | routing/ovg_ports.rs::hierarchy_merge_retains_whole_nodes_across_both_builds |
| `ovg-live-port-blocker-source-v090.json` | 18,378 | routing/ovg_intersections.rs::source_live_port_is_read_after_blocker_callbacks |
| `ovg-live-query-source-v090.json` | 18,437 | routing/ovg_intersections.rs::source_live_query_is_read_after_callbacks |
| `ovg-port-map-source-v090.json` | 39,365 | routing/ovg_intersections.rs::source_port_map_lookup_observes_removal_and_restoration |
| `packing-commit-operation-source-v090.json` | 48,601 | packing/binpack_operation_source_tests.rs::real_commit_operation_errors_and_panics_restore_live_aliases (file deleted) |
| `packing-overlap-snapshot-source-v090.json` | 126,325 | placement/transaction_overlap_source_tests.rs::transaction_overlap_snapshots_match_original_go_at_every_captured_boundary (file deleted) |
| `packing-trial-preparation-source-v090.json` | 103,141 | packing/binpack_preparation_source_tests.rs::original_trial_preparation_preserves_inventory_candidates_deltas_and_failures (file deleted) |
| `post-snapshot-lifetimes-source-v090.json` | 16,376 | layoutgraph/owner_post_snapshot_lifetime_tests.rs::original_go_post_snapshot_graph_and_abduction_lifetimes (file deleted) |
| `post-snapshot-node-lifetimes-source-v090.json` | 26,480 | layoutgraph/owner_post_snapshot_lifetime_tests.rs::original_go_post_snapshot_canonical_node_lifetimes |
| `sequence-sync-source-v090.json` | 139,186 | grouping/sequence_sync_source_tests.rs::sequence_sync_matches_original_go_geometry_and_local_point_replacement (file deleted)<br>packing/binpack_guard.rs::real_sequence_vessel_cache_rolls_back_with_geometry_on_every_trial_exit |
| `tala-column-incidence-v090.json` | 21,947 | placement/table_columns.rs::column_incidence_matches_original_source_with_retained_edges |
| `tala-column-peer-incidence-v090.json` | 22,295 | placement/table_columns.rs::column_incidence_matches_original_source_node_edge_peer_scan |
| `tala-connected-tree-v090.json` | 10,703 | placement/sized.rs::sized_optimizer_connected_tree_members_match_source |
| `tala-direction-headers-v090.json` | 35,653 | layoutgraph/owner_directions_api.rs::independent_direction_headers_copy_replace_nil_and_fork_match_source |
| `tala-eligibility-error-causes-v090.json` | 69,175 | placement/sized.rs::owner_node_eligible_preserves_source_error_causes |
| `tala-median-error-causes-v090.json` | 87,040 | placement/sized.rs::owner_neighbor_median_preserves_source_error_causes |
| `tala-node-incidence-v090.json` | 4,739 | placementcost/scoring.rs::node_incidence_matches_original_source_when_graph_edges_changes |
| `tala-node-incidence-global-v090.json` | 909 | placementcost/scoring.rs::node_incidence_matches_original_source_global_cache_and_symmetry |
| `tala-retained-nears-v090.json` | 73,369 | placementcost/labeled_scorer_tests.rs::retained_near_targets_use_exact_owner_boxes_source_contract (file deleted) |
| `tala-scorer-nil-adjacent-v090.json` | 109,915 | placementcost/labeled_scorer_tests.rs::labeled_scores_and_cancellation_match_repeated_source_scorer (the test the first pass trimmed to this file) |
| `tala-shared-requirements-v090.json` | 108,612 | layoutgraph/node_requirements_map.rs::shared_requirement_headers_values_nullable_keys_and_restore_match_source<br>placement/sized_generation.rs::live_requirement_updates_and_replaced_headers_match_source_poll_boundaries |
| `tala-sized-rollback-v090.json` | 14,230 | placement/sized.rs::sized_optimizer_failure_restores_owner_workspace_and_point_aliases |
| `tala-sized-scorer-lifecycle-v090.json` | 129,821 | placementcost/labeled_scorer_tests.rs::sized_scorer_lazy_retry_close_and_cancellation_match_source |
| `tala-sized-scorer-lifecycle-direction-off-v090.json` | 139,560 | (same test) |

#### Internal function values on synthetic frames

12 files, 509,707 bytes, all under `crates/d2-tala-rs/tests/fixtures/`; tests under `crates/d2-tala-rs/src/internal/`.

Reason: Go `internal/placementcost` median/incidence scoring, `internal/placement` herd sync, `internal/labeling` shared-label intervals and `internal/layoutgraph` desired-geometry fields, each evaluated directly on two- to five-node hand-built frames (projected/raw positions, nil neighbors, foreign owners, sentinel trees, raw orientation values, synthetic routes). They pin internal intermediate values, not layouts: D2 input reaches these functions only through a complete layout, whose results the upstream TALA e2e goldens cover (all match; `docs/SCOREBOARD-WORKFLOW.md`). `shared-label-intervals` replays two upstream Go unit tests (`labeling/refine_test.go`, `placement_test.go`), which upstream already runs. Same reasoning as the first pass's sized-optimizer family.

| Fixture | Size | Tests deleted |
| --- | ---: | --- |
| `tala-median-cell-owner-v090.json` | 30,099 | placementcost/scoring.rs::median_cell_owner_matches_original_source |
| `tala-median-member-nears-v090.json` | 85,647 | placementcost/scoring.rs::median_member_nears_match_original_source |
| `tala-median-near-trees-v090.json` | 43,395 | placementcost/scoring.rs::median_near_tree_projection_matches_original_source |
| `tala-median-near-groups-v090.json` | 12,403 | placementcost/scoring.rs::median_near_group_projection_matches_original_source |
| `tala-median-near-scope-v090.json` | 5,115 | placementcost/scoring.rs::median_near_scope_matches_original_source |
| `tala-median-position-gate-v090.json` | 2,315 | placementcost/scoring.rs::median_filters_nil_current_neighbor_before_abduction |
| `tala-median-incidence-v090.json` | 20,679 | placementcost/scoring.rs::median_incidence_matches_original_source |
| `tala-equidistance-incidence-v090.json` | 2,649 | placementcost/scoring.rs::median_incidence_matches_original_source_equidistance |
| `tala-herd-route-bounds-v090.json` | 28,503 | placement/sized_herd_sync_tests.rs::original_sync_includes_routed_graph_bounds (file deleted) |
| `tala-owner-join-publication-v090.json` | 14,467 | placement/sized_herd_sync_tests.rs::original_cluster_join_publishes_points_before_owner_fences |
| `shared-label-intervals-source-v090.json` | 171,085 | labeling/labels.rs::original_shared_label_intervals_preserve_gaps_and_live_edges |
| `desired-geometry-source-v090.json` | 93,350 | layoutgraph/arena.rs::measured_box_authored_dimensions_and_locked_coordinates_are_independent |

### Retired layout fixtures without recoverable D2 sources

4 files, 449,751 bytes, under `crates/d2-rs-layout/tests/fixtures/`; tests in `crates/d2-rs-layout/src/`.

| Fixture | Size | Reason | Tests deleted (or trimmed) |
| --- | ---: | --- | --- |
| `tala-cylinder-padding-source-v090.json` | 347,775 | No generator survives, and the 41 controls (three shapes x four positions x plain/fixed/icon profiles, four grid cylinders, one "lion reader" diagram) carry only d2-rs-lowered graphs with measured label sizes: the D2 text, labels and icon URLs that produced them are not recorded, so the Go side cannot be rebuilt from source. Cylinder, package and rectangle container padding under TALA is exercised by the upstream TALA e2e goldens (all match). | `lib.rs::tala_cylinder_padding_matches_original_go_complete_object_boxes` |
| `sequence-extracted-text-source-v090.json` | 93,885 | Six v0.9.0 probe rows of the TALA adapter's text-font bookkeeping (`pre`/`adapter` declared fonts and label levels) with no recorded D2 source; the rows are probe instrumentation, not a public output. Sequence-in-grid font sizing is covered by the upstream goldens. | `lib.rs::extracted_text_source_contract` (module) |
| `sequence-span-endpoint-source-v090.json` | 7,796 | One case named `imported_variable` whose D2 source and imported file were not recorded (empty-label spans `a.x`, `a.y` from an import), so it cannot be recompiled. | trimmed from `sequence.rs::complete_boxes_and_message_routes_match_original_go_targets` (the other three inputs of that test are recipe'd) |
| `sequence-empty-label-source-v090.json` | 295 | Five hand-placed boxes that Go's internal `adjustGroupLabel` leaves unchanged for a cleared group label: an internal function on a synthetic frame, not D2 output. | `sequence.rs::empty_group_label_source_contract` (module) |

Second-pass total: 61 retired files (57 TALA, 4 layout), 3,495,815 bytes, and 12 orphans, 1,686,556 bytes.
