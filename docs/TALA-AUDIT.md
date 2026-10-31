# TALA source audit — 2026-10-01

Evidence JSON cited below as `docs/*.json` is no longer in the tree. It was
retired at 9b706ac1 and in the 2026-10-31 second pass; `docs/RETIRED-EVIDENCE.md`
lists each file and how to read it back. Checkers now write fresh reports
under the gitignored `oracle-cache/reports/`.

## Cluster branch routes, root adjacency, and OVG order — 2026-10-01

The expanded `big_connected.json` trace exposed an index mismatch in Rust's
`fixClusterEdgeBranching`: the Go implementation scores each candidate against
`g.Edges`, but Rust paired a cluster-only route slice with whole-graph edge
indices. Rust now snapshots all routes in graph order for each candidate and
updates the corresponding global route slot as it scores the branch group,
matching Go's sequential mutation. A unit regression adds an unrelated edge
before the cluster group and confirms that global indexing remains valid.

For source-orientation tracing, the Go field `root_edges` is the complete
`root.Edges` length, while the projected boundary list can omit loops and can
add cross-scope edges absent from the stable node's local adjacency. Rust now
tracks those quantities separately and derives the count from local adjacency
plus newly projected external edges. This aligns both trace payloads and the
minimum-degree gate with the source.

The final stable trace mismatch moved to the OVG topology event. Static source
comparison showed Go builds `NodesInsideBoundingBox` in nearby-node, routed
scope, and hierarchy-merge order, while several Rust OVG passes iterated a
`BTreeMap`. Rust now threads the source box order through fixed-overlap scans,
collision candidates, center insertion, and diagnostic snapshots. The box
order matches on this witness.

Repeated Go 1.27.0 runs still yield different OVG point and neighbor orders for
the same graph and seed. Four oracle processes produced four distinct point
orders for the 10,246-point topology, while the two 236-point topology runs
varied once. Rust's point order was stable across two runs. Despite this
internal order difference, the direct GraphJSON result matches exactly:
71 nodes and 114 routes, by stable ID, in four Go runs and two Rust runs. The
raw trace comparator therefore continues to show the first map-order
difference at event 252,138; no normalization hides it. Reproduction hashes
and per-run details are recorded in
`docs/tala-big-connected-ovg-nondeterminism-20261001.json`.

The TALA crate passes 740 tests, with 25 historical ignores and two passing
doctests. This audit closes the graph-indexed branch-route bug and the
deterministic OVG box-order gap for this case; it does not claim that other
large-graph traces or runtime map nondeterminism are resolved.

## Large multi-subgraph trace and final geometry — 2026-10-01

Static comparison of upstream `OptimizeClusters` showed that each accepted
cluster alignment calls `reduceGapToNeighbors` for a follow-up reduction. The
Go connectivity observer records that call inside the shared helper; Rust uses
a specialized cluster-gap implementation and did not emit the equivalent
event. Rust now emits the matching JSON fields from
`internal/grouping/gap_reduction.rs::cluster_gap_direct_trial`, including the
active vessel identity and source-ordered owner/fixed-node lists. The
`big_many_subgraphs.json` trace now contains the same **173** connectivity
events on both sides.

The next static mismatch was in the observer's route list. Go's
`coordinator.go` appends tree-sentinel and self-loop routes to `existingRoutes`
while walking `Graph.Nodes`; the observer sees that interleaved insertion
order before the flavor-sorted ordinary edges. Rust already retained this
order for route interaction state, but its trace rebuilt it as all tree edges
followed by all loops. `route_edges_in_order` now returns its actual
`preexisting_route_order` to the diagnostic observer. In non-diagnostic builds
that extra return value is empty; routing and public behavior are unchanged.

After these fixes, the exact normalized full trace matches when the
process-variable `ovg-topology` family alone is excluded. The repeatability
audit found **three different Go topology hashes across three runs** for the
same seed and **one stable Rust hash**; both executables' public outputs remain
stable. The strict trace still reports its first difference at event **598,698**
inside OVG point/neighbor order, and the comparator preserves that difference.
The exact-bits layout comparison nevertheless matches all **130 node records
and 197 edge routes**. Details and hashes:
`docs/graphjson-full-trace-big-many-subgraphs-seed1-20261001.json`,
`docs/tala-big-many-subgraphs-trace-repeatability-seed1-20261001.json`, and
`docs/tala-big-many-subgraphs-layout-seed1-20261001.json`.

The TALA crate passes **740 tests**, with **25 historical ignores** and **two
passing doctests**. Both diagnostic and ordinary optimized graph adapters
build. This closes the cluster-gap and pre-route trace instrumentation gaps
for this large corpus case; Go's run-variable OVG point ordering remains an
explicit unresolved trace difference.

## Source-container orientation port — 2026-09-28

Static comparison against the pinned v0.9.0 source found
`orientSourceInterior` immediately after `packing.CombineSubgraphs` and before
the containing node is fitted. The Go pass uses the source container's
outgoing boundary edges and its parent direction to decide whether a small,
plain, horizontally dominant interior should be quarter-turned. It declines
when loops, fixed ancestor obstacles, local directions, fixed nodes, or
unsupported graph features make that move unsafe.

Source mapping: upstream
`d2layouts/d2talalayout/internal/placement/node_placement.go::placeNodes`
calls `placement/container_orientation.go::orientSourceInterior`. Rust builds
the context in `internal/hierarchy/hierarchy_scope.rs::place_flat_scope`,
implements the pass in `internal/placement/container_orientation.rs`, and
publishes committed shared-pointer geometry in
`internal/hierarchy/hierarchy.rs::place_nodes_recursively`. The trace owner
cache is refreshed at `after-orient` in `internal/engine/pipeline.rs` so the
subsequent fit snapshot observes that same committed state.

Rust now captures those owner-graph inputs before projecting the direct-child
scope, then ports the eligibility checks, directed-flow reduction, center
rotation, cluster-axis adjustment, transition compaction, size/improvement
limits, and candidate validation. Visibility scans traverse the active
`Graph.Nodes` slice for both candidates and blockers, and transition compaction
uses a source-shaped `CompactionVisibility` guard. Rust also runs the guarded
`orientationContext` loop preflight before recursive placement. The pass
charges source boundary edges, interior-flow edges, and node moves under their
source operation guards.
This is source-algorithm work; it does not use a fixture-specific geometry
patch.

Boundary targets are resolved through current aggregate ownership after
cluster reconnection, matching Go's active vessel endpoints rather than raw
stable member IDs. When a candidate commits, Rust copies its active stable-node
boxes back to the owner before fitting, matching the pointer aliasing in Go's
temporary graphs. The Go 1.27.0/Rust trace now agrees at both the
`after-orient` and following `before-fit` boundaries.

The upstream direct fixture was instrumented in the separate trace worktree to
record its final boxes. Go 1.27.0 and the Rust regression agree exactly for
both directions and both internal arrow directions:

| Parent flow | Internal flow | Final top-left coordinates |
| --- | --- | --- |
| Down | Forward | `(-80,20), (-80,420), (-80,820)` |
| Down | Reverse | `(-20,-80), (-20,-480), (-20,-880)` |
| Up | Forward | `(-20,-80), (-20,-480), (-20,-880)` |
| Up | Reverse | `(-80,20), (-80,420), (-80,820)` |

The upstream active cluster fixture revealed one alias-specific mismatch. Go
moves a separate cluster vessel, but Rust stores that vessel frame alongside
the first stable member. Rust now updates the cached vessel position before
cluster synchronization. Go 1.27.0 and Rust agree exactly on source nodes
`(-80,20)` and `(-580,20)`, members `(-160,1030)` and `(-40,1030)`, and the
row vessel frame `(-160,1030)` with size `220x40`; arrangement, desired
arrangement, padding, member dimensions, and cell size also match.

The candidate-validation helper now preserves the source transaction's
`IgnoreContainerEscape` option. It skips only container containment; overlap,
fixed-position, size, and external-container checks remain active. A focused
regression confirms an escaped child can pass that option while a newly
overlapping candidate is still rejected.

Full D2 measurements for `container-orientation-source.d2`,
`container-orientation-source-reverse.d2`, and
`container-orientation-source-up.d2` match exact public bytes and normalized
traces for seeds 1 and 42 (**6/6**). These inputs leave the optional transform
inactive because their interiors are already vertical. See
`docs/tala-container-orientation-parity-20260928.json` for release and
diagnostic binary hashes and per-run event totals.

The active D2 fixture `tests/d2/container-orientation-source-active.d2`
matches exact public bytes and the complete normalized trace at seeds 1, 2,
and 42 (**3/3**, 44,816–51,420 events). Seeds 1 and 42 commit the rotation;
seed 2 rolls it back with the upstream reason. Two seed-1 executions have
identical raw traces and public JSON. Reproduction details and hashes are in
`docs/tala-container-orientation-active-parity-20260928.json`.

Placement errors now travel through the recursive-scope result and per-seed
layout boundary; diagnostic snapshots retain the internal error without
changing serialized output. Injected cancellation and work-limit tests verify
that failed candidates are not published. The TALA crate passes 687 tests and
2 doctests, with 25 historical ignores. Ancestor/fixed-container end-to-end
cases and broader TALA trace coverage remain open. This closes one source gap,
not all TALA behavior or full D2 compatibility.

# Initial static audit and strict baseline

## Self-loop fringe degree and router work guards — 2026-09-23

Static source comparison against pristine v0.9.0 showed that
`trees/preprocess_helpers.go:treeFringeNodes` checks `len(node.Edges)` before
following the one edge to its adjacent node. `Graph.Connect` stores a real
self-loop once on its node, so it contributes one to fringe degree. Rust's
preprocessor had skipped self-loops, incorrectly peeling loop-bearing nodes
and changing `PreprocessTrees` node and edge order. It now counts a raw
self-loop once, preserves the sequence-alias exclusion for collapsed
non-loop edges, and has a focused mixed-loop/tree regression asserting the
recovered node and edge order.

The same audit compared `routing/ovg_edge_router.go` construction to Rust's
route-work guard. Rust now charges the per-flavor cluster/member inventory,
endpoint adjacency scans, sort setup, and pre-existing routes at the same
boundaries as Go. Retained tree and loop routes are accounted as construction
work instead of being charged again as candidate searches. The boundary unit
test checks a nine-unit inventory and failure at limit eight. Full emitted
traces and public bytes match for the mixed-loop fixture at seeds
0, 1, 2, 7, 42, and 123456789, and the all-shapes and nested-cluster traces
remain exact at seeds 1 and 2. Reports:
`docs/tala-loop-router-work-seed-sweep-20260923.json`,
`docs/all-shapes-loop-router-replay-20260923.json`, and
`docs/nested-cluster-loop-router-replay-20260923.json`. The TALA crate suite
passed 623 tests before the herd-exclusion regression was added, with 25
historical ignores and two passing doctests. These are qualified cases, not
completion of the wider parity goal.

The expanded replay found that the herd-exclusion path read retired members
from Rust's stored container list. Go's `reduceGapToNeighbors` instead scans
the live `Graph.Containers[node.Container]` slice after aggregate replacement.
Rust now uses `container_node_order` for that scan, which removes hidden
cluster members and reintroduces the vessel in its source position. I added
owner-node and fixed-node inventories to both development trace hooks to
distinguish graph-slice identity from exclusion behavior. The first concrete
case was `queue_workers.d2`: its ordered connected sets, scores, RNG, routing,
stages, and public bytes already matched, but eight and eleven gap events
contained a duplicate active-vessel identity. After the live-slice fix, its
full trace matches at seeds 1 and 2. Retained-grid and teleport-grid also
match at both seeds; reports are
`docs/queue-workers-live-herd-order-replay-20260923.json`,
`docs/retained-grid-loop-router-replay-20260923.json`, and
`docs/teleport-grid-loop-router-replay-20260923.json`. Final TALA suite:
624 passed, 25 historical ignores, and 2 doctests passed.

## Current route-search requalification — 2026-09-23

The older `docs/routing-search-upstream-remaining.json` recorded a candidate
order divergence for `multiple_constant_nears.d2`. I rebuilt both current
oracles and reran the explicit 12-fixture, seed-1
`ShortestToLongest` matrix. All eight fixtures with routed edges now match the
ordered route-search event stream and public graph, including the 414-event
`multiple_constant_nears` case. Four fixtures have no measured edges and are
recorded as route-search-inapplicable. Current evidence is
`docs/routing-search-upstream-live-herd-20260923.json`. This resolves the stale
matrix failure; other routing flavors, seed coverage, and broader TALA/D2
qualification remain open.

## Four routing-worker trace sweep — 2026-09-23

The route-search matrix now checks all four upstream worker flavors at seeds 1
and 2 on `outside_bottom_labels.d2` and `all_shapes_link.d2`. Fourteen of the
sixteen cases match the ordered route-search trace, public graph and ordinary/
diagnostic output bytes exactly. The two remaining rows are explicitly
inapplicable: both Go and Rust assign no routes to `TopDownLeftRight` for
`outside_bottom_labels.d2`. The gate reads each implementation's emitted
`route-flavor` inventory before accepting an empty trace as inapplicable; a
worker that received routes but emitted no search events remains insufficient
evidence, and unequal route inventories are divergent. Unit tests cover these
classifications. Reports are
`docs/outside-bottom-route-search-<flavor>-20260923.json` and
`docs/all-shapes-route-search-<flavor>-20260923.json`. This is a two-fixture,
two-seed worker trace qualification, not broad TALA routing completion.

## Four routing-worker heap mutation sweep — 2026-09-23

The same 16-case matrix now enables the diagnostic queue observer and compares
`route-search-heap` independently as well as in the interleaved search stream.
This covers ordered initial pushes, per-edge pushes, popped entries, and
decrease-key calls, including exact priority bits and queue length at each
operation. The result remains **14/16 matching and 2 inapplicable**, with zero
divergences or insufficient-evidence rows. Both inapplicable rows have empty
route inventories and no search or queue operations in either implementation.
All applicable rows also retain exact public graphs and ordinary/diagnostic
output bytes. Reports are
`docs/outside-bottom-route-search-heap-<flavor>-20260923.json` and
`docs/all-shapes-route-search-heap-<flavor>-20260923.json`. Diagnostic heap
hooks are compile-time gated; this adds no instrumentation to release builds.
These two fixtures do not qualify cancellation, timeout, broader routing
topologies, or the remaining TALA pipeline.

## Upstream route-search corpus with heap operations — 2026-09-23

I expanded `ShortestToLongest` with the heap observer over the existing twelve
upstream route-search fixtures and seeds 1 and 2. The eight fixtures that have
edges match in both seeds (**16/16 applicable cases**); the four edgeless
fixtures are inapplicable in both seeds. The gate compares **245,578** ordered
search and queue events, of which **68,853** are heap mutations. Every applicable
row also has identical public graph JSON and ordinary/diagnostic bytes. There
are no divergent or insufficient-evidence rows. The aggregate report is
`docs/routing-search-upstream-heap-seeds1-2-20260923.json`; split reports keep
the captures independently reviewable. This extends detailed evidence for one
worker and two seeds; it does not establish every worker, routing topology,
timeout behavior, or all of TALA.

After the route-stage cancellation-cadence fix, the two-witness four-worker
matrix and all four worker corpus reports were regenerated from the same
release and diagnostic builds. The eight witness reports, four worker reports,
and aggregate report record release SHA-256
`c193c21ba0825e3a07460a324456a99b0b69e307c9fd80aea017a1f553319cf3` and
diagnostic SHA-256
`73ce3b4c4955ef2d60faa610221f4549880dda05f5d7a3f80d034423b25c5eaf`.

I ran the same corpus for `LongestToShortest`, `Default`, and
`TopDownLeftRight`. The first two each match **16 applicable cases** with the
same eight edgeless cases inapplicable. The corpus assigns no routes to
`TopDownLeftRight` in any row, so its 24 rows are inapplicable rather than
trace matches. That worker is exercised separately by `all_shapes_link.d2` at
seeds 1 and 2, where all **4/4** route/heap traces match with 35 routes per
seed. Across the four worker corpora, the aggregate is **48 matched, 48
inapplicable, zero divergent, and zero insufficient-evidence rows**, comparing
**729,531 search events** and **204,345 heap operations**. Reports are
`docs/routing-search-upstream-heap-<flavor>-seeds1-2-20260923.json` and
`docs/routing-search-upstream-heap-all-workers-seeds1-2-20260923.json`. This
qualifies these routing workers on the selected corpus only.

## Route-work cancellation and limit precedence — 2026-09-23

Static comparison of `route_search_guard.go` with Rust's
`LocalRouteSearchWork` found that upstream rechecks context before reporting a
step, bulk reservation, arithmetic-overflow, or shared-stage limit failure.
Rust now preserves the accepted local charge, gives cancellation precedence,
and keeps a cancelled worker unmarked as limit-exhausted. A zero-unit charge
also resets the next poll stride.

Extending the comparison to `ovg_resource.go` and `route_stage_guard.go` found
two related gaps: Rust polled every OVG work reservation instead of using the
upstream 1024-unit stride, and its shared stage counter lacked post-charge
stride checks. Rust now mirrors the OVG and aggregate cadence, checks before
resource-limit errors, preserves accepted counters when cancellation is seen
after a stride-crossing charge, and finishes deferred flavor accounting while
the worker cancellation token is installed. Deterministic tests cover local,
OVG, shared-stage, overflow, zero-charge, and deferred-finish boundaries.
Call-site review then found `reserve_checked` was polling before every helper
charge, earlier than upstream's `routeWorkGuard.add`. It now delegates directly
to the shared guard, preserving accepted in-bucket charges until the next
1024-unit boundary or explicit zero-unit checkpoint. A regression verifies
that cancellation already pending within a bucket does not cause an early
poll. Validation: **25 route-work guard tests** and the full TALA crate (**635
passed, 25 historical ignores, 2 doctests**) pass, as do formatting checks.
The corresponding pinned-source Go guard tests pass under `go1.27.0 darwin/arm64`.
This aligns guard decision boundaries; it does not establish identical
wall-clock cancellation latency or complete application timeout parity.

## Full emitted TALA traces and Go map-order variation — 2026-09-23

The complete set of 18 copied upstream D2 layout fixtures, at seeds 1 and 2,
now has 32 exact full-trace matches and four strict divergences. All 36 public
graph JSON values and ordinary release output bytes match.
`arrowhead_scaling.d2` differs only in
`gap-connectivity.ordered_connected`; `cross_arrowhead.d2` differs only in
`ordered_connected` or `movement_connected` membership order. Three repeated
Go 1.27.0 captures at each affected seed produce three distinct gap-trace
hashes with stable event counts and byte-identical public output. The pinned
`Node.connectedNodes` implementation ranges over `Graph.Clusters` and
`Graph.Containers` maps, confirming these are upstream map-order traces. The
strict comparator retains the source order and reports these rows as
divergent rather than sorting them away. The Go connected-set membership test
passes under `go1.27.0 darwin/arm64`.

The same sweep exposed a real singleton mismatch in `multiple_person_label.d2`:
Rust ran root `CombineSubgraphs` after TALA's one-node early return, shifting
the `style.multiple` node by (10, 10) before normalization. Rust now skips that
combination only for a singleton root; nested singleton scopes and roots with
additional source nodes keep their existing path. The regression verifies
the `NodePlacement` origin is exactly (0, 0), and both seeds now match every
trace event.

The aggregate report is
`docs/tala-full-trace-upstream-gap-all18-seeds1-2-20260923.json`; its three
source reports are
`docs/tala-full-trace-upstream-gap-12-seeds1-2-20260923.json`,
`docs/tala-full-trace-upstream-gap-remaining6-seed1-20260923.json`, and
`docs/tala-full-trace-upstream-gap-remaining6-seed2-20260923.json`. Repeatability
evidence is
`docs/arrowhead-scaling-go-repeatability-seed1-20260923.json`,
`docs/arrowhead-scaling-go-repeatability-seed2-20260923.json`, and
`docs/cross-arrowhead-go-repeatability-seeds1-2-20260923.json`. The parity
harness reports matched, divergent, insufficient-evidence, and capture-failed
rows separately, so strict differences are not mislabeled as incomplete
captures.

The complete pinned TALA Go test tree passes under Go 1.27.0 with vet enabled.
Vet identified and prevented a format typo in the diagnostic-only combine
trace (`%d` for a float); the trace hook and checked-in patch now use `%g`.
After rebuilding the instrumented oracle, the singleton regression still
matches both seeds. This formatting-only correction does not alter TALA's
algorithm or emitted JSONL trace events.

### Route-identity requalification — 2026-09-23

After carrying OVG-node identities through all final-route producers, the
18-fixture source corpus still matches public graph JSON and ordinary output
bytes on all 108 rows across seeds 1, 2, 3, 5, 7, and 42. Strict traces match
on 93 rows. Fifteen differences are Go map-order fields in `gap-connectivity`:
`arrowhead_scaling` and `cross_arrowhead` at all six seeds, and
`nested_layout_bug` at seeds 2, 5, and 42. Ten independent Go captures confirm
the seed-2 nested case has two observed trace hashes while Rust stays stable;
comparison order remains strict. Existing repeatability evidence covers the
other 14 rows. The aggregate classifies all 15 as upstream-nondeterministic,
leaving no unclassified rows. See
`docs/tala-full-trace-upstream-gap-all18-seeds1-2-3-5-7-42-requal-20260923.json`
and `docs/nested-layout-bug-oracle-repeatability-seed2-20260923.json`.

## Current expanded GraphJSON requalification — 2026-09-23

The prior expanded report listed four final-geometry failures at seeds 7, 17,
and 99. Those rows came from older Rust/oracle binaries. After rebuilding the
optimized Rust engine and verifying the instrumented Go oracle against the
current trace patch, the complete 37-fixture benchmark/regression corpus
matches **111/111** at those three seeds. This includes the previous
`big_many_subgraphs` seeds 7, 17, and 99 and `container_escape_3` seed 17.
The refreshed report is
`docs/graphjson-expanded-current-20260923-seeds7-17-99.json`; the former
failures should no longer be treated as current divergences. This is a
final-geometry requalification and does not prove the full operation trace or
all D2 TALA capabilities.

## Current direct-engine trace requalification — 2026-09-23

A fresh five-seed run first showed an identical 37-versus-38 stage count on
every direct GraphJSON fixture. Static source inspection found the sole extra
event: Rust's `SelectedResult` is emitted at the D2 adapter's post-race
selection boundary, while the Go GraphJSON oracle calls
`internal/engine.Layout` and does not pass through `d2talalayout.Layout`. The
Go trace patch confirms its `TraceSelectedGraph` call belongs only to that
outer wrapper. The checker now excludes this wrapper-only event for the
internal-engine corpus; it does not filter any engine stage or operation.
Final geometry remains separately compared, and wrapper-level selected-result
snapshots remain in the emitted-trace gate. The direct source fixture matrix
now matches **220/220** at seeds 0, 2, 7, 42, and 123456789. The updated
checker, its boundary regression, and provenance-bearing report are
`scripts/check-graphjson-trace.py`, `scripts/test_graphjson_trace.py`, and
`docs/graphjson-trace-parity-current-20260923.json`. This closes the comparator
scope error, not the broader TALA trace or capability gates.

## Exact multi-seed ranking — 2026-09-23

The pinned Go wrapper's source comparator ranks each completed candidate by
exact quality penalty, then by unrounded `Graph.Area()`. Rust had instead
included the legacy rounded area term in one scalar and used a `0.0001`
precision window. On `sequence-trace-order.d2`, individual seeds 1, 2, and 3
each produced the same geometry as Go, but the combined call selected seed 3
instead of Go's seed 1.

Rust now compares the exact penalty tuple and only consults graph area for an
exact penalty tie. The area path follows the live node/edge slices, preserves
fractional node bounds, and includes source-rounded edge bounds, labels, and
icons. The obsolete rounded-area calculation was removed from the production
seed path. Unit tests cover close-but-distinct penalties, area ties, and
request-order ties. The optimized multi-seed output now matches the pinned Go
source build byte-for-byte on six fixtures: `sequence-trace-order`,
`all_shapes_link`, `nested-lock`, `retained-grid-cluster-gap`,
`nested-direction-grid`, and `right-near`. Rust's selection probe reports
seed 1 for `sequence-trace-order` and seed 3 for the other five. The pinned Go
release's multi-seed output bytes match in
all cases. The public release CLI does not expose its internal winning seed,
and some fixtures have identical single-seed output for multiple seeds, so the
checker records every equivalent Go winner. The reproducible check and
per-candidate report are
`scripts/check-seed-selection.py` and `docs/seed-selection-parity.json`.
The historical `sequence-trace-order` map-order difference in diagnostic gap
events remains a separate trace issue; this fix closes only the public
multi-seed output mismatch.

## Cluster gap directions — 2026-09-23

The stable-arena cluster reducer now matches instrumented v0.9.0 on direct
east, west, south, and north operations. Each case compares the moved nearest
node and connected tail, plus the exact final sized edge-length bits. The Go
placement test and Rust unit test are recorded in
`docs/cluster-gap-direction-parity-20260923.json`; both use Go 1.27.0 and the
pinned source commit. Focused Go placement tests pass, and the Rust gap test
group passes 9/9. These are operation fixtures; whole-layout cluster, nested
aggregate, and blocker-fallback parity remain open.

## Nested-layout emitted-trace repeatability — 2026-09-23

Fresh seed-1/2 captures for the reduced retained-cluster witness and upstream
`teleport_grid.d2` match all event families and public bytes. On
`nesting_power.d2`, public output bytes and all event fields except
`ordered_connected` match at seeds 1 and 2; the connected-node sets are
identical, while their order differs in 364 and 385 gap events. A three-run
Go 1.27.0 seed-1 capture produced three unique gap traces with identical
public output, and only `ordered_connected` varied between runs. The pinned
source's `layoutgraph/node.go:connectedNodes` iterates Go `Clusters` and
`Containers` maps, so this is upstream map-order nondeterminism. The strict
comparator keeps the order difference visible. Reports:
`docs/emitted-trace-retained-grid-cluster-current-20260923.json`,
`docs/emitted-trace-teleport-grid-current-20260923.json`,
`docs/emitted-trace-nesting-power-current-20260923.json`, and
`docs/nesting-power-go-repeatability-current-20260923.json`. This corrects
the earlier full-trace claim for `nesting_power`; public geometry still
matches.

## Current alignment qualification — 2026-09-23

The endpoint-alignment consumer now retains `ConnectedNode` identity during
candidate traversal, speculative movement, and accepted movement. The direct
Go/Rust trial comparison matches **1,554/1,554 ordered trials** over four
fixtures at seeds 1 and 2, including exact score and translation bits; full
emitted traces match **8/8 fixture/seed cases**. Reports:
`docs/alignment-trial-parity-20260923.json` and the four
`*-alignment-connected-trace-20260923.json` files.

The final seed-2 nesting mismatch was isolated to `ContainerAlignmentCost`:
Go lazily creates its non-center-port cache from the first candidate that
produces a scored sibling pair, while Rust had sampled before any candidate
move. Rust now preserves that first-use timing. The regression
`container_alignment_cost_is_first_scored_from_candidate_geometry` passes.
Cluster gap reduction now has direct Go/Rust fixtures for active connected
views, fixed-node exclusions, and nested sibling-clearance fallback. The cases
match stable traversal IDs, exact candidate scores, accept/reject decisions,
and final positions. The nested isolated Rust fixture initializes Go's lazy
TurnCost explicitly; omitting this stage cache had produced a misleading
score-only test failure. See
`docs/cluster-gap-operation-parity-20260923.json`. The diagnostic TALA suite
passes 619 tests with 25 historical ignores plus 2 doctests. These are
fixture-scoped results; full v0.9.0 capability parity is not complete.

## Expanded shape and nested-cluster emitted-trace qualification — 2026-09-23

The upstream `all_shapes_link.d2` case matches the pinned Go 1.27.0
source oracle and optimized Rust release at seeds 1 and 2. Public output bytes
and all eight captured event families match exactly: stages, placement,
candidates, scores, packing, routing, RNG, and gap reduction. Gap traces have
1,527 events at seed 1 and 1,600 at seed 2. This run exposed an omitted
Rust-side `gap-connectivity` diagnostic event; Rust now captures Go's ordered
connected views, movement set, nearest-between choice, and exclusion order.
The full report and witness hashes are in
`docs/all-shapes-link-emitted-trace-current-20260923.json`. The optimized
release oracle was rebuilt and contains no gap trace marker strings. This
qualifies this fixture at two seeds and does not close the wider TALA or D2
capability gates.

The distinct `nested-left-cluster.d2` source fixture also matches at seeds 1
and 2, including public bytes and all eight event families. Each trace has 49
stage, 165 placement, 936 candidate, 4 routing, and 114 gap events; score and
RNG event counts vary by seed and match exactly. This extends the matrix by
four fixture/seed cases and exercises nested aggregate geometry alongside
the all-shapes case. See
`docs/nested-left-cluster-emitted-trace-current-20260923.json`.

Four further source fixtures also match at seeds 1 and 2:
`fixed-container.d2`, `sequence-wide-self-label.d2`, `near.d2`, and
`multiple_constant_nears.d2`. Public bytes and all applicable event families
match exactly, extending this trace sweep across fixed positions, sequence
self-labels, near-object placement, and multiple constant-near constraints.
Reports are `docs/fixed-container-emitted-trace-current-20260923.json`,
`docs/sequence-wide-self-label-emitted-trace-current-20260923.json`,
`docs/near-emitted-trace-current-20260923.json`, and
`docs/multiple-constant-nears-emitted-trace-current-20260923.json`.
`right-near.d2` and `nested_shape_labels.d2` also match at both seeds, with
identical public bytes and all applicable trace families. See
`docs/right-near-emitted-trace-current-20260923.json` and
`docs/nested-shape-labels-emitted-trace-current-20260923.json`.
`icon_positions.d2` and `nested-order.d2` likewise pass the two-seed
full-trace comparison with identical public bytes. Reports:
`docs/icon-positions-emitted-trace-current-20260923.json` and
`docs/nested-order-emitted-trace-current-20260923.json`.
`nested-lock.d2` and `grid-routing.d2` also match public bytes and all
applicable trace families at both seeds; see
`docs/nested-lock-emitted-trace-current-20260923.json` and
`docs/grid-routing-emitted-trace-current-20260923.json`.

`sequence-trace-order.d2` remains a strict-trace exception. Both seeds have
identical public bytes and matching non-gap event families. In the gap family,
every difference is confined to `ordered_connected`; all other event fields
match. In one five-capture sample, Go 1.27.0 seed 1 produced two orderings for
the same connected-node event while the public output hash remained identical.
A separate automated five-run audit found five distinct gap streams while
public output and the other seven families stayed identical. The source
ranges over Go maps in `Node.connectedNodes`, so the
strict trace difference is upstream map-iteration nondeterminism. The
comparator retains it without normalization. See
`docs/sequence-trace-order-emitted-trace-current-20260923.json` and
`docs/sequence-trace-order-go-map-order-20260923.json`. The repeatability
script is `scripts/audit-oracle-trace-repeatability.py`, which now records
both Go and Rust traces. The refreshed five-run seed-1 report is in
`docs/sequence-trace-order-go-repeatability-current-20260923.json`.

## Refreshed qualification — 2026-09-22

The current release binaries supersede the historical boundary counts below:
`check-boundaries.py` now reports 114/114 applicable layout fixtures and
88/88 source-position cases, with one explicit pre-measure inapplicable source
and zero failures. Complete emitted traces also match the focused 15-case
matrix at seeds 1–3; see `docs/boundary-baseline-20260922.json` and
`docs/layout-focused-20260922.json`. Older queue, packing, nudge, and lock
failure rows remain as localization history and are not current qualification
results.

## Latest qualification — connected-set trace parity

The clean pinned v0.9.0 oracle comparison now matches the previously failing
small `nested_layout_bug.d2` witness at seeds 1 and 2 (9826 and 9847 events),
while retaining exact `nesting_power.d2` traces at seeds 1, 2, 3, 5 and 42.
The broader qualification is exact for 28 upstream cluster-herd cases, 10
focused cases, 6 inner-gap cases and 4 cluster-flip cases. The fix follows
Go's pointer-visible edge slice for ordinary connected-node traversal and uses
the Tala-ID order for the small graph path that calls Go's legacy traversal;
large connected-set ordering remains unchanged. Temporary probes used to
localize the mismatch were removed before rebuilding the oracle patch and both
Rust binaries. Evidence is recorded in the current `qualification-*.json`
reports and `pre-sync-container-overlap-nesting-seeds5-clean.json`.

Authority: pristine D2 v0.9.0, not the old modified `/tmp` checkouts. The
translation-map records the inherited files and their physical upstream areas.
The internal Rust module aliases are transitional; moving files is not proof
of equivalent source structure or behavior.

The current `nesting_power.d2` all-family qualification is recorded separately
in `docs/nesting-power-emitted-trace-parity.json` and reproduced by
`uv run --no-project python scripts/check-nesting-power-trace.py`. At seeds 1
and 2, the pinned Go and Rust oracles match public bytes plus every captured
stage, placement, candidate, score, packing, routing, RNG, and gap event. The
older single-seed gap rejection note later in this audit is historical evidence
from before the retained-container copy-back fixes and is superseded by that
report. The same checker and event-family matrix qualify `nested_layout_bug.d2`
at seeds 1 and 2; see `docs/nested-layout-bug-emitted-trace-parity.json`.

## Source structure map

`docs/tala-source-structure.json` is generated by
`uv run --no-project python scripts/audit-tala-structure.py --output docs/tala-source-structure.json` from the pinned
checkout. Run it with `--check` in CI to detect an unreviewed source-layout
change. The report currently inventories 20 upstream internal packages, 201
Go implementation files, and 197 Go test files. Rust has 103 internal files
across 20 physical upstream-area homes; `runtime` and `tests` are Rust-only
support areas. The source-shaped `invariant`, `labelgeom`, and `typedpool`
boundaries now have focused Rust implementations. Fixture data is intentionally
kept outside the implementation package map. The renderer's source-shaped
paths are a separate target concern.

Several mapped areas are intentionally collapsed into fewer Rust modules. That
is useful for ownership and does not change execution, but it makes source
comparison and trace navigation less direct. When a parity fix lands in one of
those areas, keep the upstream file and symbol in the source-to-Rust ledger and
in the diagnostic witness. Split a module when its boundaries clarify a real
algorithm or test seam; do not split files solely to make line counts match.

The package directories already match the v0.9.0 Go tree (`engine`,
`layoutgraph`, `hierarchy`, `placement`, `routing`, and so on). Some of those
files are currently included as source-shaped modules under one internal Rust
namespace to keep the borrow-checked graph boundary explicit; the physical
path and the source-to-Rust ledger preserve the upstream mapping. File
organization cannot itself change execution; it only changes how quickly we
can find a divergence. The first source end-to-end mismatch found during the
release corpus run was the CLI's synthetic font fallback, which changed node
dimensions before TALA ran. The native path now embeds D2's pinned Source Sans
Pro Regular face by default. Remaining geometry differences are being
investigated at the graph, layout, and renderer boundaries rather than
attributed to filenames.

## Proven differences requiring translation

- Upstream routing uses `internal/routing/priority_queue.go`: an indexed binary
  heap with insertion-order tie breaking, decrease-key validation, storage reuse
  and pre-mutation work-budget checks. Rust visibility search now uses a
  translated indexed binary heap with insertion-order ties and vertical-first
  source insertion. The queue now has source-shaped reusable 256-entry chunks,
  reset semantics, exact pre-mutation reservation bounds, and classified empty,
  stale-handle, and non-decreasing-key errors. Focused tests cover mixed and
  decreased priorities, invalid-operation nonmutation, chunk boundaries,
  reset reuse, and guarded-operation ordering. The current visibility search
  production search calls the guarded entry points through a thread-local
  per-flavor budget, with edge-inventory, sort, port-pair, and per-edge
  reservations accounted before the heap loop and final usage aggregated at
  the flavor boundary to avoid synchronization in the hot queue path. The
  OVG construction now has a separate aggregate candidate/node/edge/work
  envelope. The checked path charges the Cartesian grid, derived edge nodes,
  tree/boundary/tunnel stages, line visibility checks, center links, and
  near-port traversal with cancellation checks before publishing a completed
  context; the remaining slingshot call-site accounting and exact route-stage
  error classification still need completion. Historical Fibonacci tests remain
  under cfg(test), but that implementation is absent from ordinary builds.
  `queue-boundary-baseline.json` records the result: the same 23/33 layout
  matches and ten first differing fields. This source alignment does not prove
  the earliest divergence in those witnesses or operation-level trace parity.
- Upstream TALA Options distinguish absent/empty seed lists, graph-data
  precedence, a maximum of 64 entries and 16 unique seeds, and configurable
  concurrency. Rust now deduplicates and validates those limits, honors
  `data.tala-seeds` precedence at the JSON adapter boundary, and bounds native
  seed workers. Cancellation and the complete D2 option/configuration surface
  remain pending.
- The pinned Go 1.27.0 toolchain was checked directly: its legacy
  `math/rand` stream and `sort.Slice` PDQsort produce the existing seed and
  permutation witnesses byte-for-byte. The translated RNG preserves draw
  counts and rejection rules; the sort port matches Go 1.27's
  `insertionSort`, `partition`, `partitionEqual`, pattern breaking, and
  ninther thresholds. Broader operation-level stream coverage remains part of
  the parity work; a seed-one sample alone is not sufficient.
- An always-on shuffle-history Vec was inherited despite tracing being disabled
  by default. That storage and mutation now require `diagnostic-traces`; the
  broader instrumentation/allocation audit is still pending.
- Old trace comparison drops selected events and adjusts component coordinates.
  It does not establish the requested detailed event equivalence. The retained
  source patch is only a stage observer and is labeled as such.
- The current route-search diagnostics now cover the Go priority-queue heap
  operations as well as neighbor visitation, candidate cost construction, and
  target pops. A fresh expanded upstream rerun matches all applicable
  `ShortestToLongest` rows at seeds 1, 2, and 3, and the two formerly divergent
  witnesses match `LongestToShortest` and `Default` as well. The older routing
  reports that recorded those rows as divergent are historical; transaction,
  map/sort traversal, and larger nested-placement trace coverage remain open.
- `TopDownLeftRight` had previously been marked insufficient only because the
  two selected witnesses emitted no search edges for that worker. The
  non-empty `all_shapes_link.d2` witness now matches Go's ordered search and
  heap events at seeds 1, 2, and 3 (report
  `docs/routing-search-all-shapes-TopDownLeftRight-20260921.json`). This is a
  worker-specific qualification, not a complete TALA trace gate.
- Optimization sort accounting now uses Go's fixed-width `bits.Len64` rule
  instead of Rust `usize` width, preserving work-limit semantics on wasm32.
  The focused limits tests and wasm32 release check pass.
- A fresh capture of `nesting_power.d2` supersedes the later historical note
  that called seed 1 the remaining full-trace failure. Seeds 1 and 2 now match
  every enabled family (stages, placement, candidates, scores, packing,
  routing, RNG, and gap), including exact float-bit fields and public graph
  bytes; see `docs/nesting-power-emitted-trace-rerun-20260921.json`.
- D2's ordinary non-container labels are bold by default (`Object.Text`), so
  the graph measurement boundary must choose the bold face before TALA sees
  node dimensions. The native source path now bundles D2's pinned regular and
  bold faces and carries effective bold/italic style into measurement. A
  one-letter witness (`a -> b`) now matches the v0.9.0 measured boxes at
  53x66 with 8x21 labels; this was a semantic font/style fix, not a module
  layout fix.
- Container titles use the upstream level size ladder (28, 24, 20, then 16
  px), and ordinary multiline labels advance one em per additional line. The
  source graph lowering and TrueType measurer now preserve both rules; the
  `dagre_spacing.d2` container witness matches its 28/24/16px title and child
  measurements. Remaining container differences involve shape/icon fitting,
  not the source file layout.

## Current strict comparisons

The latest strict rerun records 176/176 passing comparisons (87 successful
layouts plus the source-position cases and one documented pre-measure
inapplicable source) in `boundary-baseline.json`. It compares all JSON fields,
ordered arrays and numeric IEEE-754 values without translation/route
normalization. Earlier failures in this section are retained as historical
localization evidence; they are not the current boundary result.

The historical first-difference set was:

- `benchmark-medium`, seed 2: route coordinate 570 versus 897.
- `features`, seeds 1–3: route coordinate 376 versus 166.
- `nested-lock`, seeds 1–3: route length or position differences.
- `routing`, seeds 1–3: route coordinate 236 versus 235.

Those were first differing serialized fields, not earliest algorithm
divergences. The current full TALA matrix and strict boundary rerun supersede
their pass/fail status while the witnesses remain useful for provenance.

The diagnostic measure operation uses theme 0/default fonts and does not stand
in for full D2 CLI configuration or nested layout orchestration. This is a
measured-graph boundary comparison. Official-release end-to-end qualification
is a separate required gate.

`instrumentation-baseline.json` verifies that the separate Go stage-trace
worktree retains byte-identical public output for all 11 diagrams at seed 1.
This validates that patch on this corpus, not the entire requested trace surface.

## Stage localization after queue alignment

`localize-layout.py` now retains measured inputs, both final outputs and stderr
under `target/parity-witnesses`, and compares every top-level field separately.
`layout-localization.json` shows seven failing cases also disagree in node
positions. Only `routing.d2` seeds 1–3 have identical objects and differing routes.

For `routing.d2`, seed 1, `routing-nudge-localization.json` captures raw selected
stage events with float bits and their original event indices. Route coordinates
match through PlaceLabels: the middle segment is at y=1236 in both. At
NudgeEdgeChannels Rust moves it to y=1235; Go retains y=1236. The final normalized
outputs consequently differ by one pixel (235 versus 236). No offset adjustment
was used in this comparison. Rust has duplicate and differently named stage
emissions; this evidence is localization, not an operation-level parity verdict.

Static source comparison confirms that inherited `routing/nudge.rs` implements
an independent +/-1 wire-shortening heuristic. Upstream `nudge_channels.go`
builds grouped separation constraints, requires finite real-wall bounds on both
sides, solves earliest/latest/midpoint proposals, preserves clearance, and uses
atomic snapshots plus quality guards. Translate that complete postpass and its
upstream tests next; do not suppress the stage or insert a fixture-specific fix.
The remaining seven layout cases still require earlier-stage localization.

## Production channel bridge

The inherited +/-1 heuristic has been replaced with the connected source-shaped
channel pass. `nudge-bridge-boundary.json` records 26/33 exact layout matches:
all three routing.d2 seeds now pass, and the seven other failures remain. The
88 position cases also continue to pass. This establishes final-output equality
for those witnesses, not complete detailed trace, work-limit or error parity.

The bridge uses current node/icon/edge/arrowhead label geometry and translated
inspection metrics. ArenaGraph presently represents routes as independent point
values; upstream pointer aliases, curve flags and exact live inventory ownership
remain structural translation work. Full and second-pass production routing now
use a checked cancellation boundary, poll the active token, and account edge
inventory, deterministic sort work, port-pair setup, per-edge steps, and queue
reservations through thread-local per-flavor work envelopes whose final charges
are aggregated at the flavor boundary. OVG construction now has an independent
aggregate resource envelope, but slingshot call-site accounting and exact
upstream route-stage error propagation still need completion. Optional-stage failure
restores input.
Do not mark these gaps complete based on the improved corpus count.

## Features trace localization

The Rust stage tracer now uses the input D2 ID for ordinary nodes, matching
upstream Node.DebugID, instead of numeric FNV identifiers. Synthetic identities
and operation events still need alignment. `capture-layout-trace.py` now
verifies exact public output bytes against each uninstrumented oracle during
capture, and `compare-layout-trace.py` reports the first event/path mismatch
without sorting or tolerating float differences.

`features-stage-localization.json` retains every event index and unmatched stage.
For seed 1, the first observed state mismatch is that Rust initializes the locked
anchor position in ArenaGraph::from_input, while upstream still has nil TopLeft
through preprocessing. Node snapshots converge at NodePlacement and remain equal
through the immediately pre-BinPack AlignAxes events (Go 17, Rust 18). The first
BinPack snapshots (Go 18, Rust 19) differ: fixed.x is 253 upstream and 0 in Rust.
This localizes the persistent final geometry mismatch to packing, without hiding
the earlier position-lifecycle trace difference. Align that earlier difference
before claiming execution parity; then instrument packing groups, bounds,
candidates and acceptance. The edge-ID mismatch and duplicate/different stage
names remain explicit failures, not normalized away.

The normalized stage stream now matches all **38/38** events for `near.d2` seed
1. Placement snapshots match the first event, then diverge at the next event's
first node position (Rust `0x403e000000000000`, Go
`0x4068c00000000000`); Rust emits 67 placement snapshots while the Go artifact
emits 68. The Go artifact also emits 256 candidate and 2,276 score events while
Rust does not yet emit those operation events. These are explicit remaining
trace gaps, not normalized away. Both instrumented binaries retain identical
public bytes to their ordinary counterparts.

## Locked-position lifecycle correction

Fresh layout Pipeline construction now retains FixedTopLeft without initializing
TopLeft. Prearranged routing keeps its supplied position. This follows upstream
adapter.go and placement/node_placement.go rather than hiding position fields in
the tracer. The regression test checks both layout and routing behavior.

The updated features-stage-localization.json shows matching node snapshots at
all comparable stages from Prescale through Equidistance, and at the immediately
pre-BinPack AlignAxes boundary. The previous early initialization mismatch is
resolved; first-BinPack remains the next node-geometry divergence. Edge IDs and
unmatched stage emissions still prevent a full execution-trace parity verdict.

Both trace builds preserve their ordinary public output. The rebuilt release
oracle comparison in locked-lifecycle-boundary.json retains 26/33 exact layout
matches and 88/88 position matches, with the same seven known layout failures.

## Packing escape bounds correction

Static comparison with v0.9.0 `packing/binpack.go` (packedTLX/packedTLY) found
an inherited edge-dependent branch: Rust used fixed-origin bounds whenever any
edge existed. Upstream always calls `graphbounds.BoundingBox` here. Rust now
uses visible bounds for these escape candidates, retaining fixed bounds for
candidate inventory and scoring where upstream calls `FixedBoundingBox`.

The features seed-1 witness now chooses the left-of-anchor candidate (253,132),
and all comparable node snapshots match through Normalize. Both instrumented
oracles retain byte-identical public output. Stage names/coverage and edge IDs
still differ, so this does not establish full trace parity.
`packing-bounds-boundary.json` records 29/33 strict layout matches and 88/88
source-position matches. Features seeds 1–3 now pass; benchmark-medium seed 2
and nested-lock seeds 1–3 remain failures.

The new internal regression covers a locked component and a descendant edge.
An inherited adapter test expected the old shelf location (0,165); checking its
input with the source oracle (adding required root AbsID and object IDs for Go
deserialization) confirmed the corrected location (20,120), and its expectation
was updated. The near and target positions are unchanged.

A fresh nested-lock seed-1 stage capture locates its first comparable node
mismatch at NodePlacement, before packing: upstream first-node x=396, Rust=495.
See `nested-lock-stage-localization.json`; this is the next algorithm boundary.

## Nested-lock placement localization

The separate source worktree now emits optional `TALA_TRACE_PLACEMENT` phase
snapshots, including descendant boxes and exact floating-point bits. Its patch
is tracked and byte-verified by build-oracle.py. Capture with
`uv run --no-project python scripts/capture-layout-trace.py nested-lock.d2 --placement`.
Both instrumented outputs retain identical public bytes to their ordinary builds.

For seed 1, recursive child placement and fitting produce child (120,110) in both
implementations. Root initialization has container (2,1), outside (3,1), and the
shared child at (182,171). Root coordinates still agree at sized transition:
container (891,0), outside (600,80). The first sized iteration, 64, chooses container
(99,0) upstream versus (99,-99) in Rust. This narrows the next investigation to
candidate generation/scoring and descendant projection state at that iteration.
`nested-lock-placement-localization.json` retains the observed phases, oracle
hashes and transparency checks. No algorithm or strict-output result changed.

## Root fixed-origin constraints in sized placement

Source comparison of `medianPointGuarded` and `optimizerFixedOrigin` found that
Rust skipped the fixed-origin lookup for root nodes. The lookup now includes
root siblings. The upstream optimizer also caches its scope origin and rejects
points below it during generation, occupancy checks and candidate evaluation;
these filters were absent and are now applied. Median computation retains the
live origin lookup, separately from the cached candidate bound.

The instrumented source now records raw and shuffled candidate points using
exact float bits. In nested-lock seed 1, the first root sized iteration (64)
now matches upstream's median (232,0), all nine raw and shuffled candidates,
and selected point (99,0). Previously Rust's median was (232,-245). The next
position divergence is iteration 65: Go (396,297), Rust (792,0). Candidate
scoring and descendant geometry remain to be compared there. Evidence is in
`nested-lock-root-clamp-localization.json`; earlier localization reports retain
historical evidence and are superseded at these boundaries.

`root-clamp-boundary.json` still records 29/33 strict layout matches and 88/88
position matches, without losses among previous passes. Both instrumented
oracles preserve ordinary public bytes. Normal/all-feature workspace tests,
formatting and Clippy pass. The regression checks translated root-origin floors
under repeated random median perturbations, candidate generation and occupancy.

## Fixed-anchor descendant translation

The first raw/shuffled candidates and selected point agreed after the root-origin
fix, but candidate scores already differed. At iteration 65, candidate (792,0)
scored 412.7625 upstream versus 152.42916666666667 in Rust. Rust's external child
was at (375,91), although the correct child offset is (180,170), giving (972,170).
The missing translation (-597,-79) was exactly the earlier fixed-anchor shift.

Static inspection found `anchor_fixed_positions` translated only the dense
arena's node positions, omitting live descendant projections outside the
subgraph. Upstream calls MoveWithChildren for the common shift. Rust now uses
its descendant-aware translation. The pre-sized exact adjustment also uses
MoveAbsWithChildren; the final exact adjustment assigns only TopLeft, preserving
the distinct upstream calls.

The regression explicitly gives a temporary parent an external child and checks
both move with the anchored graph. `anchor-descendant-boundary.json` records
32/33 strict layout matches, including all three nested-lock seeds, and 88/88
source-position matches. The remaining strict layout failure is benchmark-medium
seed 2. Workspace tests in both configurations, Clippy and formatting pass.
The tracked development patch now emits raw edge and total scores with exact
bits; it does not call scoring again or change candidate decisions.

The fresh nested-lock trace has no differences in comparable node snapshots
through Normalize. Its iteration-65 scored-candidate totals match bit-for-bit
in encounter order (`nested-lock-anchor-score-comparison.json`). Both diagnostic
oracles preserve ordinary public bytes. Edge identities and unmatched stage
coverage still prevent a full trace verdict. A fresh medium seed-2 capture is
stored in `medium-stage-localization.json` for the remaining witness.

## Even-seed graph-distance initialization

Static comparison found the v0.9.0 `placeNodesOrthogonally` even-seed path missing
from Rust. `stress_initialize.go` uses all-pairs graph distances, 48 pairwise
relaxation sweeps, then stable degree-ordered placement into unoccupied integer
cells. It falls back for fewer than 4 or more than 64 nodes, over 256 edges,
fixed nodes, or disconnected edge connectivity. The Rust translation now runs
at the same per-scope boundary and leaves the annealing RNG untouched.

The ordinary `initializeNodes` diagnostic entry remains ordinary, as upstream's
function does; selection of the alternative belongs to `placeNodesOrthogonally`.
This preserves the inherited direct-optimizer regression while changing actual
even-seed layout. Tests port upstream deterministic/unique grid placement,
unchanged dimensions, global path spacing, and fallback-without-mutation checks.

Qualification is incomplete: context cancellation and full graph validation are
pending, as are exact intermediate sin/cos/pow bits and relaxation traces. Hypot
uses the existing Go translation; other transcendentals currently use native
Rust. The Go constant .02/.7 is represented as 1/35 to avoid evaluating rounded
operands before division. Matching final integer cells does not prove full
floating-point trace parity.

Three recent ledger entries incorrectly copied the archived Weftan source hash
into upstream_commit. They now explicitly reference pinned D2 d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda;
the ledger distinguishes its inherited source revision from upstream_source_commit.

The final release build passes all 33 measured-graph layout comparisons and all
88 source-position cases (`stress-boundary.json`). Normal/all-feature workspace
tests, formatting and Clippy pass. This closes only the initial 11-fixture,
three-seed corpus; the complete upstream capability/trace gates remain open.

The fresh medium seed-2 trace has matching comparable node snapshots through
Normalize and both diagnostic oracles retain identical ordinary public bytes
(`medium-stage-localization.json`). Edge identity and unmatched stage gaps still
prevent a complete trace verdict.

## Upstream corpus expansion

Five unchanged upstream end-to-end D2 inputs are copied under tests/upstream/d2
with paths, MPL attribution and checksums. The manifest also inventories 129
internal JSON/fuzz fixture files (including goldens), all pending direct-engine
adaptation. They are not silently treated as D2 serialized graphs.

The expanded measured-graph checks pass 14/15 across seeds 1–3. all_shapes_link
seed 2 fails. Its comparable node snapshots match through ReorderDuplicates;
comparing the raw second BinPack events shows the node divergence starts there,
not in the newly added initializer. Both diagnostic builds retain byte-identical
public output to their ordinary binaries. Edge trace IDs remain different, so
this localizes node geometry without proving earlier route parity.

The harness now accepts a fixture directory and seed list, supports layout-only
runs and reports harness timeouts as failures even if both programs time out.
The grid and near cases test the measured adapter boundary only; they do not
establish the missing full CLI/layout orchestration semantics.

## Routed packing rollback boundary

For all_shapes_link seed 2, both node geometry and the ordered edge endpoint/
route-point fields match immediately before the second BinPack. Edge IDs still
differ; the comparison reports those separately rather than rewriting them.

The development Go packing trace establishes identical component inventories,
chosen candidates and final/original scores for tooltipped. Rust then restores
all child positions. Static comparison found the outside-original-box predicate
was computed from the proposed wrap before KeepOriginalBox restored the routed
container. Upstream validates the resulting box after making that decision.
Rust now computes its envelope check from the resulting box, preserving valid
packed child translations when the proposal was discarded. The broader routed
container proof is still approximated and remains a separate qualification task.

Packing diagnostics are restricted to the separate upstream checkout, with a
tracked byte-verified patch. Stage comparison now also reports ordered endpoint
and route fields independently while retaining the complete edge comparison;
this prevents ID mismatches from hiding geometry differences and is not a full
trace-parity normalization.

### Compound proposal trace and source gap

The routed packing fix preserves 33/33 original checks and 88/88 source-position
checks. Expanded coverage remains 14/15: all_shapes_link seed 2 has a one-pixel
public route difference. New opt-in Go/Rust `Compound/*` stage snapshots and
`SelectedResult` show the difference already exists after rigid placement,
before routing. `tooltipped` starts at y=928 in Go and y=929 in Rust, with all
its descendants translated by the same extra pixel. Both instrumented binaries
retain byte-identical public output to their respective ordinary binaries.

Removed Rust's global trace suppression around compound rerouting. All proposals
must remain observable, including rejected proposals; these stage snapshots
still lack the full scoring, RNG and transaction event contract. Unscoped Rust
stage emissions and differing edge IDs remain in the captured data. Matching
stage names/occurrences is only a localization aid and can pair unrelated events
when a stage is emitted inside a proposal; use the explicit `Compound/*` names
for that boundary, not the unscoped duplicate stage matches.

Static comparison against pinned `hierarchy/compound.go` shows that
`apply_compound_flow` in Rust packing is a reconstruction, not a source port:

- Go builds an isolated graph of root-sized proxy nodes, copying interface
  arrows, labels and endpoint/table offsets. Rust currently constructs a root
  adjacency/topological order and does not construct that proxy graph.
- Go calls hierarchy `build`, verifies forward/backward direction counts, then
  calls `Place` with seed zero. Rust substitutes hand-computed cursor positions,
  rounded cross-axis offsets and index-dependent one-pixel adjustments.
- Go runs two interface-score adjacent-swap sweeps, comparing crossings then
  span. Rust lacks this operation and uses additional admission heuristics,
  including an icon-descendant rejection not present in `PlaceCompound`.
- Go translates rigid members and internal routes in a guarded transaction,
  preserving hierarchy assignments and rollback behavior. Rust's current
  outer clone/score transaction is not proof of the same internal operations.

Next: translate the proxy graph, hierarchy placement and interface scoring into
an upstream-shaped hierarchy module, audit the reused hierarchy routines, and
import compound admission/interface/cancellation tests. Do not patch the
observed pixel or retain the index adjustments as an upstream-equivalent rule.
Normal/all-feature workspace tests (583 active), doctests and all-feature Clippy
pass after the diagnostic changes; this does not qualify compound parity.

### Compound interface helper translation

`hierarchy/compound.rs` now translates `compoundInterfaceScore` and
`orderCompoundInterfaces`: boundary-facing segments, strict segment crossings,
ordered span accumulation, two adjacent-swap sweeps, strict score improvement,
rollback on ties, and upstream work charges/final context check. The block
coordinates are isolated proxy data; the original layout must remain untouched
until the future `PlaceCompound` transaction commits.

The copied upstream boundary-offset witness passes in Rust and is also tested
with horizontal axes. The original Go test passes on the pristine checkout with
Go 1.27.0 (`go test -mod=readonly ./d2layouts/d2talalayout/internal/hierarchy
-run ^TestCompoundOrderingUsesBoundaryOffsets$ -count=1`); the checkout remains
clean. Tie rollback and pairwise work-limit tests pass. The
workspace now has 588 active unit/integration tests plus three doctests;
all-feature tests and Clippy pass. This is helper coverage only: production
still uses the reconstructed `apply_compound_flow`, and public parity remains
14/15 for the expanded corpus. Proxy construction, hierarchy ranking/placement,
endpoint/table offsets, admission and transaction integration remain pending.
The helper uses the inherited Go PDQsort implementation; large equal-key
ordering against Go 1.27 `slices.SortFunc` still needs qualification.

### Compound proxy construction

The compound module now builds an isolated arena with one plain node per ordered
root, retains ordered rigid-member lists and original edge identities, copies
external edge arrows/labels and computes descendant endpoint offsets (right-side
SQL row ports where applicable, otherwise centers, clamped to the root box).
Connectivity and detailed-interior admission follow the source preparation.
It does not reproduce the old icon-based rejection or cursor/index offsets.

A graph translated from upstream `compoundTestGraph` checks the six proxy nodes,
six external interfaces, detailed member membership and edge identity order;
mutating the proxy leaves the original positions unchanged. Other tests check
fixed/disconnected rejection and non-table fallback/clamping. All 589 active
workspace tests and three doctests pass with all features; Clippy passes.

This remains preparation, not a published geometry fix. Production still uses
the previous compound implementation. The caller must validate a fully placed
graph, then build/rank/place the proxy, order interfaces and commit a guarded
rigid translation. Revalidate the arena's active-vessel and tree-membership
representations against upstream before claiming those admission cases. The
existing arena has dense edge IDs; the proxy keeps original IDs in a parallel
mapping, which must be consumed during hierarchy/routing integration. The one
pixel witness and expanded 14/15 result remain unresolved.

### Production compound proxy integration

Removed the reconstructed compound function from packing, including icon-based
admission, cursor placement and index-dependent pixel adjustments. Production
now creates the proxy, ranks it, checks forward-edge admission, places its
hierarchy, orders descendant interfaces and copies rigid block translations
through an isolated candidate. Cross-axis detour admission remains outside
`PlaceCompound`, matching the upstream separation. Hierarchy direction handling
transposes horizontal proxy dimensions/label extents and mirrors left/up output.

The translated upstream detailed-interior witness retains offsets in all four
directions; a work-limit failure preserves original positions. The original Go
`TestCompoundPreservesDetailedInteriorsAndOuterDirection` passes under Go 1.27.0.
These checks establish the tested invariants, not complete operation parity.

Remaining gaps include caller cancellation (wrapper still uses Background),
full hierarchy transaction/work charging, source graph validation, candidate
validation/scoring equivalence and stable edge IDs. The reused hierarchy code
must be audited against current source, including RNG/sort and equal-key order.
The original IDs carried beside the dense proxy remain a mapping rather than
proof of upstream graph identity semantics. The complete direct-engine compound
fixture set and seed-race checks remain pending.

Verification after integration: 33/33 original layout checks and 88/88 source
positions pass. Expanded inputs pass 15/15 at seeds 1–3 and another 15/15 at
seeds 4–6. Normal/all-feature workspace tests (588 active plus three doctests),
Clippy, formatting and packaged-notice checks pass. No broader language/render
or direct-engine fixture claim follows from these measured-graph comparisons.

The refreshed all_shapes_link seed 2 capture preserves public bytes for both
instrumented binaries. All explicit compound node snapshots and selected-result
geometry match. However, `Compound/EdgeRouting` through
`Compound/StraightEdgesFallback` have a route difference at ordered edge 27,
linked.parallelogram → linked.document: x bits `0x4076d00000000000` in Go versus
`0x4074f00000000000` in Rust. `Compound/BalanceEdgeSegments` removes it; subsequent
route snapshots agree. Full edge ID comparisons still differ. Thus the public
pixel fix does not establish operation-level parity. Investigate this earlier
routing/port selection witness next, preserving the event order and exact bits.

### Routing flavor witness

Added opt-in `route-flavor` JSON events, emitted in declared reduction order,
plus `capture-layout-trace.py --routing`. Rust's hook and its argument creation
are guarded by `diagnostic-traces`; the upstream hook exists only in the pinned
trace patch. Each event retains edge identity, ordered endpoints, exact distance
bits and raw points. Go records full OVG chains including centers; Rust records
candidate route points. These different representations must not be treated as
identical arrays or silently normalized. Worker/search operation traces remain
pending.

For all_shapes_link seed 2, both emit four flavor events. All ordered endpoint
pairs agree. ShortestToLongest, LongestToShortest and Default ordinary routing
scores agree bit-for-bit. The compound TopDownLeftRight score is
8300.079999999998 in Go and 8225.079999999998 in Rust (difference 75). Its ordered
route 11, linked.parallelogram → linked.document, uses x=365 in Go and x=335 in
Rust; this is stage edge 27 from the earlier witness. The raw proposal is saved
in `all-shapes-routing-flavors.json`. The source node spans x=267..463, so x=365
is its center. Do not infer which cost component is wrong from that alone.

Upstream sortEdges uses stable sorting, consistent with Rust's flavor sorting;
this witness is not a flavor order/selection divergence. Next inspect the
slingshot/search path, port occupancy and center/non-center costs before route
11. Both instrumented outputs remain byte-identical to ordinary outputs.
Workspace all-feature tests, Clippy, default-feature check, formatting and
packaged notices pass. Runtime layout behavior is unchanged by this diagnostic
checkpoint, and the full operation trace goal remains incomplete.

### Shared hierarchy routing search

Static comparison found that upstream's flavor selects edge ordering, not a
different search implementation: `generateRoutes` invokes the same `search` for
TopDownLeftRight and the ordinary flavors. Rust had bypassed its multi-source
OVG search for hierarchy scopes and enumerated synthetic port-pair candidates,
scoring them with a reconstructed route formula. It could choose an off-center
port with a lower score than the actual OVG traversal.

Hierarchy routing now uses the shared multi-source OVG search and endpoint cost
rules. Removed the competing pair-route fallback, including synthetic shared
segments and orthogonal elbows; failure now proceeds to the existing routeLine
fallback. The obsolete scope-mode argument was removed. This is an algorithmic
correction for all hierarchy scopes, not an adjustment to the witness's port or
score. Some unreferenced helper definitions from the reconstruction remain for
a subsequent cleanup; they no longer participate in runtime route selection.

After the shared-search correction, all 63 measured-graph layout checks and
88 source-position cases pass on the final release binary. All four routing
flavor scores in all_shapes_link seed 2 now agree bit-for-bit, as do their
ordered endpoints. Every explicit compound node/route stage snapshot agrees,
including the formerly divergent edge before balancing. Both diagnostic
oracles preserve byte-identical public output. The refreshed flavor/stage
reports retain raw IDs and point representations; identity and full search
operation parity are still not established.

Normal/all-feature workspace tests (588 active plus three doctests), Clippy,
formatting and notice checks pass. Continue with broader upstream direct-engine
fixtures and complete search event/identity coverage; do not equate these 63
layout cases or matching stage snapshots with the full goal.

### Slingshot work-accounting boundary

Static comparison of `routing/slingshot.go` found that the Rust path was
charging only the surrounding edge loop. The v0.9.0 implementation charges the
slingshot entry, each L/S worker entry, endpoint-port scans, flight and anchor
walks, and the S-shaped midpoint ordering before it mutates route state. Rust
now carries the active `LocalRouteSearchWork` into both slingshot workers and
charges those same boundaries. A failed charge leaves the worker guard sticky;
the enclosing flavor reports the aggregate work-limit failure after the worker
returns, matching the coordinator's scheduler-independent error policy.

The change is independent of diagnostics and preserves ordinary geometry. The
`near.d2` seed-1 witness still matches **6,719 normalized events**, and its
diagnostic and release Rust outputs remain byte-identical. The TALA crate has
**596 passing tests and 25 intentionally ignored historical tests**; the full
workspace all-target gate also passes. The subsequent `addRoute` pass now
charges route-node publication, label materialization, point-map insertion,
and static-axis/edge scans. Go's final duplicate check is pointer identity;
the original Rust point-only route representation could not reproduce it.
The 2026-09-23 fixed-route pass now keeps OVG indices from
`VisibilityGraph::add_existing_route` through publication, rejecting repeated
identities while allowing equal coordinates with different identities. The
follow-up identity-carry audit and generated-route wiring are recorded below.
The Rust regression covers both identity cases and the
`route_additional_edges` entrypoint; pinned Go tests
`TestAddRouteRejectsDuplicateOVGNode` and
`TestAddRouteAllowsLoopCenterAtBothEnds` pass at Go 1.27.0.

### Route center occurrences and generated-path identity — 2026-09-23

The follow-up source walk found that Go `addRoute` always retains both center
pointers around a generated route. A port can share a center's coordinates
while remaining a distinct OVG node. Rust had elided those center occurrences
by coordinate, changing `pointToRoute` multiplicity and publication work. It
now keeps both occurrences for generated routes; fixed imported routes keep
the separate consecutive-coordinate interning performed by
`routeAdditionalEdges`. A regression verifies a center-coincident port is
indexed twice, and the Rust TALA suite passes 640 tests plus two doctests, with
25 intentionally ignored cases. The full emitted trace and public bytes match
for `all_shapes_link.d2`, `tala-loop-router-work.d2`, `right-near.d2`, and
`near.d2` at seeds 1 and 2 (8/8); the Go 1.27.0 comparison is retained in
`docs/route-publication-center-requal-20260923.json`.

The ordinary visibility-search and slingshot results now retain their exact
center-inclusive index paths and pass them into the duplicate-pointer check.
Tree S-shape walks return OVG indices directly. Loop, quick-route, and clipped
center-fallback points receive distinct synthetic identities matching Go's
fresh `NewOVGNode` calls; successful `RouteLine` endpoints resolve through the
visibility graph's canonical coordinate index. Every current final-route
producer now supplies a complete center-inclusive identity sequence. The
regression asserts point-to-index mapping and identity-versus-coordinate
duplicate behavior. `all_shapes_link.d2`, `tala-loop-router-work.d2`,
`right-near.d2`, and `near.d2` match the Go 1.27.0 emitted trace and public
bytes at seeds 1 and 2 (8/8); see
`docs/route-publication-center-requal-20260923.json`. Broader adversarial
coverage for coincident owners and route-line branch selection remains open.

The same source pass now charges slingshot `fillPath` loop/adjacency probes,
route-share segment checks, complete launch-leg node/segment scans, and
candidate duplicate-path comparisons. Work guards and routing checks are part
of ordinary runtime behavior; detailed diagnostic trace serialization remains
behind the explicit `diagnostic-traces` feature.

### Direct-engine reference and fixture formats

Added a separate development-only oracle importing pristine `graphjson` and
`engine.Layout`. It reconstructs internal state using upstream's codec, avoiding
loss through the measured-D2 adapter. Its Go module path lies within TALA's
internal-package import subtree; the actual module and binary live under
`.reference/`, without modifying pristine D2. Go 1.27.0, driver/dependency and
binary hashes plus acquisition commands are recorded. It is not a shipped
algorithm or the official release executable.

The oracle matches all 12 exact digests from upstream
`layout_seed_regression_test.go` (four fixtures, seeds 1, 123 and 8675309).
All 129 manifest hashes were checked. Of these files, 115 encode layout graphs,
11 encode OVG/other JSON and 3 are Go fuzz corpus entries. All 115 graph files
round-trip through the upstream codec successfully, including expected outputs.
Their field inventory includes clusters (73), cluster vessels (58), trees (57),
sequences and sequence vessels (20 each), hubs (12), and hierarchies (6).
Field presence includes empty values and does not imply that many active groups.

The first diagnostic reconstruction pass now translates the validated record
inventory into the shared graph model. It preserves signed node and edge IDs,
ordered containers, positions, routes, labels, arrowheads, table columns,
styles, near relations, and auxiliary vessel/tree records. All 44 inventoried
`layout` and `ovg` input records reconstruct and complete a Rust layout call;
the seed-1 comparison is **44/44 exact**, including the large hierarchy
witness. Serialized sequence ownership and vessel geometry now survive the
boundary as well: **28/34** post-layout `graph.exp.json` snapshots match at
seed 1, with six explicit pre-positioned hierarchy, nested, routing, and
self-edge failures. The optional `--include-exp` sweep records these in
`docs/graphjson-postlayout-boundary.json`. Cluster map hydration remains a
diagnostic API but is not enabled in normal reconstruction because it changes
the upstream cluster lifecycle for existing snapshots; cached costs, atomic
publication semantics, and OVG/fuzz adapters remain open.

The final routing postpass now includes TALA's fixed-port interior balancing
pass. Diamond/table-port edges remain locked during the ordinary balancing
pass, then eligible long routes are reconsidered with their endpoint points
held fixed. The Rust arena uses exact coordinate bits for the endpoint identity
that Go represents with shared `geo.Point` pointers, and rolls back the pass if
the resulting route leaves its admissible scope. OVG intersection proximity is
scoped to the active split graph, matching the upstream builder and preserving
valid intersections near nodes belonging to another split scope. This closes
the `self_edges_large` witness in the diagnostic sweep; the 44-fixture sweep
is now **44/44 exact** at seeds 1, 5, 7, 13, and 42. Full operation-level trace,
work-limit, and cancellation parity remain open.

### Typed graph-state wire representation

Translated every struct/field from pinned graphjson/representation.go into
`internal/graphjson/wire.rs`, retaining signed IDs, nullable collections and
pointers, recursive trees, hierarchy maps, cached costs and edge-abduction state.
The public `d2_tala_rs::graphjson` adapter checks this representation, performs
preflight validation, and reconstructs a shared graph before layout. The
`graphjson-wire` example remains a small wire probe; it does not bypass the
atomic reconstruction boundary. Added a separate Go `wire` operation which
decodes and marshals the serialized structs without graph reconstruction.

The initial exact comparison passed 109/115. Six expected-output fixtures had
one-ULP cost differences because workspace serde_json lacked `float_roundtrip`.
Enabling that feature fixes all 115 typed-wire comparisons. A regression checks
six reproduced decimal lexemes against exact reference float bits. Unknown
nested fields, signed minimum IDs, nullable values and retained grouping fields
have focused tests. No input floats are rounded or normalized to pass the check.

This is not a complete Go JSON decoder or engine codec. Arbitrary field casing,
duplicate object merges, all null-in-value positions and exact error text need
translation, followed by source preflight and atomic arena reconstruction.
The direct Go oracle still matches all 12 layout seed goldens after adding its
wire operation; its updated driver/binary hashes are recorded in the manifest.

The final float-roundtrip-enabled release retains 33/33 original layouts,
30/30 expanded layouts (seeds 1–6), and 88/88 source positions. Reports are
`graphjson-float-layout-boundary.json` and `graphjson-float-upstream-layouts.json`.
Normal/all-feature workspace tests now pass 595 active cases plus three
doctests; Clippy, formatting and packaged notices pass. These checks do not
convert the 115 wire matches into direct-engine layout passes.

## Ordered wire decoding checkpoint

The decoder preserves object entry order and follows Go 1.27 encoding/json
updates: folded field lookup, pointer/struct merging, slice element reuse across
shrink/grow operations, empty/null resets, and map value replacement. All 115
inventoried wire fixtures still match. Seventeen of nineteen focused decoder
cases match, including unknown-field rejection; two type-error diagnostics
remain different and are recorded as failures, not normalized away. Six focused
Rust graphjson tests pass. This is wire decoding only; graph preflight, arena
reconstruction, invalid Unicode and comprehensive error parity remain pending.

## Go 1.27 JSON error mapping

Static inspection of `internal/buildcfg/exp.go` confirms JSONv2 is enabled by
default in the pinned toolchain. The v1 API uses `encoding/json/v2_inject.go`
and `v2_decode.go`: errors name the root struct and retain input path spelling
and array indexes. The earlier legacy decoder was not the correct diagnostic
reference. Wire decoding now retains raw numeric tokens, distinguishes Go int
from int64, and reports typed errors with the reference path and spelling.
All 39 targeted cases match, expanding the former 17/19 result. The 115
canonical wire fixtures still match. Syntax and invalid-Unicode handling remain
unqualified, and no direct-engine layout pass is implied.

The Python comparator previously read Go `-0` as an unsigned Python integer
zero. Its output loader now retains negative zero while preserving arbitrary
integer identities. Checks prove `-0` equals `-0.0`, differs from `0`, and does
not round a signed-64-bit identity. This repairs comparison evidence without
normalizing away a floating-point sign. Go BSD attribution is included in
workspace and crate notices.

Validation: 598 normal/all-feature workspace unit/integration tests and three
doctests pass; 20 inherited tests remain ignored. Clippy with warnings denied,
formatting and packaged notice checks pass.

## Serialized topology preflight

Translated `graphjson/preflight.go` with source work-guard calls and bounds on
node/edge records, topology references, route points and tree depth. Repeated
values compare all fields, including nil versus empty vectors and every style
scalar. Tree traversal uses an explicit stack; hierarchy and tree ownership
checks retain upstream error text. The diagnostic `--preflight` path checks
these records without claiming to reconstruct a graph.

The pristine full Go codec and Rust preflight accept all 115 upstream graph
fixtures, and 18 targeted malformed records produce identical rejection text.
Four new Rust tests cover cancellation before validation, record limits and
vessel aggregation, repeated nil/empty values, and tree depth. Go map walks
can select different first errors when multiple entries are invalid; Rust
currently uses signed-ID map order and that traversal parity remains open.

Next, `codec.go` reconstructs top-level records, sorted containers, cluster and
sequence vessels, hubs, abductions, trees, nears, hierarchies and directions,
then atomically publishes. Rust arena nodes still use unsigned TALA identities,
while edges lack a separate source EntityID and store routes as non-nullable
point values. Carrier/ownership state is reconstructed from the public adapter; the adapter must preserve signed
wire identity and distinct active/auxiliary records rather than route fixtures
through the public graph builder and silently lose those fields. No direct
engine output parity case is claimed by this preflight checkpoint.

Validation for this checkpoint: 602 normal/all-feature unit/integration tests
and three doctests pass; 20 inherited tests remain ignored. Clippy with
warnings denied, default compilation, formatting and packaged notices pass.

## Source edge identity and collision allocation

Translated `allocateD2EntityIDs` from pinned `d2talalayout/adapter.go`: preserve
nonzero singleton FNV-1a hashes and allocate sorted ambiguous buckets above
2^32. Upstream collision, high-bit singleton, zero-hash and permutation vectors
pass. Node overrides now receive those IDs; edges carry a signed source ID
separately from dense EdgeId. Hierarchy projections, tree projections and
compound proxies retain that identity, including negative values. Ordinary
public Graph edges retain upstream NewEdge's zero default.

The measured-D2 adapter reconstructs ordinary edge AbsID from actual parent
paths and object ID segments, retaining quoted dotted names and the source
arrow/index qualifiers. Lifeline-specific canonicalization and allocator
cancellation remain pending. General malformed-input diagnostics are not
qualified by this change. Graph identity metadata is skipped by public serde
serialization, so the public graph wire shape remains unchanged.

The all_shapes_link seed-2 witness now has identical ordered edge/from/to
identities and distance bits for all four route flavors. Route point-chain
representation and full search event parity remain separate open gates.
`compare-routing-identities.py` reports only those explicit fields, never a
whole-trace verdict. The existing 63 layouts and 88 source-position comparisons
pass; new source-inspired collision and quoted-name fixtures pass six layouts.
A full pointer-owned aggregate reconstruction is still required; the passing
direct-engine records above are diagnostic qualification only and do not count
as release protocol parity.

Validation: 605 normal/all-feature workspace tests and three doctests pass;
20 inherited tests remain ignored. Clippy with warnings denied and packaged
notice checks pass.

## Signed node identity migration

Pinned `layoutgraph/id.go` defines EntityID as int64 because it participates in
serialization and deterministic ordering. Rust node IDs, temporary vessel IDs,
projected identity maps, transaction lookups, scoring identities and diagnostic
selectors now use i64 end to end. Dense NodeId/EdgeId indexes remain separate.
FNV32 singleton values still widen from u32 without sign extension; Int63 vessel
IDs keep their nonnegative values. Cache hashing writes the same little-endian
identity bits while retaining its unsigned hash state. RNG counters and float
bit keys remain unsigned.

The old u64::MAX and u64::MAX-1 test identities now explicitly express their
original two's-complement values as -1 and -2. Their existing geometry
assertions remain unchanged. A new test exercises i64::MIN/-1/i64::MAX through
container ordering and projected-node lookup. Static inventory found no
negative record IDs in the 115 imported JSON fixtures, so those fixtures alone
cannot qualify signed-ID behavior (`signed-identity-inventory.json`).
This migration removes an arena prerequisite; direct graphjson reconstruction
and complete trace parity remain pending.

Validation: 606 normal/all-feature tests and three doctests pass; 20 inherited
tests remain ignored. Clippy with warnings denied, wasm32 compilation and
packaged notices pass. Differential reports are `signed-identity-*.json`.
The rebuilt optimized oracle retains all 69 exact layout comparisons and 88
source positions. The selected witness retains four matching ordered routing
identity/cost events; full trace parity remains unproven.

## Independent edge visibility state

`graphjson/codec.go` restores Edge.IsInvisible directly; it does not derive it
from SerializedStyle. ArenaEdge now stores that flag, and routing/scoring read
it without reparsing style. The public graph adapter derives the initial value
from opacity-zero or transparent stroke only when no explicit metadata is
present. Hierarchy and tree projections retain the flag. Compound proxies do
not copy it, matching the fresh Connect edges in hierarchy/compound.go.

The new regression checks explicit false with transparent styling, explicit
true with ordinary styling, and subsequent style changes. Existing visibility
routing tests retain their expectations. This preserves one reconstruction
field; nullable route points, complete labels/metadata, auxiliary ownership and
atomic graph publication remain pending.

Validation: 607 normal/all-feature tests and three doctests pass; 20 inherited
tests remain ignored. Clippy and packaged notices pass. The optimized binary
retains 69 existing layouts and 88 source positions; the new opacity/stroke
fixture adds three exact layouts. These are output comparisons, not a claim
of full operation-level trace or direct graphjson engine parity.

## Route-derived placement cost reset

Static comparison with `engine/pipeline.go:resetFullLayoutRouteState` and
`engine/compound_candidate.go:reroutePlaced` found that the Rust compound
reset cleared route points and automatic label positions but retained the
edge-length cache and crossing, turn and non-center-port costs. It now clears
those four stores as upstream `layoutgraph.ResetPlacementCosts` does. The
regression uses the state values from upstream
`TestRunAllStagesResetsRouteDerivedStateBeforeFirstStage` and verifies that the
cache retains its identity and remains usable after reset.

Validation: 608 normal/all-feature workspace tests and three doctests pass;
20 inherited tests remain ignored. Formatting and Clippy pass. The four
`route-reset-*.json` reports retain all 72 exact measured-graph layouts and
88 source-position comparisons. No new direct-engine or detailed-trace
qualification is claimed. Complete fixed-label state and curve-state
reconstruction remain outstanding.

## Independent edge-label fixed state

`layoutgraph/label.go` keeps `positionFixed` independently from the position
enum. The arena now represents this state, imports explicit internal metadata,
and carries it through hierarchy projections. `resetFullLayoutRouteState`
preserves fixed label position/percentage; automatic node/edge placement uses
fixed edge labels as obstacles and excludes them from movable candidates.
Quality scoring still seeds obstacles with loop labels only, matching
`quality/labels.go`, rather than double-counting fixed labels.

The standalone `PlaceNewEdgeLabels` translation now gathers arrowhead and
retained non-loop labels in logical edge order, matching
`labeling/placement.go`. Unlike ordinary placement, that upstream path excludes
loop main labels. Two regressions cover explicitly fixed unlocked positions,
route reset, obstacle inclusion and both automatic placement entry points.

Important remaining lifecycle gap: `layoutgraph/clone.go:copyLabelRecord` and
`copyIconRecord` clear the private fixed flag. Rust arena cloning currently
preserves it, including the pre-existing node flag. Complete source Clone
semantics and full pointer-owned graphjson reconstruction are still pending;
these tests do not qualify clone or direct-engine behavior. Serialized graphjson
labels have no fixed flag.

Normal and all-feature workspace suites pass 610 tests and three doctests;
20 tests remain ignored. Formatting, Clippy and packaged notices pass.
The four `fixed-label-*.json` reports retain 72 exact measured-graph layouts
and 88 source-position comparisons. They do not exercise direct fixed-label
metadata or establish detailed trace parity.

## Compound candidate isolation

`layoutgraph/clone.go` allocates a fresh edge-length cache while copying the
three routing cost scalars. The compound candidate previously retained the
incumbent's shared cache through the Rust derived clone, so resetting candidate
costs also cleared the saved incumbent's cache. Candidate preparation now
allocates independent cache storage and clears node/edge fixed-label flags,
matching `copyNodeRecord`/`copyLabelRecord` at this boundary. Rejected and
inapplicable compound proposals restore the complete saved incumbent.

The isolation regression checks cache identity, candidate reset, preserved
incumbent entries and independent fixed-label flags. Normal/all-feature suites
pass 611 tests plus three doctests; 20 tests remain ignored. Formatting and
Clippy pass. This addresses only these represented fields at the compound
boundary. It is not a complete `layoutgraph.Clone` implementation: icon fixed
state, derived node state, auxiliary ownership, bounded validation and
cancellation still require translation and direct oracle qualification.
The four `compound-clone-*.json` reports retain all 72 measured-graph layouts
and 88 source-position comparisons, with exact numeric bits. This corpus does
not qualify the full clone or operation-level trace contract.

## Icon position and fixed state

`layoutgraph/icon.go` stores position and a private fixed flag independently.
Rust previously treated every `Some(icon_position)` as fixed, including an
automatically selected position on a later pass. The arena now has an explicit
icon flag. Adapter initialization follows `labeling.Initialize`: only a present
non-Unset position is fixed. Automatic placement consults the flag, and compound
candidate preparation clears it as upstream `copyIconRecord` does.

The new unit regression checks that an explicit fixed position is retained,
then that the same position without the fixed flag is reconsidered identically
to an unpositioned icon. The candidate isolation test also checks icon flags.
The new locally authored nested-icon diagram embeds its SVG asset and combines
automatic/fixed positions and cross-container edges. Its three seeds match
exactly (`icon-fixed-regression.json`). Full Clone, graphjson reconstruction,
rendering and operation-level trace qualification remain pending.

Normal/all-feature suites pass 612 tests plus three doctests; 20 tests remain
ignored. Formatting and Clippy pass.
The five `icon-fixed-*.json` reports retain 72 earlier layouts and add the three
icon layouts, for 75 exact layouts plus 88 source-position comparisons.

## Expanded upstream label and icon corpus

Seven additional unmodified `e2etests/testdata/files` inputs are now copied and
hashed in `tests/upstream/manifest.json`: `icon_positions`, `label_positions`,
`nested_shape_labels`, `icons_on_top`, `outside_bottom_labels`,
`overlapping_child_label`, and `multiple_person_label`. All 12 imported inputs
match at seeds 1–3: 12 comparisons in `expanded-label-icon-baseline.json` and
24 in `expanded-label-icon-seeds.json`. The seven new inputs add 21 distinct
comparisons, bringing cumulative measured-layout coverage to 96.

The harness now verifies the fixture manifest's source commit, each local
checksum and each unmodified upstream file before execution. Manual negative
checks with disposable copies confirmed rejection of modified bytes and a
wrong source commit; all 12 authoritative copies validate. No Rust algorithm
changed in this corpus expansion. These are measured-graph comparisons, not
native parser, grid orchestration, renderer or full internal trace passes.

## Expanded connected routing corpus

Six further unchanged upstream inputs are now pinned: `cross_arrowhead`,
`arrowhead_scaling`, `arrowhead_font_color`, `centered_horizontal_connections`,
`disconnect_direction_right`, and `nested_layout_bug`. These add cross
arrowheads, stroke-width variants, endpoint labels, disconnected nested blocks
and descendant connections. All 18 imported files were rechecked at seeds
1–3 in `expanded-routing-qualified.json`: 54 successful matching layouts, zero
errors or mismatches. This adds 18 distinct comparisons to the preceding 96.

The harness now records each engine's outcome and counts successful layouts
separately from matching error responses. Matched diagnostics remain passing
error comparisons and their text is retained. No Rust algorithm changed;
the broader full-D2 and detailed-trace qualification gaps remain open.

## Placement trace boundary checkpoint

The placement comparator now preserves event order, stable external identities,
and exact IEEE-754 values for every placement snapshot. Rust carries the owning
scope identity through recursive placement, emits the upstream `after-fit`
boundary, and projects completed child pointers into root snapshots without
changing ordinary layout state. This corrected the earlier false mismatch from
an extra Rust snapshot and the missing nested root identity.

On the pinned Go 1.27/v0.9.0 oracle, `near.d2` seed 1 now matches all **68/68**
placement events and **38/38** stage events, plus **256/256** candidate and
**2,276/2,276** score events. `nested-lock.d2` matches **118/118** placement,
**128/128** candidate, and **779/779** score events, as well as its **38/38**
stage stream. The Rust sized compaction path now rejects moves past the owning
scope's fixed origin at the same point as upstream. These remain focused
diagnostic witnesses; RNG, transaction, rollback, routing, and full corpus
qualification still remain in scope.

The expanded normalized JSONL checkpoint now also matches the complete event
stream for `near.d2` seeds 1, 5, and 42 (**6,711**, **6,700**, and **6,590**
events), `nested-lock.d2` seed 1 (**3,079**), `right-near.d2` seed 1
(**6,711**), and `routing.d2` seed 1 (**276**). The fixes were algorithmic:
the sized candidate lower-bound prune is applied for every direction, fixed
split-subgraph discovery processes roots in Go's sequential order, and the
diagnostic graph view projects temporary descendant-edge reconnects and
restores. These witnesses use the instrumented pinned v0.9.0 source checkout;
the trace hooks remain diagnostic-only and are compiled out of ordinary builds.

## Source-shaped module layout

The Rust TALA implementation deliberately keeps the v0.9.0 Go package shape as
its primary source map:

| Go package | Rust module |
| --- | --- |
| `engine` | `internal/engine` |
| `layoutgraph` | `internal/layoutgraph` |
| `hierarchy` | `internal/hierarchy` |
| `placement` | `internal/placement` |
| `placementcost` | `internal/placementcost` |
| `routing` | `internal/routing` |
| `packing` | `internal/packing` |
| `labeling` | `internal/labeling` |
| `graphbounds`, `grouping`, `quality`, `trees` | matching Rust modules |

The smaller v0.9.0 support packages are represented deliberately: `nodeshape`
now owns the source-shaped port policy shared by `engine/container_shapes` and
routing, while `labelgeom` is shared by `labeling` and `routing/route_geometry`,
`proximity` is shared by `grouping` and placement scope code, `loops` has its
own module, and `invariant` is part of engine validation. The Go `typedpool`
helper is represented by Rust-owned collections and the runtime adapters. These
choices preserve the source boundary that affects behavior while avoiding
module indirection that would obscure ownership.

This mapping makes a Go symbol, its Rust translation, its tests, and its
remaining differences easy to audit together. It is a debugging aid and a
translation boundary; directory names alone do not create runtime parity.
Ownership, pointer aliases, nil-versus-empty state, identity, traversal order,
floating-point operations, and endpoint direction still have to be translated
explicitly. The recent `routing_through_center` fix is an example: the first
divergence came from source-arrow endpoint semantics and the external parent
identity lost while compacting an induced graph, not from file placement.

Keep this package-level shape as TALA grows. Split a Rust file when ownership
or testability requires it, but preserve the upstream package name in the path
and record the symbol mapping in the source ledger. Do not copy Go files or
introduce one-file-per-symbol structure merely for visual similarity.

## Primitive RNG trace checkpoint

The diagnostic builds now record each Go `math/rand` primitive draw with its
stream name, operation, decimal result, and draw count. The Rust adapter uses
the same source stream and Go 1.27 integer rules; the instrumented v0.9.0
oracle records the corresponding source calls. A snapshot-only Rust lookahead
now reads from a silent clone, so it cannot appear as an algorithmic draw.

The normalized RNG streams match exactly for `near.d2`, `nested-lock.d2`,
`right-near.d2`, and `routing.d2` at seed 1, and for `near.d2` at seeds 5 and
42. This is a focused primitive-stream checkpoint, not a claim that the full
operation-level trace or all v0.9.0 functionality has been qualified.

## Current source-shaped parity checkpoint

After the routed packing bounds correction, the regenerated strict boundary
contains **55/55** exact measured-layout results across the 11 retained D2
fixtures and seeds 1, 2, 3, 5, and 42, plus **88/88** source-position results. The current
`docs/layout-localization.json` records no differing top-level field, including
ordered nodes, edges, routes, errors, and numeric values.

The same rebuild of the optimized diagnostic profile produces an event-for-event
match for `features.d2` seed 1: **3,185/3,185** events across stages, placement,
candidate ordering, scores, packing, routing flavor, and primitive RNG draws.
The instrumented binaries still produce byte-identical public output to their
ordinary counterparts. These are stronger current witnesses than the earlier
intermediate snapshots in this document; the full v0.9.0 corpus and API surface
remain unfinished.

The initial three-seed subset matches all 11 retained measured fixtures
(**33/33** fixture/seed pairs), including
`benchmark-medium.d2` at **792,050/792,050**, **823,858/823,858**, and
**811,316/811,316** events. Every capture matches its complete stage,
placement, candidate, score, packing, routing-flavor, and RNG streams.
Release-mode Rust output remains byte-identical to the pinned Go 1.27/v0.9.0
oracle on every fixture and seed. This is a broader diagnostic checkpoint;
additional seeds and full D2 API qualification remain unfinished.

The shape-port policy now has a physical `internal/nodeshape` Rust home, with
routing re-exporting the same `Port` types for existing callers. This makes the
Go-to-Rust symbol path easier to inspect without adding a second implementation;
the generated structure report now shows 17 physical upstream-area homes and
four intentionally logical support areas. The public `c4-person` spelling is
accepted at the language boundary, with the historical `c4_person` form kept as
an alias.

The reproducible corpus runner now extends the detailed comparison to five
seeds. All 11 retained measured fixtures at seeds 1, 2, 3, 5, and 42 match the
instrumented v0.9.0 source **55/55** times, including complete candidate,
score, packing, routing-flavor, and primitive-RNG event streams. Release-mode
Rust output remains byte-identical to the uninstrumented Go oracle on every
entry. `docs/layout-multiseed-parity.json` records the matrix, binary hashes,
witness locations, and event counts; `scripts/check-layout-parity.py`
regenerates it. This does not qualify the unported D2 subsystems or imply that
the current Rust workspace is feature-complete.

The source-shaped decision was also applied to the direct graphjson witnesses:
keep the upstream package paths, while representing Go's shared-pointer
placement scopes with explicit Rust arena side tables. In particular, the
`after-fit` diagnostic now projects a crossing descendant edge onto its fitted
root using the retained source ancestry, matching the Go graph's live endpoint
view without mutating release edges. The remaining direct seed-123 witness
differences are placement geometry in the sized pass; changing directory names
would not address them.

## Latest v0.9.0 TALA trace sweep

The recovered upstream fixture set now has **18/18** event-for-event matches at
seed 1 against the instrumented D2 v0.9.0 source built with Go 1.27.0. The
comparison includes stage snapshots, placement state, candidate order, exact
floating-point score bits, packing, route-flavor order, and primitive RNG
draws. The release Rust binary is byte-identical to the uninstrumented Go
oracle on all 18 fixtures. The final fixes covered root container traversal
order, singleton `style.multiple` placement, fused baseline arithmetic, and
loop route append order. This is a qualified TALA corpus result; the broader
D2 language, rendering/export, and application surfaces remain separate work.

## Routing search arithmetic — 2026-09-20

The pinned `routing/ovg_edge_router.go:ovgEdgeRouter.search` turn-penalty
addition maps to `routing/visibility.rs:VisibilityGraph::route_between_any_group`.
The Go 1.27.0 ARM64 source oracle emits `FMADDD` for
`nextDistance += turnCost * multiplier`; Rust must use `mul_add` at that
boundary. The upstream `label_positions.d2` witness first differed at search
event 142: candidate bits `0x40792f0395810625` versus
`0x40792f0395810624`. Separate multiplication/addition rounded too early.
`docs/routing-search-fma-first-divergence.json` retains the raw event and
oracle provenance plus disassembly evidence. The fixed fixture and four
additional upstream fixtures pass at seeds 1/2 in
`docs/routing-search-upstream-fma-fix.json`, including public graph checks.
This qualifies the pinned ARM64 arithmetic, not all upstream platform builds.

## Visibility fill-point scope — 2026-09-20

Go `routing/ovg.go:OVG.addEdgesNodes` iterates the active split graph's
`g.Edges`. Rust `routing/edge_nodes.rs:edge_nodes` previously scanned the owning
arena's whole edge order and accepted an edge whenever both endpoints appeared
in the obstacle boxes. Nearby nodes also populate those boxes, so this leaked
another subgraph's perimeter/midpoint vertices into the current OVG.

The generator now receives the active node scope and excludes edges outside
it while retaining nearby boxes for obstruction. A regression keeps the same
obstacle/port inventory while adding/removing an external edge and verifies
that active-scope fill points are unchanged. In `multiple_constant_nears.d2`,
the leaking point at (1213,1093) changed the first leftward neighbor from
(1153,1093). The corrected search matches 414 events at both seeds 1 and 2;
public graphs remain exact (`docs/routing-search-near-scope-fix.json`).

## Port ownership and connector arithmetic — 2026-09-20

In `ovg_edge_router.go:search`, `portMetadataFor(gSource/gTarget)` guards
directional filtering using actual OVG ownership. Rust candidate inventories
can include other members of the endpoint's cluster/sequence. They must not
be used alone to decide whether a point is an endpoint-owned port. The Rust
search now checks `port_owners` before applying these direction predicates.
This restores the missing high-cost candidate in `outside_bottom_labels`.

The former event-1184 one-bit connector mismatch was traced to the source
constant used for non-center ports. Go v0.9.0 defines `centerPortMultiplier` as
the literal `0.04287499999999999`; evaluating `pow(0.35, 3)` in Rust produces a
different last bit. Rust now keeps the source literal in both routing-cost
paths. The regenerated
`docs/routing-search-connector-cost-fix.json` matches all 4,671 search events
at seeds 1 and 2 with identical public output.

### Connector accumulator translation (2026-09-20)

Static comparison against `ovg_edge_router.go:search` (source/target center
connector blocks) found reordered floating-point additions and an additive
prohibited-sharing base in Rust. Candidate construction now begins with 1 or
10,000,000, then adds shared-cluster, facing, and center-port preferences in
upstream order. The visibility search accepts complete connector costs rather
than reconstructing them from aggregated penalties. Existing penalty-based
internal convenience APIs adapt at their boundary. The direct tunnel fallback
also passes complete costs into the search, avoiding subtraction/re-addition
of the base to recover a penalty. The focused source-order and literal-cost
qualification is recorded in STATUS.

### Shape-local mirrored-port indices (2026-09-20)

The next `outside_bottom_labels` discrepancy (event 1236) was a missing
mirrored-center waiver: Rust had recorded the occupied point (1151,1146),
whose physical person-shape mirror is (1023,1146), but used a concatenated
cluster inventory for the index mapping. Upstream `centerSymmetricalPorts`
uses `gSource.MirroredPorts()` / `gTarget.MirroredPorts()`; `Node.mirroredPorts`
indexes the physical shape's `ports()`. Rust now uses each endpoint's saved
physical port inventory for that mapping, while retaining combined inventories
for route candidates. This directly addresses the source-order representation
difference; qualification follows in STATUS.

### Opposing-route source connector (2026-09-20)

The event-4110 diagnostic identifies `opposing_share`, with accepted edge
2217515438 beginning (1420,1160) → (1420,1210). Upstream
`Route.isOpposingColinear` scans OVGNodes from index zero through len-3,
including the source center connector but excluding the target connector.
Rust stored port-to-port routes and omitted that first interval. With the
source center (1420,1080), upstream first sees endpoint-only contact at
(1420,1160) and returns false before the later opposing interval. Rust now
chains the retained source center into this predicate without allocating,
omits a target-center connector if already present, and checks the accepted
edge's directedness as upstream does. A regression covers endpoint-only
contact versus strict interior overlap. Optional search-filter diagnostics
now expose overlap identities and the individual prohibition predicates,
with all construction guarded by the diagnostic feature.

### Nil external transaction containers (2026-09-20)

Expanding to upstream `nesting_power.d2` exposed an expect panic in
`reposition_external_containers`. Pristine `layoutgraph/transaction.go`
`repositionContainers` filters nil TopLeft before refitting, independent of
node count. Rust skipped missing positions only in small projected scopes;
children can acquire positions during synchronization after the earlier
container pass. The refit now skips every still-unpositioned container.
The original panic/provenance is saved in
`docs/nesting-power-transaction-panic.json`. Historical projection heuristics
elsewhere in this function remain unqualified; removing this panic alone
does not establish transaction parity for the fixture.

### External hierarchy source/sink edges (2026-09-20)

`nesting_power` first diverges at PreprocessHierarchies on `r.is`. A reduced
fixture (`tests/d2/hierarchy-external-source.d2`, derived from the upstream
MPL-2.0 fixture's right-near container and external edge) preserves the failure.
Diagnostic raw-stage records confirm the raw arena already assigns hierarchy 0
and position (10,0); the trace projection is not its cause. Upstream
`hierarchy/placement.go:isSource/isSink` inspect the full structural edge slice;
Rust inspected only induced scope edges. Source/sink qualification now also
checks edges crossing the scope boundary, preserving those edges outside the
ranking DAG. The source/sink counters use upstream's if/else-if distinction.
`WEFTAN_TRACE_RAW_STAGE` is diagnostic-feature gated and emits raw membership,
position, scope and active status separately from normalized events.

### Explicit direction in induced placement scoring (2026-09-20)

Placement/RNG capture of `nesting_power` locates the first active-scope
geometry divergence at `directions.l` initialization: Go positions the chain
leftward, Rust rightward. `tests/d2/nested-left-cluster.d2` reduces it to a
left-directed container and two external edges. Upstream InitializeNodes
uses NodeEdgeLength with direction penalties, consulting shared Graph.Directions.
Rust copies explicit directions by stable TALA ID into the induced scope's
scoring map but looked up the desired orientation in the separate local map,
falling back to Right. `edge_length_direction` now consults the retained explicit
scoring map first. The original witness is saved in
`docs/nested-left-direction-first-divergence.json`.

A separate earlier diagnostic-only difference remains: the `after-fit`
snapshot for `r.constant` moves the unrelated directions.l cluster from
index 55 to 143. Its geometry matches by identity; the owner-grouping projection
uses dotted string prefixes on synthetic cluster names. This remains a failing
trace-order gate; do not sort it away in the comparator or claim full trace
parity from the direction fix.

### Stable container identity in sized scoring (2026-09-20)

The sized direction selector now prefers the retained explicit scoring map,
matching the sizeless fix. A regression verifies identical sized cost with
a left-directed parent absent from the induced graph. This correctness fix
does not itself resolve the reduced fixture's score difference.

Per-edge diagnostics show Go and Rust both accumulate exactly
0x40a43e6b0361f3d3 before flow continuity. Rust then adds 199.21142107780074.
Upstream `flow_continuity.go` requires the replacement adjacent's Container
to equal the receiver's Container. Rust used the local induced graph's None
for the receiver, conflating it with detached cluster members' genuine nil
Container. Flow eligibility now preserves the receiver/ordinary neighbor's
stable scoring parent while retaining explicit None for replacement members.
The original per-edge evidence is in `docs/nested-left-flow-first-divergence.json`.

### Structural parent identities across induced arenas

The next `nested-left-cluster` mismatch occurs at placement snapshot 80,
`before-direct`: both engines select Column, but Rust rejects the flip.
`WEFTAN_TRACE_TRANSACTION_INVALID` identified `group.1` as its own container:
a retained source `NodeId(0)` (the group) was resolved in the induced arena,
where index zero is `group.1`. Go's `layoutgraph/graph.go` validates retained
node pointers, not a new index space.

`ProjectedTransactionNode` now carries structural parent and ancestor stable
IDs captured in the source arena. These remain separate from aggregate
scoring identities, preserving real member containers. Containment, ancestor
exclusions and fixed-origin grouping all use those structural IDs. The new
`projected_transaction_preserves_parent_across_arena_reindexing` regression
exercises an induced arena whose index zero changes identity. The diagnostic
rejection dump and all argument construction are explicitly feature-gated.

After preserving structural identities, both reduced-fixture public graphs
match and the event counts match. The remaining pre-direct cell-size mismatch
comes from rebuilding the induced graph after the flip: Rust recalculated
152, but upstream computes CellSize once before placement and retains 128.
The direct graph now receives the actual optimized component's retained cell
size rather than recalculating from resized cluster/tree dimensions.

### After-fit traversal audit

The pinned diagnostic `placement/node_placement.go:tracePlacement` traverses
`Graph.Nodes` in order and recursively visits `g.Containers[n]`, deduplicating
by pointer. `grouping/clusters.go:AddCluster` appends the vessel to the owning
container before removing its members. It does not rank display names.

Rust's placement serializer reconstructed source traversal, then applied two
additional owner sorts using `record.id.split('.')`. Synthetic cluster DebugIDs
were ranked as unknown owners and moved to the end of the entire graph. Those
final display-name sorts have been removed. The comparator continues to retain
all event/array order and float bits; this is an emitter correction, not a
normalization rule. The existing source-traversal projection still requires
qualification against more complex restored-tree/aggregate combinations.

`tests/d2/nested-direction-grid.d2` extracts the `directions` block from pinned
D2 `e2etests/testdata/files/nesting_power.d2` (MPL-2.0) and replaces its external
connections with two ordinary targets. It retains nested grids and four
explicit directions. It reproduces the aggregate trace alias defect but its
public graph is exact; it does not reproduce the full fixture's later
`directions` cell-size/score difference and must not be cited as proving that
algorithm issue fixed.

### Nil external containers in large graphs

The full nesting fixture's first per-identity geometry mismatch is placement
1991, after fitting `directions.l`: height 272 in Go versus 186 in Rust. The
cluster flip fails only in Rust because external container `r` is treated as
an obstacle at (-60,-60). Diagnostic-only rejection details identify the
carrier and its children without changing ordinary builds.

`reposition_external_containers` still had a separate `>64`-node rule that
materialized nil parents at (0,0) before the previously corrected nil-refit
skip. Upstream `layoutgraph/transaction.go` skips nil container TopLeft in
both refit and validation. Removed the materialization rule. Regression
`external_nil_container_stays_nil_with_large_projected_carrier` checks both
small and 65-node carriers with positioned child projections. Evidence:
`docs/nesting-power-cluster-external-rejection.json`.

### Sequence traversal qualification

`tests/d2/sequence-trace-order.d2` extracts `more.container` from pinned
`e2etests/testdata/files/nesting_power.d2` (MPL-2.0), retaining nested sequences
and restoring its incoming/outgoing external connections with ordinary nodes.
Its seed-1 trace matches 178,970 events. It does not reproduce the full graph's
sequence ordering defect and is a regression case, not evidence that defect
is fixed. A feature-gated container-order probe records source, cached,
transaction and trace child identities at one selected root/phase.

The full sequence probe (`docs/sequence-child-order-probe.txt`) has no live
transaction child slice. Source/cached children are scorer, itemResponse,
item, essayRubric, concept; the fallback trace copy has item first. Static
source-order fallback is insufficient: trying it regressed earlier placement
49 (`l.here.grid.a` vs `l.here.grid.with`) and was reverted. Tree preprocessing
refreshes `trace_container_children` from remove/restore order in
`trees/trees.rs`, while later completed scopes refresh geometry through
`TRACE_SYNCED_NODES` without an equivalent child-slice lifecycle. Investigate
copyback of the completed scope's live Containers order; do not sort snapshots
or force declaration order globally.

### Placement worklist is not the owning container slice

Static inspection found an incorrect diagnostic copyback in
`hierarchy/hierarchy.rs`: it replaced `trace_container_children[parent]` with
`placement.child_order`. Upstream `placement/node_placement.go:PlaceChildrenOrder`
allocates a separate `ordered` slice for recursion. It never overwrites its
input or `g.Containers[root]`. The owner synchronizes the actual container slice
after reconnecting trees instead. Diagnostic copyback now snapshots the live
owner's `containers[Some(container)]`, including an empty slice, rather than
publishing the recursive worklist. This change is compile-time feature-gated;
ordinary placement and comparator rules are unchanged.

The owner-slice correction moves the full seed-1 placement prefix from 730 to
1,908 snapshots without changing any score/candidate/RNG event or final graph.
The next mismatch is `directions.r` after-fit: Go 1,2,3,4; Rust 1,3,4,2. The
owner's tree-copyback loop appends missing auxiliaries to retained children,
whereas upstream explicitly synchronizes the owning `Containers[root]` from
its restored subgraph slice. Inspect the completed scope list and its identity
mapping before changing runtime traversal. Preserve genuine restoration order
instead of forcing source declaration order.

### Tree publication preserves container order

The `directions.r` copyback probe shows the owner initially contains only node
1, while the completed scope contains 1,2,3,4. Copyback adds the auxiliaries,
but `publish_placed_tree_edges` then moved container children to 1,3,4,2 using
the descendant-first Graph.Nodes publication sequence. Upstream's publication
loop calls AddNodeUnchecked, not AddNodeToContainer: reconnectTree's root-first
container order remains separate. Removed the helper's container mutation and
extended its regression to assert distinct Graph.Nodes and Containers orders.
All 29 tree tests pass (one previously ignored). Probe evidence is retained in
`docs/tree-container-copyback-probe.json`.

The runtime publication change preserves the full seed-1 graph and every
score/candidate/RNG event. The serializer additionally sorted child slices by
Graph.Nodes after-fit order in its direct and projected traversal paths,
reintroducing the same conflation. Removed those post-fit node-vector sorts;
tracePlacement must recurse through the container slice independently. The
aggregate-vessel representation logic and comparator remain unchanged.

### Retained child order versus declaration order

The `l.here.grid` probe at `directions.r` after-fit shows its trace copy already
has the live order (a, sequence diagram, you can, with), while the immutable
source/cached list starts with with. `append_projected_trace_node` reordered
the available children by that immutable source list before recursing. Removed
this override; it neither restored missing aliases nor changed membership.
The traversal now consumes the retained slice directly, as upstream's trace
hook consumes `g.Containers[n]`. Identity evidence is recorded in
`docs/retained-grid-order-probe.json`.

After this correction, full seed-1 placement first differs at 2186:
`directions.v` initialized. The `directions.v.d` completed scope and owner both
have 2,4,1,3 (`docs/down-tree-copyback-probe.json`), but its projected trace has
1,2,3,4. Static inspection of `hierarchy_scope.rs` finds a direction-dependent
sort of external-container children by `source_container_child_order` while
building the next scope. That resets a live restored slice to declaration
order for explicitly directed containers. Audit/remove that source-order
substitution against upstream CopyEntitiesFrom semantics next; unlike the
serializer fix, this affects runtime projections and needs full differential
verification.

### Explicit directions preserve the live shared child slice

Pinned `layoutgraph/graph.go:1074` (`CopyEntitiesFrom`) assigns Containers
straight from the owner. There is no declaration-order reconstruction for
explicit direction keys. Removed the direction-dependent source-order sort
from `placement_scope_graph_after`'s external child projection. Retained its
missing-inventory fallback and tree-descendant restoration.

`placement_scope_preserves_live_directed_container_child_order` checks that a
parent scope sees an explicitly directed nested container's reordered live
slice, independently of declaration order. All six placement-scope tests and
37 projected tests pass (three existing ignores). Diagnostic and release
oracle builds pass; differential results are recorded separately below.

Differential verification: full nesting seeds 1 and 2 now first differ at
placement 2285 (`directions` initialized), where Go exposes the retained
l.3/l.4 cluster vessel and Rust exposes the ordinary l.3 identity. All final
graphs, scores, candidates and primitive RNG streams match. Both ordinary vs
diagnostic byte-identity checks pass. The focused ten-case matrix retains
six full trace matches and its same four preexisting aggregate failures,
with ten exact public graphs. Evidence is recorded in
`nesting-power-directed-order-seed{1,2}.json` and
`directed-child-order-regression.json`. This is progress on the placement
prefix, not completion of operation-level or full D2 parity.

### Retained synthetic vessel identity

`project_external_active_node` converts an active carrier into its synthetic
vessel ID and box while retaining the member external_id. A targeted probe
shows directions.l.3 has vessel ID 5577006791947779410, not its member ID
760575154. Go's `layoutgraph/node.go:679` DebugID uses vessel metadata for this
pointer. The nested trace walker recognized only members of the aggregate,
so emitted the wrong name despite an exact vessel box.

Added aggregate-key lookup for actual projected vessels in record creation
and traversal. Preserve recorded size/position and discard the obsolete
member-name record; no runtime geometry or comparison rules change. Evidence
is in `projected-vessel-identity-probe.json`. Full nesting seeds 1/2 now match
4821 placement events before the separate root-initialized alias path fails.
All score/candidate/RNG arrays and final graphs match; ordinary/diagnostic
outputs are identical. The ten-case matrix keeps six full trace matches and
ten exact graphs; its reduced direction-grid failure also advances to root
initialized. See `projected-vessel-identity-regression.json` and
`nesting-power-vessel-identity-seed{1,2}.json`.

### Root retained vessel identity

The root trace's `source_record_id` and `append_initial_aliases` still used
ordinary external names for children that already carried a synthetic vessel
ID. Apply the same aggregate-key identity lookup as the nested projection.
The snapshot's existing geometry is unchanged. This follows upstream
`tracePlacement`'s Containers traversal and `Node.DebugID` vessel naming;
comparison normalization remains unchanged.

All ten focused fixture/seed pairs now match exact public graphs and their
complete emitted traces, including the four formerly failing nested cluster
cases. Full nesting seeds 1 and 2 match all 4944 placement snapshots and all
scores, candidates and RNG arrays, with exact public graphs and unchanged
ordinary-vs-diagnostic bytes. Evidence: `root-vessel-identity-regression.json`
and `nesting-power-root-vessel-seed{1,2}.json`. Combined packing/routing-flavor
trace qualification is recorded separately, and does not cover filtered
routing heap/OVG instrumentation.

The combined nesting emitted-trace check (including packing and route-flavor
events) fails at packing event 15, `center` packed: group 0 contains
center.cloud in Rust before the center.queue entry seen in Go. Both seeds
have this same first combined divergence despite exact final graphs and
fully matching placement snapshots. See
`nesting-power-emitted-trace-parity.json` and
`nesting-power-packing-first-divergence.json`. This gate is explicitly not
passing. The grid-cell search witness was independently revalidated and
still fails at neighbor event 1057 (`grid-cell-search-revalidation.json`).

### Packing traversal modes

Full nesting packing event 15 exposed premature center.cloud/hexagon visits
in the virtual aggregate trace walk. Static comparison found two distinctions:
the diagnostic edge sort differs from Go's live Node.Edges, and its near
traversal was unconditional. Removing the sort regressed full nesting while
six focused cases still passed. That experiment was reverted; its reports
`nesting-power-packing-edge-order.json` and
`packing-edge-order-regression.json` are explicitly rejected.

Pinned `packing/binpack.go` constructs cross-container groups with
`AllReachableNodesContext(true, false, false, ...)`: near links are disabled.
Contained and fixed-group walks include near links. Rust's runtime grouping
already distinguishes those modes, but trace_binpack_virtual_group_ids did
not. Return the cross-container prefix count alongside the groups and pass
that provenance into trace construction; the fixed group appended at the
end retains near traversal, as do contained/chosen groups. No group contents,
geometry or comparator rules are changed. All 23 packing tests pass.

Verification of traversal-mode propagation: all ten focused traces and public
graphs pass. Full nesting at both seeds advances to packing event 27, root
directions, with directions.v.d.4 versus directions.v.d.1 at group index 93.
All placement, score, candidate and RNG arrays remain exact, as do final
outputs and ordinary/diagnostic byte checks. The combined trace gate remains
failing. Evidence: `packing-nears-regression.json`,
`nesting-power-packing-nears.json`, and
`nesting-power-packing-child-order.json`.

### Packing endpoint restoration witness

`WEFTAN_TRACE_PACKING_BFS_NODE` and `TALA_TRACE_PACKING_BFS_NODE` accept
comma-separated node names and record endpoint order only under diagnostic
packing instrumentation. Go changes are confined to the instrumented worktree
and included in the pinned trace patch; build-oracle verified all patch bytes
and rebuilt provenance. Both targeted runs preserve exact public outputs.

`packing-endpoint-restoration-probe.json` records the contrast: Go step has
package before external neighbors, Rust has it last; Go down-tree sentinel
has 4,1,3 before the external scorer, Rust interleaves scorer after 4. Source
`placement/node_placement.go` restores abductions around BinPack;
`layoutgraph/layout.go` reconnects original endpoints in abduction order.
The trace's serialized-edge sort masks this lifecycle gap for step and
introduces the first divergence for the tree sentinel. Reconstruct the actual
restoration sequence next, rather than adding a fixture-specific ordering.

### Ordered hierarchy endpoint restoration implemented

PlacementScope retains original endpoint/edge pairs from sized abductions,
using the original-to-projected edge map and stable original endpoint IDs.
Each completed nested scope and then the root appends its restoration list.
After global tree adjacency synchronization, replay those per-endpoint
remove/append operations. The other endpoint is left untouched. The packing
trace consumes that live order without serialized-ID sorting.

This corrects both independently probed endpoint slices and passes all
505881/499094 emitted nesting events at seeds 1/2, with exact final graphs.
Ten focused cases pass; the endpoint-specific regression, 23 packing tests,
29 tree tests (one existing ignore) and 15 tooling tests pass. Both optimized
builds pass and ordinary/diagnostic bytes match. Evidence is recorded in
`nesting-power-endpoint-restoration.json`,
`endpoint-restoration-regression.json`, and `endpoint-restoration-probe.json`.

Source clarification: placeNodes temporarily restores/reapplies abductions
around direct(), then restores them permanently on recursive return before
the subsequent packing stage. The earlier note describing temporary
restoration directly around BinPack was imprecise; the observable endpoint
order comes from that recursive-return restoration sequence. The independent
grid routing-search witness now first fails at event 186 instead of 1057:
the runtime endpoint-order change advances the nesting trace but regresses
this search prefix. Public geometry remains exact. Preserve this explicit
regression for the next investigation; emitted routing-flavor success does
not qualify the internal search order.

### Global tunnel order at owner centers

The grid-edge search regression at event 186 was already present in the OVG
adjacency. At the second distinct center pointer with coordinates (1386,1493),
Go appended tunnel ports (1387,1460) then (1359,1493); Rust reversed them.
`grid-cell-tunnel-order-localization.json` preserves both center records
without merging coordinate duplicates.

Pinned `routing/ovg.go:addTunnels` walks the global buildTunnels result and
appends each EntryA/EntryB to its owner's Ports slice. connectPortsToCenter
consumes that slice. Rust reconstructed owner tunnel lists from owner-local
incident edges, discarding tunnels discovered earlier through another owner.
Removed this reconstruction; owner lists now consume ordered_tunnel_ids,
the same global order used for tunnel construction. A regression with
opposing global-node and owner-incident orders proves the distinction.

The new regression and nine existing tunnel tests pass (one existing ignore).
Grid search matches all 1577 ShortestToLongest, 1608 LongestToShortest and
1550 Default events at both seeds. TopDownLeftRight emits no search events
for this fixture and remains insufficient evidence, not a pass. All ten
focused emitted-trace/public-output cases pass. Final nesting verification
is recorded in `nesting-power-global-tunnel-order.json`.

Full nesting verification passes 505881/499094 emitted events at seeds 1/2,
with exact public graphs and ordinary/diagnostic byte identity. This retains
the endpoint-restoration improvement while resolving its grid-search
regression. It does not qualify TopDownLeftRight (no events on this witness)
or every routing fixture/operation. Broader corpus qualification remains.

### Routing requalification and mixed-root subtree ordering

Re-ran all 26 prior interleaved search cases with current binaries/provenance:
all match, totaling 112127 search events. This includes TopDownLeftRight on
all_shapes_link at both seeds; upstream selects that flavor for hierarchy
routing, so it cannot be qualified using the ordinary grid witness alone.
`routing-search-requalification.json` references the four per-flavor reports.

Expanded to eight upstream fixture/seed cases: nested_diagram_types,
nested_shape_labels, teleport_grid and multiple_constant_nears. All public
graphs are exact, but the initial trace matrix matched only four cases.
teleport_grid first differed at placement 98, users after-fit: the child
records had exact geometry but were deferred behind unrelated roots. The
aggregate-specific after-fit ordering pass emitted ordinary roots without
walking their Containers subtree. It now invokes the ordinary subtree walk
while retaining special aggregate descendant handling.

Five of eight expanded traces now match: teleport_grid seed 2 passes fully.
Seed 1 matches all placement snapshots, then differs at NodePlacement stage
(jita.Slack position). nested_diagram_types remains divergent at placement
180, a after-direct, a.3.x X position. No differences are normalized away.
Evidence: `upstream-routing-expansion.json`,
`teleport-grid-subtree-probe.json`, and
`upstream-subtree-order-regression.json`.

### Stable identities for mirror reachability

nested_diagram_types first differed at placement 180, a after-direct: the
parent box mirrored identically, but Rust reversed a.3.x/y inside it while Go
translated them preserving relative positions. Before-direct snapshots were
exact. mirrorAxes tests EffectiveContainer against reachable containers;
those pointers must not be confused by compact-arena indices.

restored_mirror_reachable_containers previously returned NodeIds from mixed
arenas. Its caller looked for every current/projected node with that numeric
index, inventing reachable identities. Return stable parent TALA IDs instead,
using captured scoring/source-parent identities. Translate to the compact
arena for direct and to original owner IDs for deferred copyback separately.
The strengthened regression asserts original parent identity survives index
zero reuse. Sixteen mirror tests pass (four existing ignores).

Both nested_diagram_types seeds now pass full emitted traces. Expanded
upstream coverage is 7/8 with eight exact final outputs; teleport_grid seed 1
retains its later NodePlacement-stage jita.Slack position mismatch. Full
nesting and all ten focused cases remain passing, including diagnostic/public
byte identity. See stable-mirror-upstream-regression.json,
stable-mirror-regression.json and nesting-power-stable-mirror.json.

## Hidden cluster child-frame localization (2026-09-20)

Upstream `layoutgraph/graph.go:syncNested` calls cluster SyncGeometry and then
adds `containerPadding(cn, true)` to member children. `placement/direct.go`'s
postorder mirror callback calls `node.PositionContainerChildren(true)` even
when that node is not itself reflected. `layoutgraph/node.go` implements that
call using `containerPadding(n, false)`. These are distinct operations: with
child icons their padding floors are 74 and 60 respectively.

Rust's `placement/transactions.rs:diagnostic_trace_projected_positions` always
refits hidden container members with ordinary padding. teleport_grid seed 1
therefore loses a valid 14-unit inset; bypassing the bridge exposes an additional
infra member-frame error and regresses seed 2. The bypass experiment was rejected.
Root `external_shared_child_offsets` also retain negative infra child Y values
and no mirrored offsets at either seed. Correct root projected geometry and
copyback next; do not replace this with fixture-specific diagnostic adjustments.
See `teleport-grid-child-frame-probe.json` and the explicitly rejected
`live-stage-geometry-upstream.json` experiment.

## Preserve cluster member nil state during scope construction (2026-09-20)

`hierarchy/hierarchy_scope.rs:placement_scope_graph` synthesized a member's
arrangement position before cloning it into the temporary aggregate map.
For a previously unpositioned member, that lost the nil-position distinction
used by `layoutgraph/cluster.go:arrangeNodesWithWork`. The Go nil branch sets
the member position and calls PositionContainerChildren(false); its non-nil
branch instead translates the existing subtree by a delta. A synthesized
second-member Y offset therefore caused Rust to omit its initial descendant
translation.

Scope construction now preserves `transaction_position_was_nil` for a local
aggregate member whose source position is absent. Existing
`grouping/clusters.rs:sync_external_cluster` consumes it exactly once and
performs the initial container fit. Subsequent root mirroring then sees the
correct projected descendant geometry. The new scope regression exercises a
container as the second member, where a nonzero arrangement offset exposes
the distinction.

Removed `diagnostic_trace_projected_positions` and the stage serializer's
save/refit/restore pass. Stage traces now observe the runtime coordinates
without changing the geometry for diagnostics. teleport_grid's two seeds
match both different padding histories through the actual runtime operations.

## Cluster sibling order and queue-workers expansion (2026-09-20)

Static source: `grouping/clusters.go:AddCluster` appends the synthetic vessel
with AddNewNodeToContainer, then removes absorbed members from Containers and
Graph.Nodes. Retained source aliases must therefore place that vessel after
remaining ordinary siblings. A carrier's later Graph.Nodes rank must not make
`trace_child_order_is_node_ordered` infer that ordinary siblings reversed.
The diagnostic source walks now replay this sibling transition instead of
substituting the vessel at the first member's declaration slot.

The new upstream offset/label corpus exposed this earliest mismatch in
queue_workers at event 60. It also exposed a separate runtime divergence at
seed 2: stages agree through the first AlignAxes, then GapNormalization shifts
user03, user04, container_producer_02 and part of other_store by three X units.
`other_store` width differs by three as well. Final edge-route X differs
(634 versus 637), so this is not solely a diagnostic ordering issue.
`queue-workers-gap-first-divergence.json` preserves the stage evidence.
Next instrument ordered gap transactions and locate the first accepted move
or rollback disagreement before altering gap reduction.


## Cluster herd scoring and gap exclusions (2026-09-20)

Source mapping: `placementcost/edge_length.go:1295` scores HerdAssignment for
all sized nodes, including vessels; `placement/gapreduction.go:302` excludes
same-orientation herd siblings from the moving connected component, also
without a vessel exception. `proximity/herding.go:129` explicitly handles
assignments on cluster vessels. Rust's stable aggregate carrier retains that
assignment after recursive placement publishes it.

Added opt-in gap pass and proposal events to the instrumented oracle and
Rust, preserving connected-node order, candidate identities, geometry and
IEEE-754 score/translation bits. `--gap` capture and matrix gates require
actual gap events, so matching stage snapshots alone cannot satisfy this
request. The oracle changes remain in the tracked instrumentation patch;
the pristine reference is unchanged. All Rust hook construction is feature
conditional.

queue_workers seed 2 first disagreed on the initial proposal's old score:
Go 28074.52932999149 versus Rust 27394.52932999149. Individual edge terms
matched; the omitted 680 was the cluster vessel's Top herd penalty. Remove
the aggregate exemption in `proximity/herding.rs::herd_penalty` and measure
the active vessel box. The next divergence was a three-unit backward
horizontal proposal: Go moved seven connected nodes, Rust twenty-seven.
Remove the matching aggregate exemption in
`placement/gap_normalization.rs::reduce_gap_to_neighbors_scored_with_overlaps`.
The herd's siblings now remain excluded, as upstream requires.

The new focused test `herd_penalty_uses_active_cluster_box` checks all four
fences, a crossed fence, and an unassigned vessel with distinct member and
vessel geometry. Both queue_workers gap event arrays now match (484/481),
including the previously divergent component. Full release and corpus
requalification is recorded in STATUS.md; this does not establish complete
TALA operation coverage or full D2 compatibility.


## Gap trial decisions and fused symmetry cost (2026-09-20)

Added gap-trial events for direct candidate scores and transaction decisions
(accepted, non-improving, invalid, or Go error). Capture uses already-computed
values, emits exact bits, and adds no scoring calls. Rust hooks and argument
construction are diagnostic-feature conditional. Capture manifests and matrix
reports now retain per-family and per-decision counts to expose coverage gaps.

Eight existing fixture/seed cases match with these events, including both
invalid and non-improving rollbacks (`queue-workers-gap-trials.json`,
`gap-trials-focused.json`). The four-fixture upstream expansion passes the
centered-horizontal and disconnected-right cases at both seeds, but reveals
one-ULP baseline mismatches in ent2d2_basic and ent2d2_right despite matching
public graphs (`gap-trials-upstream-expansion.json`, pre-fix).

Static source and disassembly localize the basic fixture's first mismatch to
`placementcost/graph.go`'s symmetry deduction. The User node's edge cost,
symmetry, edge order/count and column cost agree. Go rounds symmetry*CellSize
then fuses multiplication by edge count with subtraction from the node cost;
Rust rounded the second product too. Ordinary and instrumented Go 1.27 ARM64
binaries both contain FMSUBD. Preserve that order using mul_add in
`placement/transactions.rs::global_sized_edge_length_with_direction_mode`,
shared by restored and ordinary sized scoring. `gap-score-fma-evidence.json` records binary hashes
and extracted instructions. Qualification against other architectures remains
part of the full plan, not proven by this host's disassembly.


Post-fix qualification: `gap-trials-fma-regression.json` passes 10/10,
`gap-trials-fma-focused.json` passes 10/10 and
`gap-trials-fma-upstream.json` passes 28/28. The separately rebuilt release
replays all 48 witnesses byte-identically to both source-oracle and diagnostic
outputs (`gap-trials-fma-release-replay.json`), including witnesses initially
captured while the ordinary release was rebuilding. Seventeen symmetry tests
and sixteen tooling tests pass; no trace normalization or output tolerances
were introduced. Inner-container trial and standalone symmetry-transaction
hooks remain outstanding.


## Inner gap trials and retained traversal views (2026-09-20)

Instrument `placement/gapreduction.go`'s inner-boundary and sibling-clearance
fallback transactions with gap-inner events: moving node, axis, translation,
baseline, optional resulting score and accepted/invalid/regression decision.
Rust's `ordinary_gap_inner_trial` now emits the corresponding event. Invalid
trials have no score, matching Go's validation-before-EdgeLength order. Rust
previously evaluated and cached scores even after rejecting validation; it
now restores the snapshot and returns immediately. Hook construction is
compiled out without diagnostic-traces.

`gap-inner-upstream.json` passes 6/6 and `gap-inner-focused.json` passes 10/10.
The reports separately count inner decisions; the flat SQL-table cases have
zero such events and are not inner-path coverage. Thirteen gap-normalization
Rust tests and sixteen tooling tests pass.

The deeper `gap-inner-nesting.json` run is a failing qualification report:
teleport_grid seeds 1/2 differ in connected traversal (14 Go nodes versus 12
Rust), nearest identity and baseline score. The first source event's vessel
and twelve entries match, but Go also reaches two retained container nodes.
Static comparison: Go connectedNodes iterates every Graph.Containers key;
Rust connected_nodes_excluding_list builds container_orders only from
node_order and canonicalizes carriers into aggregate owners. The retained
container and active vessel pointer views must remain distinct; sorting,
dropping, or duplicating diagnostic IDs would not fix this.

nesting_power seed 1 first differs at gap event 2794 (complete event 508546):
direct trial for node 1148939789, vertical/forward, score bits
0x41376e430160d28a. Go accepts and Rust rejects as invalid. The differing
trial is direct, not inner; later rollback hides it in final stage/output
comparisons. Seed 2 matches 505159 events. All four public graphs still
match. These findings supersede any inference of complete operation parity
from earlier stage-only nesting/teleport reports. Dedicated symmetry-probe
transaction decisions remain uninstrumented.


Final validation for this increment: rebuilt ordinary release outputs match
all 20 expanded witnesses byte-for-byte (`gap-inner-release-replay.json`).
This includes the three explicitly failing operation traces and does not
convert them into passes. The two omitted teleport identities are jita
(3272220923) and infra (815107357), alongside their active vessel. The nesting
node is directions.v.d.2. Exact events, preceding context and full capture
manifests are retained in `gap-inner-first-divergences.json`.


## Active vessel container-alignment scoring (2026-09-20)

The teleport traversal mismatch also carried a separate 801-point score
error. Per-edge comparison isolates four spurious 200.25 turn-cost charges:
each direction of both teleport-to-cluster edges. Distance, routing and
cluster-arrangement components match. Go's `edge_length.go:527` requires
both current endpoints to be IsContainer; an AddCluster vessel is a plain
node. Rust used its first member's is_container flag in a materialized scope.
Exclude active aggregate vessels from this alignment term in scoring.rs,
while retaining ordinary container alignment and endpoint replacement rules.

`teleport-vessel-container-score.json` verifies exact first-proposal score
bits for both seeds with capture provenance. The full teleport trace still
fails: connected traversal omits jita/infra and chooses the vessel instead
of the retained jita boundary. This is deliberately not counted as parity.
`gap-vessel-container-score.json` includes both teleport failures and the
queue_workers successes. The explicit connected-node identity representation
must distinguish vessel and retained members throughout traversal, boundary
selection and translation, not merely in diagnostic serialization.


Reduced witness: `tests/d2/retained-grid-cluster-gap.d2` is a handwritten
11-node reduction of pinned `e2etests/testdata/files/teleport_grid.d2`'s three
grid-container pattern. It contains no remote assets. At both seeds, its
first connected proposal returns nine Go identities versus seven Rust
identities (missing retained first/second containers). It also differs in
intermediate coordinates before that proposal; do not conflate the earlier
geometry mismatch with the traversal cause. `retained-grid-cluster-gap.json`
records both failures despite exact final graphs. It adds two failing
fixture/seed cases, not passing coverage.


Vessel-alignment fix verification: ten focused and 32 broader upstream cases
retain their full emitted-trace matches; queue_workers remains 2/2. Nesting
retains its one known failure and one pass. Rebuilt-release replay of all 50
current witnesses matches both captured public outputs exactly
(`gap-vessel-container-release-replay.json`). Five operation-trace cases
remain failures, including both reduced fixtures. These are not hidden by
the public-output result.


## Connected-node identity representation (2026-09-20)

Added `layoutgraph/connected_set.rs`, corresponding to Node.connectedNodes
in upstream layoutgraph/node.go and the ownership contract tested by
connected_set_test.go. Active(NodeId) denotes the current graph endpoint;
ClusterMember(NodeId) denotes a retained member pointer with its own ID and
box. AddCluster cleared the latter's Container and Reconnect moved its edges
to the vessel, but Graph.Containers still owns the member's children. The
new traversal preserves both ordinary owner and cluster owner links.

The two new tests cover vessel/member distinctions, ancestry, exclusions,
and the upstream 4/12/128-child matrix. All 16 connected Rust tests and
`go test ./d2layouts/d2talalayout/internal/layoutgraph -run '^TestConnectedNodeSet' -count=1`
pass. Upstream's overlapping-owner test explicitly allows undefined order
when several map entries enqueue different parents in the same BFS step.

`WEFTAN_TRACE_CONNECTED_VIEWS` is a feature-gated component probe alongside
ordinary gap diagnostics. It does not alter gap-proposal serialization or
the active movement list. Its result agrees with all 137 Go gap-proposal
connected arrays from six cases (`connected-node-view-components.json`).
The report retains source manifests and input/output hashes. This validates
the new traversal component, not full algorithm parity: boundary selection,
spacing and movement still need to consume the distinct references. Keep
existing full-trace failures visible until that integration is verified.


## Connected-node views in live gap transactions (2026-09-20)

Gap normalization now consumes ConnectedNode references in its two connected
traversals, nearest-between selection, blocked-node tests, proposal identity
and direct translation. Retained container boxes remain distinct from vessel
boxes. OwningContainer still follows the active vessel's owning scope. The
NodeId-only nearest-between wrapper remains for older non-gap callers; it
passes only active references to the shared predicate.

Factored existing spacing and loop geometry arithmetic into shared helpers.
Retained cluster members have no incident edges after Reconnect, so their
spacing does not inherit the vessel's 60-unit connected-edge clearance or
loop extents. A test distinguishes 20-unit retained-member clearance from
60-unit vessel clearance. Translation tests verify that moving the vessel
and retained first member moves each box once and does not implicitly move
an unlisted child. Ownership child slices are precomputed once per BFS,
removing per-visited-node allocations while preserving traversal order.

The parallel CONNECTED_VIEWS_RUST diagnostic probe is removed; ordinary gap
trace events now describe the new runtime list itself. Initial verification
matches full teleport_grid traces (35296/36287 events) and queue_workers
(159560/160212 events). Reduced fixture geometry still differs at the first
gap pass and remains a failing test. Broader requalification is recorded in
STATUS.md and connected-node-runtime reports.


## Retained-container ancestry in alignment validation (2026-09-20)

Static comparison with layoutgraph/layout.go:isDescendantOf shows that the
structural ancestry walk must follow Container, then Cluster.Vessel. The
projected transaction builder previously followed only ordinary containers,
omitting the vessel above a retained cluster container. Reuse ConnectedNode
parent traversal to retain both identities. Tests cover the first and later
cluster member's child ancestry. In the reduced fixture, this removes a false
second.a overlap rejection and matches Go's +279 source move at score 1332.

This is a secondary defect, not the earliest reduced-fixture difference.
Comparing every stage snapshot locates that difference at NodePlacement.
Root copy-back reads column-arranged external aggregate member positions
(279,279 and 279,571) while the row vessel is at (146,279). Its children thus
start in a stale frame. Follow root SyncNestedGeometry and external aggregate
publication next; do not normalize away the intermediate coordinates.
The attempted alignment synchronization change did not address this and was
removed. Baseline and corrected trial evidence is retained in
retained-container-align-ancestry-investigation.json.

Final qualification: 47/50 full emitted traces match; the same three cases
remain failures (reduced witness seeds 1/2 and nesting_power seed 1). The
rebuilt ordinary release matches all 50 public outputs byte-for-byte.
See retained-container-ancestry reports and release replay hashes. Sixteen
connected, fourteen alignment (one inherited ignored) and six projected
transaction tests pass, as do both builds, no-default-features, formatting
and source-structure checks. No performance improvement is claimed.


## Component cluster flip publication (2026-09-20)

Static comparison with placement/node_placement.go:placeNodes (component
optimization before CombineSubgraphs) found Rust published retained aliases
before optimize_projected_cluster_flip, then copied only visible arena nodes
afterward. The reduced fixture's external-sync probe demonstrates a successful
column-to-row flip inside the component, followed by column coordinates in
the parent root copy-back. Move that final component flip before
publish_induced_external_geometry and the retained alias copy-back loops.
This preserves the subgraph algorithm order while publishing the mutations
that Go's shared pointers already expose. The reduced witness now passes
complete emitted traces at seeds 1/2/3/5/42, with counts
6284/6215/6331/6536/6476. This resolves the earlier NodePlacement geometry
difference without normalizing or suppressing intermediate coordinates.

Final copy-back qualification: 52/53 expanded emitted traces match (49/50
of the preceding corpus); all 53 public outputs match the rebuilt ordinary
release byte-for-byte. Only nesting_power seed 1 remains a trace failure.
The five reduced seeds, four upstream cases, ten focused cases and 32 broad
regressions pass. Reports are cluster-flip-copyback-*.json, including release
replay hashes. Seventy-one cluster tests pass with three inherited ignored
tests; both builds, no-default-features, formatting and structural checks pass.

## Fixed-port balance admission and expanded direct corpus — 2026-09-22

The large `big_many_subgraphs` trace localized the remaining post-routing
divergence to `BalanceEdgeSegments`. Rust's fixed-port balance pass was
computing its acceptance metrics without the positioned node, icon, edge, and
arrowhead labels that v0.9.0's `quality.Inspect` includes. It therefore kept a
candidate batch that Go rolls back after one additional text occlusion. The
quality bridge now builds the same label inventory before comparing
obstructions, crossings, text occlusions, and route length. The reusable
alignment transaction also retains Go's baseline overlap set across all
speculative snapshots rather than recomputing it after each move.

With these corrections, the full stage snapshots and final graph state match
for `big_many_subgraphs.json` seed 5, including `BalanceEdgeSegments` and all
later stages. The expanded direct GraphJSON corpus is now **37/37 exact** at
seeds 1, 5, and 42 (`docs/graphjson-expanded-current-20260922*.json`). The
reports compare the pinned Go 1.27.0 v0.9.0 oracle and optimized Rust release;
temporary port diagnostics were removed, and release builds contain no trace
hooks. Broader operation-level transaction and cancellation qualification
remains open.

- **2026-09-21 grid follow-up:** Ported the v0.9.0 d2grid dynamic cut search (fast debt baseline, reverse `iterDivisions`, threshold/skip/attempt bounds, and tie order) and added direct cut-selection tests. Fixed nested grid sizing by preventing a second inset fit on a grid scope and by preserving computed label state when preparing grid margins. Added source-shaped `SizeToContent`, `GetInsidePlacement`, and `GetInnerBox` behavior for all v0.9.0 grid container shapes, including label overflow and cloud aspect selection. Root-level sibling grid scopes are now extracted before parent TALA packing, matching the implicit synthetic root boundary in `d2layouts.LayoutNested`; official-release geometry matches `grid_even`, the shaped-grid, cloud, and cylinder-label witnesses. Broader grid and pixel-render qualification remains open.

- **2026-09-21 render boundary:** Matched `d2exporter.toShape`'s integer truncation for SVG node boxes without changing the fractional layout result. The `grid_even` witness now has exact inner cell widths/heights and surrounding TALA placement; the renderer has a focused truncation regression. Pixel-render qualification still has the documented renderer residuals.
- **2026-09-21 shaped-label boundary:** The SVG renderer now derives ordinary label boxes from the same shape inner boxes used by d2target for circles, ovals, diamonds, hexagons, clouds, cylinders, queues, pages, packages, steps, stored-data, callouts, people, and documents. Cloud labels use the v0.9.0 content aspect ratio before shape fitting, and cloud path coordinates use the source's integer `chopPrecision`. This closes the prior outside-label fallback on shaped grid roots; the cylinder title, one-cloud label, and shaped-grid label baselines now match the official release witness. Supplemental cloud raster witnesses still retain small scan-conversion residuals.

## Fresh retained-container emitted-trace replay — 2026-09-22

The retained-container failure reports were captured before the connected-node
and copy-back fixes. Replaying the complete source-shaped capture now gives
exact stage, placement, candidate, score, packing, routing, RNG, and gap
streams, plus identical public graph bytes, for `retained-grid-cluster-gap.d2`
and `teleport_grid.d2` at seeds 1 and 2. `nesting_power.d2` also matches at
seeds 1 and 2. The comparison removes only Rust's additive stage seed and
retains every other field and event order. The checker and current manifests
are `scripts/check-emitted-trace-parity.py`,
`docs/emitted-trace-retained-grid-current-20260922.json`,
`docs/emitted-trace-teleport-grid-current-20260922.json`, and
`docs/emitted-trace-nesting-power-current-20260922.json`.

## Cluster gap traversal preserves active views — 2026-09-23

The cluster-specific gap adapter previously reused the canonical NodeId
traversal for both nearest-between selection and translation. That erased the
distinction between the live cluster vessel and retained member pointers.
The reducer now uses the pointer-aware `ConnectedNode` traversal and exact
nearest-between helper, filters candidates against live graph order, and
translates the source-visible active views. Nearest-ahead follows the active
vessel edge order and its live box; candidate geometry follows active
aggregate boxes while walking to the nearest shared container. Fixed nodes stay
visible during ordered nearest-between traversal but are excluded from the
translation set, matching the source reducer.

The Rust hand-built fixture had left all cluster members in `node_order`, so
`cluster_is_active` was false; it now hides the non-carrier member and rebuilds
the active aggregate order like Go's `grouping.AddCluster`. A focused Go test
directly invokes the source reducer because the existing upstream cluster test
does not reach that call with its chosen axis. Go and Rust agree on stable
ordered and movement IDs, no nearest-between candidate, -150 movement, exact
baseline/candidate IEEE-754 score bits, and final geometry. A second matched
case confirms a connected fixed node is not translated and the same candidate
is accepted. A nested-container fixture also matches the direct candidate
scores, rejected inner-boundary move, accepted sibling-clearance fallback, and
final container and node positions. See
`docs/cluster-gap-operation-parity-20260923.json`. The diagnostic TALA suite
passes (619 pass, 25 ignored, 2 doctests), and release marker scans confirm
these probes compile out. This does not establish parity for all cluster gap
cases; other nested aggregates and blocker/candidate fallback combinations
remain in the broader trace corpus.


## Slingshot source-order audit (2026-09-27)

The v0.9.0 route worker makes several order-sensitive operations explicit.
`findLShapedRoute` compares base cost and crossing/preference before path
reconstruction, adds endpoint-label penalties before it updates the winner,
and charges one fixed turn after choosing that winner. `findSShapedRoute`
checks each of its three legs in order, stopping at the first blocked leg and
adding that leg's distance and crossing cost before inspecting the next. Both
workers enumerate endpoint ports from the source inventory and charge that
inventory before direction filtering. Rust's slingshot code now mirrors those
boundaries, including short-circuiting the target leg when the source leg is
blocked.

`launch` calls `routeIntersectsNodeGuarded` over the exact
`OVG.NodesInsideBoundingBox` slice. That slice starts with nearby nodes,
continues through non-hierarchy routed-graph nodes in graph order, and appends
the hierarchy OVGs in first-seen order; each hierarchy OVG sorts members by
`Node.ID`/EntityID. The prior Rust `BTreeMap` walk did not preserve this
sequence. The route context now stores that order and the collision scan
consumes it directly.

Verification uses pristine D2 commit
`d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda` with Go 1.27.0. The targeted Go
routing tests pass. Rust slingshot tests pass 7/7. The 19 upstream D2 fixture
seed-1 corpus compares 1,724,896 normalized events and exact public outputs
(19/19); a four-case, two-seed gap-trace comparison matches 432,967 events
(4/4); and `large_nested` GraphJSON traces match for seeds 1 and 123,
including all 100,091 RNG draw events across both seeds. Exact reports are
retained in STATUS.md.

## Recursive-scope abduction projection probe (2026-09-27)

The Rust-only three-container test initially represented a descendant endpoint
with its carrier's TALA identity plus a nonzero local offset. Static comparison
with `optimizerAdjacents` and `optimizerMedian` showed that Go instead
substitutes and measures the original endpoint node pointer. A focused test in
the instrumented v0.9.0 source now constructs that pointer and verifies its
center is `(-129, 476)` pixels, or `(-129/269, 476/269)` cell units.

Rust's synthetic graph now gives the projected descendant a distinct identity
and a live box under the carrier's external-child map, so the carrier move
translates the endpoint as `Graph.moveNodeWithChildren` does. The corrected
test passes its expected geometry; all TALA tests pass (**674 passed, 25
ignored; 2 doctests passed**) and the Go placement package passes. The original
debugger graph is not present in the pinned source, so this resolves a fixture
modeling error rather than establishing another end-to-end D2 parity case.


## Go 1.27 orientation operation order — 2026-10-01

Static source mapping is `layoutgraph/node.go::orientation`, used by
`layoutgraph/edge.go::Edge.isStraight`; Rust uses
`internal/labeling/labels.rs::orientation` for both `is_straight` and
`segments_intersect`. Rust had formed the second determinant leg from `r-q`,
which is algebraically equivalent but changes IEEE-754 rounding. The pinned Go
1.27.0 macOS arm64 disassembly emits `FMSUBD` for the final multiply-subtract.
Rust now matches its fused result on AArch64 and uses the source expression's
separate operations otherwise. The pinned Go 1.27.0 amd64 `GOAMD64=v1`
cross-build was disassembled and uses separate `MULSD`/`SUBSD` operations.

The arm64 source test and AArch64 Rust test agree on the crafted determinant's
exact bits (`41abf24bb170e2e0`) in both debug and optimized release builds.
The route-level witness considered during the audit remained non-collinear in
Go and was discarded; no final geometry change is claimed from that witness.
Other release-target flags and full cross-platform behavior remain to be
qualified.
