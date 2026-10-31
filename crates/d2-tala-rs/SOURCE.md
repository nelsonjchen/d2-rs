# Corresponding source

## TALA scoring ancestry allocation (2026-10-26, Lane I64)

Pinned master `6a7b48378` `internal/layoutgraph/layout.go:314–331`
(`isDescendantOf`), `node.go:2354–2410` (container levels and shared
ancestors), and `internal/placementcost/edge_length.go:875–926`
(ordered obstruction headers) map to `layoutgraph/owner_score_frame.rs`.
The native cycle/deduplication sets now keep the first sixteen identities
inline, with the existing BTreeSet membership fallback for deeper walks.
Only membership storage changes: traversal, first-repeat errors, nil
precedence, live owner/header reads, cancellation polls and all floating
point operations retain their order. The source Go recursion/map traversal
does not require native heap allocation for each shallow ancestry query.
The deep-chain regression exercises the overflow path and cycle precedence;
performance and full SVG/TALA raster evidence is recorded in
`.reference/codex-runs/iso64/` and `report-iso64.md`.

This is an unpublished development checkout of d2-rs. There is not yet a public
release source URL. Keep this complete checkout, its Git revision and all local
modifications alongside any locally shared binary or WASM artifact. Do not
substitute the archived Weftan repository for the source of d2-rs modifications.

The upstream source and historical Rust input are pinned in
reference-manifest.json. Development references can be reconstructed with
scripts/references.py and docs/REFERENCES.md. Instrumentation patches are tracked
under tools/patches; ignored references must not contain undisclosed changes.

The source-derived math table in `crates/d2-rs-math/src/glyphs.rs` is generated
from `github.com/d2lang/mathjax-go v0.1.0/internal/font/data_normal_gen.go`,
the MathJax 3.2.2 TeX font data used by the pinned D2 release. Reproduce it
with `uv run --no-project python scripts/generate-math-glyphs.py
/path/to/mathjax-go/internal/font/data_normal_gen.go
crates/d2-rs-math/src/glyphs.rs` before extending the subset. This process uses
the verified local source and does not execute JavaScript or a browser.

Before publication, supply a verified accessible source URL and exact source
revision for every artifact. Include LICENSE, THIRD_PARTY_NOTICES.md,
the repository root's DEPENDENCY_NOTICES.md and this file with every binary/WASM distribution and
crate. Source recipients must receive the MPL-covered modifications needed to
build their version. Third-party material retains its applicable terms.

Checkpoint511: Actual arena construction resolves ordinary original Object.Text fonts after extraction, preserving cached SetDimensions boxes and explicit sizes. A dispatcher-level prototype changed an unprocessed grid actor incorrectly and was replaced by the original adapter boundary. The native suite passes1234tests with25existing historical ignores; inherited tests do not qualify full original-Go parity. Source-derived adapter/CLI evidence remains mandatory. Evidence: .reference/sequence-extracted-grid-20261007/source-comparison.md and qualification.json. Full text/style/routing, structured-header and broader PLAN gates remain open. Existing MPL-2.0 and third-party notices are preserved.

TALA step-edge publication (2026-10-23 run 12) follows pinned
`d2layouts/d2talalayout/patch.go:158–239` and
`internal/grouping/sequences.go:78–137`: sequence-defining connections are
permanently absent from the published edge inventory. Native LayoutResult
carries explicit `removed_edges` while retaining stable input arena IDs;
cache restoration and nested layout reinsertion remap these IDs alongside
routes. Target JSON/hash/SVG omit those edges, and the legacy SVG route gate
still rejects unrelated missing routes. Native D2 JSON publication uses the
same explicit inventory. Witness: `tests/regressions/e2e/tala-step-edge-removal.d2`
(exact pinned-Go SVG); scoreboard targets are TALA
`regression/nested_steps` (now exact) and `stable/investigate` (now renders;
geometry still differs). MPL-2.0 notices remain applicable.

TALA compound-result publication (2026-10-23 run 15) follows pinned
`d2layouts/d2talalayout/patch.go:330–347`: `buildObjectPatch` copies the
selected node's live `TopLeft`. `engine/compound_candidate.go:113–148`
reroutes and normalizes the accepted rigid-container proposal. Native
`refresh_seed_layout_result` now reads `graph.position(node.input_id)`, as
the ordinary snapshot does, rather than the stale arena record's position.
The complete Go/Rust `SelectedResult` events for `stable/direction` already
matched before this fix; only the exported result coordinates diverged.
Witness: `tests/regressions/e2e/tala-compound-live-position.d2`, target
2101581790 and complete normalized SVG equality with pinned Go.
Full e2e scoreboard: 905 → 908 exact SVGs, zero regressions (run15-e2e-delta.txt).
No layout heuristics or comparison normalization changed.

## Isolated-tree check uses every root of the sentinel (2026-10-23)

`trees/preprocess_helpers.go:623–648` (isIsolatedTreeGuarded) accepts a
sentinel when each of its current edges is the sentinel edge of some root in
`g.Trees[node]`. Rust compared only against the placement tree being placed.
When `buildPlacementTrees` splits a sentinel's roots by direction/arrowhead, the
first placed tree's reconnected root edge then made the second tree
non-isolated, so its root label skipped `positionEdgeLabels` (UNLOCKED_MIDDLE
0.5 for a single child, `geometry.go:330–338`) and label placement saw an unset
position. Found with an instrumented Go tree-label trace and the D2 label
candidate trace. Witness `tests/regressions/e2e/tala-isolated-tree-sibling-label.d2`
(`a <-> b`, `b -> c: test`; pinned-Go target 1738344229, byte-identical SVG).
Full upstream delta: +1 exact SVG / zero regressions (TALA
`txtar/link-on-connections`).

## Compound candidate reads live positions (2026-10-23)

Go keeps one `TopLeft` per node. In the arena, owner-bound points are the live
position (`ArenaGraph::position`) while `nodes[i].position` is a copy that
`set_position` does not refresh for bound nodes. Two readers used the stale copy:

- `apply_compound_flow` (`engine/compound_candidate.go:16–70`, PlaceCompound):
  routing's `node_rect`/`route_line` then measured `reroutePlaced`'s
  StraightEdgesFallback direct lines from the pre-normalization frame and
  replaced valid routes. The accepted candidate now refreshes the arena copies
  from the live points.
- `score_existing_label_placements_checked` (`quality/labels.go:16–243`) used
  stale node rectangles for label overlap areas, so the incumbent and the
  compound proposal received wrong label scores and `considerSeedCandidate`
  (`layout.go:290–316`) kept the wrong drawing. It now reads live positions.

Found by diffing TALA_TRACE_JSONL stage snapshots (all 48 stages, including
`Compound/*`, now equal pinned Go) and instrumented candidate/quality scores in
`../d2-v0.9.0-trace` (penalties 5.1957 vs 4.5 now reproduced exactly).
Witness `tests/regressions/e2e/tala-compound-live-label-score.d2` (reduced from
`stable/elk_shim`; pinned-Go target 4236667171). Full upstream delta: +3 exact
SVG / zero regressions (TALA `stable/{dagre_spacing_right,elk_shim,investigate}`).

## Herd fences after a committed sized swap (2026-10-23)

`placement/sized_optimizer.go:380–392,1187–1199` calls `SyncHerdFences` right
after `optimizerSwapPositions`; `proximity/herding.go:330–347` reads the bounding
box of the graph being optimized (Rust: the owner view). The Rust swap moved
only the workspace copy and refreshed the owner lazily, so the herd sync wrote a
pre-swap fence (bottom 707 instead of 688) and later candidate scores diverged.
The committed swap now publishes the workspace positions to the owner
(`update_current_member_positions`) before `sync_herd_fences_with_owner`; the
Go-derived herd-sync contract records (owner geometry) still pass. Found by diffing Go/Rust
placement events (first divergence: sized pass 98 inside `with`), then sized
move results, candidate scores, edge-length tails (`EDGE_HERD_GO`) and herd sync
bounds. Witness `tests/regressions/e2e/tala-herd-fence-after-swap.d2` (reduced
from `txtar/sql-icon`; pinned-Go target 3938423341). Full upstream delta: +1
exact SVG / zero regressions.

## Mirror reachability expands aggregate vessels (2026-10-23)

`placement/direct.go:167–260` (mirrorAxes) mirrors every node whose container
was reached by `AllReachableNodesContext(includeContainers=true)`, which
(`layoutgraph/node.go:1767–1782`) also queues a cluster vessel's members and a
sequence's steps. `restored_mirror_reachable_containers` walked edges and
ancestry only, so a scope whose only path into a nested container ran through a
cluster member (`stable/large_arch`: `r.gg -> r.s.t`) left `r.s` unreachable and
its children were translated instead of reflected. The walk now expands vessel
members from cluster/sequence records and, in compact placement graphs, from
projected aggregate children. Full upstream delta: +1 exact SVG / zero
regressions (TALA `stable/large_arch`).

## Horizontal hierarchy transpose and left/up mirrors (2026-10-23)

`hierarchy/placement.go:225–249` (placeNodesInHierarchy) transposes a horizontal
hierarchy (`IsHorizontal`, `orientation.go:8–21`: the first node's container
direction is left/right, or any member is a table) before placeNodesByLevel,
keeps it transposed through alignHierarchy/syncContainers, transposes back, and
then mirrors X for `left` and Y for `up` (`iterContainersBFS`, Node.transpose
`layoutgraph/node.go:2409–2416`, Node.mirror). The Rust hierarchy placement
applied none of these, so every horizontal hierarchy was laid out top-down.
`apply_hierarchy_placement{,_traced}` now wrap placement and container copyback in
those transforms; container padding stays untransposed, as Go recomputes it from
labels. Witness `tests/regressions/e2e/tala-horizontal-hierarchy-transpose.d2`
(all TALA stages equal pinned Go). Full upstream delta: +1 exact SVG / zero
regressions (TALA `regression/straight_hierarchy_container_direction_right`).

## Common-uncle siblings include active cluster vessels (2026-10-23)

`proximity/common_uncle.go:11–45` walks `graph.Containers[container]`, whose
children are the active cluster vessels, and follows each child's `Node.Edges`
(a vessel's are its members' abducted edges). `compute_common_uncle_map` took
the vessel's own (empty) arena edges, so vessels never joined an uncle group
and `elementos.coupling` scored a two-node group instead of Go's three-node
group (common-uncle penalty 99 vs 198 per candidate). Vessel children now take
their edges through the cluster's first member. Found from sized move results,
per-candidate scores, `EDGE_TAIL_GO`/flow tails and Go/Rust common-uncle maps.
Full upstream delta: +1 exact SVG / zero regressions (TALA
`real_world/ross_overview`); every applicable TALA e2e board now matches.

## Seed-2 TALA divergences (2026-10-24)

Three independent source mismatches surfaced only with `--tala-seeds=2`
(and so the CLI default `1,2,3`); every scoreboard pins seed 1.

- AlignAxes route obstructions (`stable/investigate`). `placement/alignment.go`
  passes `Sequence.EdgeAbductions` to EdgeLength on the live graph;
  `edge_length.go:869–899` walks `node.Graph.Containers` from the replacement
  endpoints. `prepare_root_sequence_alignment_records` records carry no scope
  inventory, but the route probe selected `ProjectedObstructionSource::Ordered`
  with their empty slice and tested no obstruction (Go blocks `cc -> dd.hh`
  with `dd.ee`, passThrough 1998 per side). An abduction with neither a
  projected slice nor `sets_present` now uses the live walk. That walk stores an
  active sequence vessel in its first member's slot (`active_node_position`), so
  the skip checks (`edge_length.go:914,925`) compare that slot as the vessel;
  before this the vessel's own box blocked its edges (3996 instead of 1998).
- Graph.direct counts (`stable/large_arch`). `containerEdgeDirections`
  (`placement/direct.go`) reads each owned child's `Node.Edges` after
  RestoreEdgeAbductions. An endpoint reconnected to an active cluster vessel by
  AddClusters is not restored, so `i.m -> c`/`i.m -> f` stay on the vessel and
  count (Go counts 3,3,3,2 and mirrors Y). The Rust direct count skipped every
  edge with overrides on both sides; a cluster-vessel reconnection no longer
  counts as restored.
- Cluster members in FitToGraph (`real_world/tpmjs_architecture`). AssignHerds
  (`proximity/herding.go:126–136`) flips a cluster's arrangement without
  resizing; `Cluster.SyncGeometry` later resizes the vessel and
  ArrangeClusterNodes centres members on its cross axis
  (`layoutgraph/cluster.go:74–82,121–141`). The fit reconstruction used member
  offsets recorded with the pre-flip vessel size (y offset 102 inside an 82px
  Row vessel, fit height 304 vs 202). It now centres on the placed vessel.

Evidence: `TALA_TRACE_EDGE_DETAIL_COMPONENTS` / `ROUTE_OBSTRUCTION_INVENTORY`,
`TALA_TRACE_DIRECT_ROOT` / `WEFTAN_TRACE_DIRECT_ROOT`, placement snapshots and
`WEFTAN_TRACE_FIT_ALL`. All three boards equal pinned Go at seeds 1, 2, 3 and
the default; seed-2/3 sweep of every e2e case: +3, zero regressions
(`stable/cycle-order` is nondeterministic in Go itself). e2e SVG 933/933.

## Preserve enclosed routes after the compound proposal (2026-10-24)

Upstream master `b34d2e936` ("tala: preserve enclosed routes when compound
containers move"; pinned via `6a7b48378`).

- `layout.go` `Layout`: when the compound proposal replaced the ordinary seed
  winner, `refineSeedResult(selected, PreserveCompoundRoutes(ordinary,
  selected))` runs and is accepted without a score comparison; an ordinary
  error/panic keeps `selected`, context errors propagate. Rust:
  `race_seed_layouts` keeps the ordinary winner and calls
  `build_preserved_compound_routes` through `consider_optional_proposal`
  (same failure semantics) in `engine/snapshot.rs`; the result is rescored
  with `refresh_seed_layout_result` (`evaluateSeedResult`'s score and
  non-finite check; Rust has no completed-graph validator on this path, as for
  the compound proposal itself).
- `engine/compound_routes.go` -> `engine/compound_routes.rs`:
  `PreserveCompoundRoutes` -> `preserve_compound_routes` (Clone =
  `selected.clone()` + `prepare_compound_candidate_state`; IsCurve copied from
  the selected edge); `preserveCompoundInteriors` -> `preserve_compound_interiors`
  + `restore_compound_interiors` (fixed node label/icon and fixed edge label
  restore from the ordinary graph; enclosed routes translated by the outer
  root's TopLeft delta with IsCurve/label/percentage from the ordinary edge;
  then `cost_cache.reset_placement`, `Pipeline::run_place_labels`
  (labeling.Place), `normalize_seed` (placement.Normalize)); `compoundRoot` /
  `compoundEnclosedRoute` -> `compound_root` / `compound_enclosed_route`
  (`labels::edge_label_top_left` for `Edge.LabelTopLeft`,
  `labels::arrowhead_label_rect_for_route` for `labelgeom.ArrowheadTopLeft`).
  Edges pair by `member_edge_indices` order like Go's `Graph.Edges` index.

Witness: `stable/elk_shim` TALA (edge label percentage 0 -> 0.325 and its
interior routes). Unit tests mirror the bus case of
`TestCompoundCandidatePreservesInteriorBusNestedAndLoopRoutes` and
`TestCompoundCandidateKeepsSelectedRouteForUnsafeInterior`. e2e TALA
svg-match 308 (was 305 + 2 target-match + 1 target-mismatch), zero regressions.

## Master optional-refinement original-call observations (Lane I59)

Pin `6a7b4837838a3907604a1d9eec57ee98b6f3cd01`:
`d2layouts/d2talalayout/layout.go:308–345` (`considerSeedCandidate`,
`refineSeedResult`) maps to `engine/snapshot.rs::consider_optional_proposal`,
the score gate in `build_compound_candidate`, and unconditional publication
after `build_preserved_compound_routes` in `race_seed_layouts`.

`refinement_test.go`'s three original functions are observed by
`tools/tala-refinement-scoreboard/hook_test.go`, with all Go assertions active.
The input-only native unit-test replay calls the production optional-proposal
wrapper and score comparator. Graph, sequence-edge and nil translation-binding
payloads remain opaque: this observes acceptance, fallback, publication and
context precedence, not routing geometry, serialization or pointer identity.
Errors expose no published value at Rust's `Result` boundary; Go's error-path
selected-value identity assertions remain Go-only. The ordinary higher-score
control observes the production comparator, not the compound-layout builder.
No production algorithm change is introduced by the replay.

## Master compound-interior original-call observations (Lane I60)

Pin `6a7b4837838a3907604a1d9eec57ee98b6f3cd01`:
`d2layouts/d2talalayout/internal/engine/compound_routes.go:19–103`
(`PreserveCompoundRoutes`, `preserveCompoundInteriors`) maps to
`engine/compound_routes.rs::{preserve_compound_routes,
preserve_compound_interiors,restore_compound_interiors}`.

`tools/tala-interior-scoreboard/hook_test.go` captures the 13 original
refinement calls in five `compound_interior_routes_test.go` functions,
with original assertions active. `engine/interior_api_tests.rs` reconstructs
only their before/selected inputs, restores curve/fixed flags and original
zero IDs omitted/assigned by graphjson, and invokes production refinement.
Ordered node/edge geometry, labels, curve flags and both unmodified inputs
compare exactly. Node label text and label-less edge percentages remain opaque
input metadata, outside the native arena's mutable state. Preceding compound placement/rerouting is source-supplied
input, not qualified by this replay. Cost caches, pointer identity, direction
map storage and the two cancellation functions are outside this observation.
The only production-file edit is a `cfg(test)` module declaration.

## Original compound cancellation observations (2026-10-26, Lane I61)

Pinned master `6a7b48378`, `engine/compound_interior_routes_test.go:358–373,
406–443` exercises pre-cancellation and three calibrated midflight cancellations
of `CompoundCandidate` followed by `PreserveCompoundRoutes`.
`scripts/tala-cancellation-scoreboard.py` retains the original calls and active
Go assertions. The input-only test adapter calls native
`snapshot::build_compound_candidate` and
`compound_routes::preserve_compound_routes`. An infinite test incumbent admits
the pre-selection proposal; no expected result is used by native execution.
The successful calibration compares ordered geometry and label/curve state;
cancellation compares the typed cause, null candidate and unchanged input.
Raw complete diagnostics are retained separately because the source assertion
uses `errors.Is`, and the independent cancellation schedules reach different
stages. Go calibrates `Context.Err` calls; the native `cfg(test)` observer
calibrates the existing stage/publication checks using the same fractions.
This does not qualify identical polls, within-stage cancellation, wrapped
error text, pointer identity or caches. Test-only graph projections share
I60's documented metadata exclusions. No runtime algorithm is changed.

## Borrowed score-frame edge mapping (2026-10-26, Lane I65)

At pin `6a7b48378`, `placementcost/edge_length_scorer.go:16–31,37–72`
retains incident topology through a candidate sweep and reads live geometry
on every score; `placement/sized_optimizer.go:888–900,961` invokes that scorer for each
candidate. Rust's `layoutgraph/owner_score_frame.rs` mapping is an arena-ID
adapter, separate from the actual Graph.Edges membership and Node.Edges.
Explicit mappings now borrow the stable session slice. The session and
sizeless/compaction readers construct this frame directly, avoiding the
temporary Graph.Edges snapshot and replacement Vec on every score. Default
frames still own their original snapshot. The former snapshot reader's nil
member panic retains its precedence through an allocation-free scan.
Arena/node/edge validation order,
live owner/header reads, iteration, arithmetic and cancellation are unchanged.
The regression covers live membership replacement, retained mappings and
node-before-edge/first-edge error precedence. Qualification and performance
limits are recorded in `.reference/codex-runs/report-iso65.md`.

The same frame module now reuses empty obstruction-set vectors, following
`placementcost/edge_length.go:50,64–96,861–902`. A checkout exclusively owns
its buffer, releases every captured header on return/error, and retains no
geometry. Nested active captures remain independent. Native per-thread
retention is capped at four empty buffers with capacity at most 4096 each;
larger buffers are dropped. The existing retained-header regression also
checks repeated reuse while an outer capture remains live. Poll boundaries
and the source-ordered inventory are unchanged.
