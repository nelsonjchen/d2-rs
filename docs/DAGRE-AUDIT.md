# Dagre source and validation audit

Evidence JSON cited below as `docs/*.json` is no longer in the tree. It was
retired at 9b706ac1 and in the 2026-10-31 second pass; `docs/RETIRED-EVIDENCE.md`
lists each file and how to read it back. Checkers now write fresh reports
under the gitignored `oracle-cache/reports/`.

This records the Dagre/sequence work against D2 **v0.9.0**, commit
`d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda`, and its pinned
`github.com/d2lang/dagro v0.2.1` dependency. The reference toolchain is
**Go 1.27.0**. [reference-manifest.json](../reference-manifest.json) is the
authority for source, release checksums, dependencies, and acquisition.
This audit is not a declaration of full Dagre or D2 parity.

## Source mapping

Paths below are relative to the pinned upstream repository or Dagro module.
Rust files are relative to this repository. The mapping identifies translated
behavior; it does not imply every branch has been qualified.

| Upstream source and symbols | Rust source and symbols | Boundary and qualification notes |
| --- | --- | --- |
| D2 `d2layouts/d2dagrelayout/layout.go`: `Layout`, `positionLabelsIcons` | `crates/d2-rs-dagre/src/lib.rs`: `layout_with_options`, `label_state` | Adapter options, dimension/rank separation, effective label/icon positions, placement, fitting, routing. Several older non-expanded compound paths still use specialized helpers. |
| D2 `layout.go`: `endpointResolver.resolve`, `getLongestEdgeChainEndpoints` | Dagre `lib.rs`: `resolve_edge_endpoints`, `container_endpoints` | Container endpoints resolve through the selected child chain before ranking; published routes are later clipped back to the authored endpoint. |
| Dagro `acyclic.go`: feedback-edge traversal and reversal | Dagre `lib.rs`: `acyclic_reversal_order`, `acyclic_reversed_edge_ids`, `dagre_edge_order` | Edge identities and reversal order are retained; a pair set alone is insufficient for parallel edges. |
| Dagro `nesting_graph.go`: `runNestingGraph`, `nestingDFS`, `treeDepths`, `cleanupNestingGraph`; `util.go`: `asNonCompoundGraph`, `removeEmptyRanks`; `layout.go`: `assignRankMinMax` | Dagre `lib.rs`: `source_compound_rank_model`, `source_compound_rank_assignments`, `network_simplex_values` | Returns normalized leaf ranks **and actual normalized top/bottom border ranks**. Compound objects and extracted sequence actors are excluded from simplex vertices. Nesting edges constrain ranking but are removed before normalization/ordering; they must not be inserted into the final BK graph. |
| Dagro `rank.go`, `network_simplex.go` and `rank_*.go`: rank/simplex helpers | Dagre `lib.rs`: `network_simplex_ranks`, `network_simplex_values` | Both legacy and source compound rank paths remain. Network-simplex operation order still requires broader qualification. |
| Dagro `layout.go`: `injectEdgeLabelProxies`, `removeEdgeLabelProxies`; `util.go`: `removeEmptyRanks`; `normalize.go`: `normalizeEdge` | Dagre `lib.rs`: `source_compound_rank_model`, `compact_compound_rank_layers`, `BkModel::from_graph`, `rank_nodes` | Label proxies occupy the raw midpoint before empty-rank compaction. Their transformed ranks survive normalization as `labelRank`; only the dummy at that rank receives the label dimensions, including its flow-axis span. Fractional proxy ranks retain the source array's non-index behavior. This replaces choosing the first dummy or recomputing a midpoint from the final endpoint ranks in the source compound path. |
| Dagro `normalize.go`, `parent_dummy_chains.go`: normalized edge chains and parent assignment; `add_border_segments.go` | Dagre `lib.rs`: `BkModel::from_graph`, `assign_parent_dummy_chain_owners` | A dummy belongs to the ancestor whose rank interval contains it, walking through the least common ancestor. Nested border ranks are read from the rank model, not inferred as leaf rank plus/minus depth. |
| Dagro `order*.go`: layer graph construction, `sortSubgraph`, barycenters, conflict resolution, subgraph constraints, crossing counts | `crates/d2-rs-dagre/src/order.rs`: `order_compounds`, `LayerGraph`, `sort_subgraph`, `barycenter`, `resolve_conflicts`, `sort_entries`, `add_subgraph_constraints`, `cross_count` | Compound tokens are recursively expanded within their borders. Flat grouping of every owner was insufficient for nested containers. |
| Dagro `position_bk.go` and `position.go` | Dagre `lib.rs`: `brandes_koepf_source_container_coordinates`, `bk_vertical_alignment_model`, BK compaction helpers | Synthetic borders participate in alignment and lane width, with graph traversal order preserved where represented. |
| D2 `d2graph/d2graph.go`: `Object.Spacing`, `SpacingOpt`; `d2graph/layout.go`: `GetModifierElementAdjustments` | `crates/d2-rs-dagre/src/spacing.rs`: `object_spacing`, `apply_decoration` | Label dimensions plus 10px and icon size 64px plus 10px contribute to the selected side via maximum, then 3D/multiple offsets are added. TALA's `content_insets` are not a substitute for this calculation. |
| D2 `layout.go`: `getRanks`, `adjustRankSpacing`, `shiftDown`, `shiftUp` | `spacing.rs`: `get_ranks`, `adjust_rank_spacing`, `shift_down_routes`, `shift_up_routes` | Geometric ranks use `ceil(start + size/2)` for active leaves. Ancestor envelopes grow and earlier/later objects shift in the flow direction. Pre-spacing routes share this mutation stage, including endpoint guards against the updated object envelopes. |
| D2 `layout.go`: `adjustCrossRankSpacing`, `shiftReachableDownIndexed` | `spacing.rs`: `adjust_cross_rank_spacing`, `shift_reachable`, `Reachable` | Traverses parents, children, incident edges and nearby overlapping objects; tracks margins already added. In the non-expanded source path, this pass advances the route shadow while the source adapter's box correction owns published geometry, avoiding a second application of the same cross-rank shift. |
| Dagro `layout.go`: `assignNodeIntersects`; `util.go`: `intersectRect` | `crates/d2-rs-dagre/src/route_geometry.rs`: `dagro_intersect_rect`; Dagre `lib.rs`: `DagreRouteModel::source_adapter_points` | Intersects the raw Dagro center and dimensions with the first/last dummy corridor, preserving arithmetic order and JavaScript numeric truthiness. It does not reconstruct those intersections from rounded public rectangles. This path currently covers expanded, non-self edges with a usable route model. |
| D2 `layout.go`: `Layout` route scan, `deduplicateRoutePoints`; `lib/geo` rectangle/segment intersections | Dagre `lib.rs`: `source_adapter_points`, `clip_route_to_authored_boxes`, `go_box_intersections` | After restoring authored direction and deduplicating, scans and clips both authored endpoint boxes before spacing. This includes ordinary leaf endpoints, not only compound endpoints. Final shape tracing occurs after the compared fit boundary. |
| D2 `lib/shape/shape.go`: `TraceToShapeBorder`; `lib/geo/ellipse.go`: ellipse intersections, with point/vector distance helpers | `crates/d2-rs-dagre/src/shape_geometry.rs`: `trace_to_shape_border`, `ellipse_intersections`, `distance_to_segment` | Final circle/oval endpoint tracing follows the rectangular route chop, preserves independent ellipse axes, intersection-root order and finite-segment filtering, and selects the source perimeter intersection. This replaces endpoint offset guesses for the qualified circle/oval witnesses; it does not qualify every other shape. |
| D2 `layout.go`: `fitPadding`, `fitContainerPadding` | Dagre `lib.rs`: `initial_source_routes`, `fit_containers_source` | Fits rectangular containers after spacing; considers children, internal route points and internal edge-label boxes. Routes created before spacing are threaded through rank/cross spacing and fitting. The source envelope is kept through spacing so fitting does not translate children to repair an incorrectly small box. |
| D2 `lib/label/label.go`: `Position.GetPointOnRoute`, `chopPrecision`; `lib/geo/math.go` distance arithmetic | `crates/d2-rs-core/src/route_label.rs`: `top_left`, `point_at_distance`, `euclidean_distance`, `chop_precision`; Dagre `route_geometry.rs`: fitting wrapper `edge_label_top_left` | Shared by container fitting and rendering. Fits internal edge-label boxes using stroke width 2 and percentage 0; rendering supplies target stroke width/percentage. Preserves the stored control-point polyline, interpolation, normal offsets, axis-aligned distance paths and float32 narrowing before final rounding. It does not sample the rendered Bézier curve. Parent inside-label/icon collision handling remains a separate gap. |
| D2 `d2exporter/export.go`: route export; `lib/geo/point.go`: `TruncateDecimals`, `TruncateFloat32`; `d2target/d2target.go`: `Connection.GetLabelTopLeft`, `Diagram.BoundingBox` | `crates/d2-rs-render/src/lib.rs`: `target_route_coordinate`, `target_route_point`, `edge_label_bounds`, `bounds`, `target_hash_connection` | Target routes truncate toward zero at three decimals, then narrow to float32, exactly once at that boundary. Label bounds use those target coordinates and the shared source label positioning, including stroke width and float32 label percentage. Exact label rectangles feed SVG placement and overall bounds; pixel qualification remains separate. |
| D2 `d2graph/d2graph.go`: `Edge.AbsID`, `initIndex`, `IsEquivalent` | Render `lib.rs`: `edge_svg_id`, reused by `target_hash_connection` | Preserves common-ancestor scope, quoted dotted identifiers, arrow direction, and parallel-edge indexing including equivalent reversed endpoints. The renderer and target-hash path share the same source-style edge identity; these IDs are distinct from the diagnostic gate's length-prefixed identities. |
| D2 `d2layouts/d2layouts.go`: `LayoutNested`, extraction/reinjection; `d2sequence/layout.go` and `sequence_diagram.go` | `crates/d2-rs-layout/src/lib.rs`: Dagre dispatcher; `sequence.rs`: `project_for_dagre`, `layout_scope`, `layout_inner`, `apply_nested_scopes` | Sequence contents are laid out before the parent Dagre graph and restored afterward. The shared Rust graph retains actor records, so every parent rank/order/spacing boundary must exclude those descendants. Internal sequence edges cannot affect parent ranks. |
| D2 `d2target/d2target.go`: `GetIconSize`, `Diagram.BoundingBox`; `d2svg/d2svg.go`: icon rendering; `lib/label/label.go`: `GetPointOnBox` | `crates/d2-rs-render/src/lib.rs`: `icon_size`, `icon_rect`, `outside_icon_envelope`, `node_label_box`, `bounds` | Icon size depends on shape dimensions, with 32px default and 64px maximum. BoundingBox reserves the selected outside side even for image shapes. Effective layout icon positions feed rendering, bounds and target hashing. |

The sequence rank regression is
`compound_ranks_preserve_the_extracted_sequence_boundary`. It compares an
extracted graph with the same graph retaining deep sequence actors/internal
edges, and checks exact nested border intervals. Ordering, spacing and icon
envelope tests live beside their implementations.

## Durable reference and trace reproduction

Run from `~/code/d2-rs`. The pristine source remains `../d2`; the historical
instrumented worktree is `../d2-v0.9.0-trace`. For the Dagre oracle, the builder
now uses a verified dependency copy and a Go source overlay under `.reference`
rather than relying on edits in that worktree or a dependency copy in `/tmp`.

```sh
uv run --no-project python scripts/references.py fetch
uv run --no-project python scripts/references.py verify
uv run --no-project python scripts/build-dagre-trace-oracle.py
```

The builder verifies the source commit, D2's Dagro module checksum and Go
toolchain. It applies the tracked patches
`tools/patches/dagro-v021-diagnostic-trace.patch` and
`tools/patches/dagro-v021-rank-ops-trace.patch`,
`tools/patches/d2-v090-dagre-adapter-trace.patch`, and
`tools/patches/d2-v090-trace-object-scope.patch`. The resulting executable is
`.reference/bin/d2-v0.9.0-dagre-trace`; its adjacent `.provenance.json` records
binary, patch, dependency-file and builder hashes plus the actual build command.
The dependency copy is `.reference/dagro-trace`, the adapter overlay is
`.reference/dagre-trace-source`, and the temporary Go modfile/overlay manifest
are in `.reference/dagre-trace-module`.

On `icon_positions` and `sequence-trace-order`, the builder compares the
official asset, pristine source build, instrumented build with tracing off,
and instrumented build with tracing on. It records both geometry and byte
observations in `.reference/dagre-trace-validation/report.json`; tracing is
required to leave the instrumented SVG unchanged and ordinary output must not
contain diagnostic records. These two witnesses establish instrumentation
noninterference only for those runs. The compact tracked
[final-route oracle report](dagre-final-oracle-validation-20260922.json) retains the
official/source/instrumented hashes, exact SVG/stdout equivalence, trace counts
and patch provenance for the oracle that emits five route snapshots, ending
after final shape tracing and curve conversion. The earlier
[four-phase oracle report](dagre-route-oracle-validation-20260922.json) is
retained as historical evidence for its recorded binary.

```sh
cargo build --profile diagnostic -p d2-rs-cli --features diagnostic-traces
uv run --no-project python scripts/check-dagre-stages.py \
  icon_positions sequence-trace-order dagre-compound-wide-label \
  dagre-compound-label-midpoint --routes \
  --output docs/dagre-route-boundary-current.json --summary

uv run --no-project python scripts/check-dagre-stages.py \
  icon_positions sequence-trace-order dagre-compound-wide-label \
  dagre-compound-label-midpoint --routes --final \
  --output docs/dagre-final-boundary-current.json --summary
```

To inspect source internals directly, retain the output under `.reference`:

```sh
mkdir -p .reference/dagre-inspection
DAGRE_TRACE_OBJECTS=1 DAGRO_TRACE_RANKS=1 DAGRO_TRACE_ORDER=1 DAGRO_TRACE_POS=1 \
  .reference/bin/d2-v0.9.0-dagre-trace --layout=dagre \
  tests/d2/icon_positions.d2 .reference/dagre-inspection/go.svg \
  2> .reference/dagre-inspection/go.stderr
DAGRE_DEBUG_PHASE=1 target/diagnostic/d2-rs --layout=dagre \
  tests/d2/icon_positions.d2 .reference/dagre-inspection/rust.svg \
  2> .reference/dagre-inspection/rust.stderr
```

Use the full CLI/orchestration path for mixed sequence/grid witnesses. Calling
`d2dagrelayout.Layout` directly on an unextracted compiled graph sends sequence
actors to Dagro and is a different oracle boundary. Old direct-call debug logs
must not override the official CLI result.

## What each gate establishes

`scripts/check-dagre-stages.py` compares four snapshots in order:
`rank_nodes` ↔ `dagro`, `rank_spacing` ↔ `rank-spacing`,
`source_cross` ↔ `cross-spacing`, and `fit` ↔ `fit`.
Every object identity and all four x/y/width/height IEEE-754 bit patterns must
match. Missing, empty, duplicate or reordered required phases fail, as does a
different object set. It checks displayed values against supplied bits. The
current gate accepts a single enclosing Dagre invocation; repeated nested
invocations fail explicitly.

With `--routes`, each selected phase must also contain exactly one explicit
`GO_EDGESET`/`RUST_EDGESET count=N` marker, including `count=0` for edgeless
snapshots. Every edge identity, the emitted edge order, point count, point order
and x/y IEEE-754 bits must match. Missing markers, duplicate identities,
malformed payloads and inconsistent counts fail; absent route records cannot
silently count as an edgeless graph. Edge identities are opaque JSON-quoted
strings of the form `len(src):src -> len(dst):dst #ordinal`, with UTF-8 byte
lengths, authored endpoint direction and a zero-based duplicate ordinal per
ordered endpoint pair. The comparator neither renames edges nor sorts their
order. Without `--routes`, the gate retains its object-only scope.

Adding `--final` requires a fifth `final` ↔ `final` snapshot after the existing
four phases, observing shape tracing and route curve conversion. The same
strict object and, with `--routes`, ordered-route checks apply. A missing,
duplicated or reordered final phase fails; an edgeless final snapshot still
needs its explicit zero-count marker. This optional mode does not change the
default four-phase scope. It can locate differences introduced after fitting,
but it does not compare target construction or rendering.

The strict route replay covers the icon, mixed-sequence, compound wide-label
and compound label-midpoint witnesses: 91, 13, 6 and 6 parent-layout objects;
2, 6, 3 and 4 edges; and 6, 34, 12 and 16 route points respectively at each
of the first four boundaries. Curve conversion changes the final point totals
to 8, 54, 12 and 22. Across all five boundaries and four fixtures, each engine
contributes **580 object snapshots and 368 point snapshots**: 272 points from
the original boundaries plus 96 final points. The gate compares 2,320 object
and 736 route-coordinate IEEE-754 bit patterns, along with all 75 ordered
edge snapshots. This remains **five boundary snapshots, not operation-level
trace parity**. Object enumeration order, intermediate decisions, scoring,
traversals, target construction and rendering are outside this comparison.

The follow-up report `docs/dagre-boundary-routes-20260924.json` extends the
same gate with `nested-order.d2`. Its five snapshots now match exactly as well:
the Rust `rank_nodes` boundary is emitted after compound pre-fit, external-lane
placement, and the source adapter's coordinate rounding; source routes are
captured for vertical and horizontal flow before spacing, then traced through
fit. This closes the earlier `other.y` boundary gap (166 versus 266) without
changing final geometry. The resulting five-fixture gate passes all object
bits and ordered route points; it still does not establish operation-level
Dagro trace parity.

`scripts/check-dagre-parity.py` compares the root SVG viewBox and a sorted
multiset of geometry-bearing shape elements. Numeric attributes are rounded
to six decimal places; path/polygon presence is retained but path `d`, polygon
points, object identity, image/text geometry and edge routes are not compared.
Polygon tags normalize to path tags to accommodate equivalent shape writers.
Consequently, a match is useful final-layout evidence but cannot establish
exact float, rendering, route or error parity. The schema-2 gate treats a Rust
failure on a Go-accepted fixture as failure; Go rejection is marked
inapplicable to successful-layout geometry, with no claim of error parity.
Timeouts, missing executables and malformed output are failures.

Example focused and complete local fixture runs:

```sh
uv run --no-project python scripts/check-dagre-parity.py \
  icon_positions sequence-trace-order \
  --rust target/diagnostic/d2-rs \
  --output docs/dagre-layout-parity-current.json --summary

dagre_fixtures=()
for dagre_fixture in tests/d2/*.d2; do
  dagre_fixtures+=("${dagre_fixture:t:r}")
done
uv run --no-project python scripts/check-dagre-parity.py "${dagre_fixtures[@]}" \
  --rust target/diagnostic/d2-rs \
  --output docs/dagre-layout-parity-all-current.json --summary
```

The loop uses zsh filename modifiers. Explicitly select the verified official
release executable with `--go` for release comparisons; on macOS ARM64 it is
`.reference/releases/macos-arm64/d2-v0.9.0/bin/d2`. The default is the separately
verified pristine source build, not the official asset.

At this audit's 2026-09-22 checkpoint, the durable oracle report showed both
original witnesses passing its official/source/instrumented equivalence
checks, including equal SVG bytes and stdout with route tracing enabled.
The earlier [four-phase boundary report](dagre-route-boundary-bits-20260922.json) records
**4 matched, 0 divergent, 0 failed**, with exact objects and ordered routes at
all four boundaries. Its Rust executable SHA-256 is
`fa7e6201a1401cfce5c0c6892c8bf54f83d6576d4a35ae6271765ebfcc15353b`;
the instrumented Go SHA-256 is
`e8628930ee0804d6cb460ed3e9ab1efa5e3e881202b2e2b6da0ef2459cea0dfd`.
The midpoint regression exercises proxy-preserved label rank and the label's
route-positioned box during fitting; matching the route alone had previously
left a different fitted container.

The first five-phase replay found **1 matched, 3 divergent, 0 failed**. Icons
still matched; mixed sequence, wide-label and label-midpoint first differed in
the `final` phase at a source endpoint's x coordinate. The first four phases
remained exact. This exposed legacy fixed endpoint offsets applied after the
matching fit snapshots. That adjustment helper has been removed, and the
circle/oval source tracing described above now supplies the perimeter step.
The [five-phase boundary report](dagre-final-boundary-bits-20260922.json)
records **4 matched, 0 divergent, 0 failed** across objects and ordered routes,
including the final curve-converted output. Its Rust executable SHA-256 is
`2c084ae3b673cd185b7ac8dfb6cc54b031eed15926d372835afa6c6bbee6d1fa`.
The five-phase Go oracle itself is verified by the
[new provenance report](dagre-final-oracle-validation-20260922.json), with
executable SHA-256
`6ccb123db58009d76b0e3261789bb29577ef39fc3c3ac88a8a8dbb00bc461d27`.
Its two noninterference witnesses each emit five phase/edge-set markers;
icons emit 10 edge records total and mixed sequence emits 30. The final
snapshots contain 8 and 54 route points respectively after curve conversion.

The ordinary release SVG-geometry replays against the
[pristine source build](dagre-route-geometry-source-20260922.json) and
[official release](dagre-route-geometry-official-20260922.json) each recorded
**34 matched, 0 divergent, 0 failed, 9 upstream-inapplicable** out of 43
fixtures. Both used Rust executable SHA-256
`6709bf925733e39b6ee4fa6fd47fbfb3c8f3037541853a426b031dc6c61a5590`.
Those are successful-layout geometry counts under the narrower comparator,
and they predate the newest shape, shared-label and rendering changes.
They establish qualification for that recorded release binary, not for a
later executable rebuilt from the current working tree.
Older `dagre-layout-parity-*-20260922*.json` files are historical reports and
must not be read as evidence for binaries built after them. Current validated
counts and unresolved witnesses belong in [STATUS.md](STATUS.md).

## Fresh Dagre requalification — 2026-09-23

The static trace audit found `icon_positions` had identical final geometry but
a different node-registration inventory in the compound-order and BK traces.
Five Go 1.27.0 captures retained the same order, ruling out map-order noise.
The cause was D2's `Graph.SortObjectsByAST`: Dagre registers source objects
after sorting by the selected component of each object's first AST reference,
while Rust had used recursive field-creation order. Rust now stores that
source order separately from stable arena IDs and feeds it to the Dagre
registration inventory. A focused graph-lowering regression covers a child
whose source occurrence follows a later sibling.

After the fix, order-sweep traces, four BK alignment/root/coordinate traces,
and strict five-boundary snapshots with ordered routes all match **4/4** for
`icon_positions`, `sequence-trace-order`, `dagre-compound-wide-label`, and
`dagre-compound-label-midpoint`. These compare exact layer order, crossing
cost bits, alignment roots, conflict sets, object dimensions and route-point
bits. Reports:
[`dagre-order-trace-after-source-order-20260923.json`](dagre-order-trace-after-source-order-20260923.json),
[`dagre-position-trace-after-source-order-20260923.json`](dagre-position-trace-after-source-order-20260923.json),
and [`dagre-final-boundary-after-source-order-20260923.json`](dagre-final-boundary-after-source-order-20260923.json).
The Rust diagnostic CLI SHA-256 is
`b3f032cef9f2cc6f7af8a0b34e122d6314f67bc24558977a80ed392590ab5c2b`; the Go
trace oracle SHA-256 is
`b0ed30fa6578bd24b678e70a8bbdd79fa9f83c31bd8956eef6b44d359feaa6f4`.

The whole local Dagre geometry corpus covers all 45 `.d2` fixtures: 36
accepted fixtures match and nine unsupported/invalid fixtures are explicitly
marked upstream-inapplicable in both ordinary release comparisons. Current
reports use the pinned Go 1.27.0 source build and official v0.9.0 binary:
[`dagre-layout-parity-all-crossshadow-source-20260923.json`](dagre-layout-parity-all-crossshadow-source-20260923.json)
and
[`dagre-layout-parity-all-crossshadow-release-20260923.json`](dagre-layout-parity-all-crossshadow-release-20260923.json).
This broader geometry checker compares normalized SVG boxes; it is weaker
than the strict trace/boundary reports and does not prove every Dagre path.
After the non-expanded cross-spacing correction, the four strict final-route
witnesses still match at all five boundaries; see
[`dagre-final-boundary-crossshadow-20260923.json`](dagre-final-boundary-crossshadow-20260923.json).
The nine CLI rejections comprise eight external-plugin feature checks, which
are out of scope, and one unsupported Markdown HTML element. The latter now
matches D2's exit status and path-qualified stderr exactly; both all-fixture
reports retain the captured error values.

## Remaining gaps

- Pre-spacing routes survive rank spacing, cross spacing and container
  fitting, and the five-phase gate now compares the final shape-traced and
  curve-converted route points for the four witnesses above. This does not
  qualify all topologies, other shape perimeters, rendered paths or the exact
  public route/error contract.
- Proxy occupancy and transformed `labelRank` now exist in the source compound
  model, including an otherwise empty midpoint layer and fractional-rank
  behavior. Non-expanded legacy rank/label-dummy paths still need the same
  source translation and broader labeled, reversed and parallel-edge coverage.
- Raw `intersectRect` and clipping against both authored endpoint boxes are
  implemented for the expanded non-self route path. Self-loop construction
  and fallback routes still follow older helpers. An intersection/model
  failure can select a fallback rather than reproducing upstream's error;
  exact error propagation remains unqualified.
- `build_dagre_route_model` still anchors reconstructed flow centers to a
  published leaf box. Odd dimensions, rounding, reverse directions and other
  anchors need direct raw-position qualification; four exact witnesses do
  not establish that this recovers every raw Dagro coordinate.
- Fitting now includes internal edge-label boxes at their actual route
  positions and measures every internal route point, including points beyond
  the current vessel border, matching v0.9.0 `fitPadding`. The unsupported
  Rust out-of-vessel point filter has been removed. The 35-fixture geometry
  corpus and four exact ordered-route boundary witnesses pass after that
  change. The inside-parent-label/icon overlap backoff against outside child
  labels/icons and internal edge labels is now ported. A new witness also
  exposed that non-expanded layouts ran `adjustRankSpacing` only on the shadow
  route boxes; it now updates the object boxes and the `rank_nodes` trace
  boundary follows compound pre-fit. The new fixture matches the pinned Go
  1.27 source build and official v0.9.0 binary in the 36-fixture geometry
  sweeps. The witness also has exact object bits at all five layout boundaries;
  that boundary report does not compare route points. Older fitting paths and
  topologies still need broader source qualification.
- The current [order sweep](dagre-order-trace-after-source-order-20260923.json)
  and [Brandes–Köpf trace](dagre-position-trace-after-source-order-20260923.json)
  compare four witnesses, including `icon_positions`; all layer sequences,
  crossing-cost bits, alignment roots, conflict sets, and raw coordinates
  match. The older 35-fixture network-simplex report used numeric object aliases
  and is retained as historical evidence. The current stable-path comparator
  matches all operation events on 17 of 18 upstream fixtures; the remaining
  fixture emits no network-simplex events in either engine and is explicitly
  inapplicable. On the 41 successful local geometry fixtures, 40 operation
  traces match and the other is likewise event-free on both sides. See
  `dagre-rank-ops-upstream-stable-identities-20260924.json` and
  `dagre-rank-ops-local-matched-stable-identities-20260924.json`. The source
  overlay preserves object paths across extracted/re-rooted scopes and
  normalizes undirected tree endpoints only after ID translation. BK block-graph construction,
  horizontal-compaction decisions, and the rest of full Dagre execution still
  need broader event-level coverage.
  The expanded order-sweep trace covers ordinary as well as compound ordering.
  Go's `SortObjectsByAST` fallback compares the live `sort.Slice` indices when
  either first reference is missing or variable-derived; the Go 1.27 PDQsort
  port now mirrors that index-sensitive behavior. This fixes the 82-node
  `grid_label_positions.d2` registration difference. A separate source audit
  found and removed Rust's flattened nested-grid prefit, which had no Go
  counterpart and caused the remaining extra order-trace groups. Current
  reports match all 17 applicable upstream calls and all 39 applicable local
  calls, with one event-free fixture in each corpus and no fixture-level
  divergences. The local release geometry gate matches 15 fixtures; eight are
  inapplicable because both engines return the same error. Matching scopes
  retain exact registration, sweep events, and invocation count/order.
  `grid_label_positions` is pixel-exact. The one-pixel
  `shaped-grid-positioning` residual recorded here was resolved on 2026-09-25;
  the expanded source and official-release raster checks now match 17/17. See
  the current reports `render-source-go1.27-20260925.json` and
  `render-official-go1.27-20260925.json` and the implementation note at the top
  of `STATUS.md`.
  Reports are `dagre-order-trace-no-grid-prefit-upstream-20260924.json`,
  `dagre-order-trace-no-grid-prefit-local-20260924.json`,
  `dagre-layout-parity-no-grid-prefit-release-20260924.json`, and
  `dagre-no-grid-prefit-render-pixels-release-20260924.json`. The targeted
  latency smoke report is
  `dagre-grid-prefit-latency-smoke-20260924.json`; it does not replace the
  plan's multi-input, phase-separated benchmark qualification.
- Go's cross-spacing ancestor-growth loop ranges over a map, while Rust walks
  the queue's discovery order. A new source-only trace hook measured four
  fixtures over 20 fresh Go runs each: map order varied in 23 of 24 calls, but
  every fixture's Go SVG stayed byte-identical. The official release, clean Go
  1.27 source build and instrumented source build also produced identical SVG
  bytes on all four fixtures. Rust and Go had matching call contexts and
  discovered-node sets in all 24 calls; Rust's order matched the first Go
  sample in 3 calls. Tracing on/off preserved SVG bytes in both implementations.
  This establishes trace nondeterminism on this corpus, not that visitation
  order is irrelevant for every graph. Keep the actual order visible and extend
  the corpus before changing the stable Rust walk. The optimized release also
  passes the four-fixture geometry gate, and the diagnostic five-boundary
  ordered-route gate matches all three supported direct-layout witnesses. See
  [`dagre-cross-map-order-20260924.json`](dagre-cross-map-order-20260924.json),
  [`dagre-cross-map-release-qualification-20260924.json`](dagre-cross-map-release-qualification-20260924.json), and
  [`dagre-cross-map-boundary-routes-20260924.json`](dagre-cross-map-boundary-routes-20260924.json).
  Reproduce the map-order run with `uv run --no-project python
  scripts/check-dagre-cross-map-order.py --runs 20`.
- Generic flow-axis rank spacing now updates boxes in expanded and
  non-expanded source paths. Cross-axis spacing still combines the exact
  expanded pass with source-specific non-expanded handling, so this does not
  establish correctness of all single-compound, flat, direction,
  fixed-position or mixed-grid paths.
- Snapshot and SVG-shape gates omit renderer pixels, SVG paths/labels/images,
  font metrics, and the exact public route/error contract. Render/raster and
  final-route/error witnesses remain separate qualification gates.
- The expanded decoded-pixel gate now matches all 15 fixtures, including both
  compound-label witnesses; see the exact-hash
  [group-mask report](render-raster-group-mask-20260922.json). The midpoint
  residual was caused by masking the route and arrowheads separately in the
  PNG-only SVG, whereas D2 masks their shared geometry group once. The raster
  adapter now keeps the route and triangle arrow paths under one mask group,
  and the high-precision arrow transform preserves Go's f64 scene coordinates.
  The standard SVG output is unchanged. Both diagnostic and optimized release
  builds match 15/15; see the [release report](render-raster-group-mask-release-20260922.json).
  The older 13/15 report below is retained as historical evidence. The
  target-route observation remains
  reproducible with
  [`tools/d2-target-probe/main.go`](../tools/d2-target-probe/main.go), run from
  the pinned source checkout:

  ```sh
  cd ../d2
  GOTOOLCHAIN=go1.27.0 go run ../d2-rs/tools/d2-target-probe/main.go \
    ../d2-rs/tests/d2/dagre-compound-wide-label.d2
  ```
- Diagnostic phase construction is behind `diagnostic-traces`; use an
  ordinary release build for performance and separately verify that diagnostic
  strings/environment checks/argument construction are absent. An optimized
  diagnostic build is not a release-performance result.

## Attribution

D2 adapter, sequence, target and rendering translations retain D2's MPL-2.0
terms, Copyright 2022 Terrastruct Inc. Rust translation and modifications are
Copyright 2026 Nelson Chen.

Dagro v0.2.1 derives from Dagre 3.1.1 and Graphlib. Translated ranking,
compound ordering and Brandes–Köpf portions retain the **MIT License**,
Copyright (c) 2012–2014 Chris Pettitt and Copyright (c) 2026 Dagro contributors.
The full applicable permission/disclaimer is preserved in
[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md#dagro-dagre-and-graphlib) and
the Dagre crate's [notices](../crates/d2-rs-dagre/THIRD_PARTY_NOTICES.md).
Those notices remain part of distributions; first-party MPL-2.0 declarations
do not replace the MIT terms on the translated dependency material.
