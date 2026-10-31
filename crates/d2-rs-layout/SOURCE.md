# Corresponding source

Run 84 (2026-10-25), master `6a7b48378`: `best_grid_cuts` follows
`d2layouts/d2grid/layout.go:563–716` threshold relaxation until a candidate
exists, including continued passes after a bounded traversal stops. The
arbitrary equal-count fallback is removed. `grid_cuts_distance` follows
`layout.go:806–831` / `grid_search.go:106–117`: sum each size plus its gap
left to right, then subtract the trailing gap; row acceptance retains its
separate size-sum-plus-gaps order. Original fractional/integral search tests
provide 443 complete ordered partitions, including five negative witnesses.
`grid_search_api_tests` replays input-only requests; the five prior failures
are normal regression tests. The nine-object CLI witness is
`tests/regressions/e2e/grid-search-threshold.d2`. All three native engines
now produce the pinned Go SVG for it. No cache/allocation contract is inferred
from these partition results. D2-derived changes retain MPL-2.0.

Run 83 (2026-10-25), master `6a7b48378`: `grid_dimensions` follows
`d2layouts/d2grid/grid_diagram.go:31–131` admission before multiplication,
allocation and checked expansion. `grid_storage_dimensions` follows
`d2layouts/d2grid/layout.go:325–337` occupied extents; even layout retains
configured empty-cell gaps while allocating only occupied row/column storage.
`grid_capacity_api_tests` replays input-only calls from the two original
`resource_limits_test.go` functions, including full errors, constructor and
occupied dimensions and the exact 1000×1000 sparse-grid content size.
Positive typed dimensions are covered; Go string parsing is outside this
native Graph projection. D2-derived changes retain MPL-2.0.

Run 82 (2026-10-25), master `6a7b48378`: all actual Dagre core calls,
including extracted grid/sequence parents, now forward their caller context
to d2dagrelayout's admission (`layout.go:48–63`). Compiled empty boards and
specialized scopes still use the source LayoutNested/compileBoard boundaries.
`admission_api_tests` is a test-only, input-only replay of the four original
engine admission calls and eight original shared-guard functions (14 calls).

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

Fixed flat grid placement follows pinned D2 v0.9.0
`d2layouts/d2grid/layout.go:Layout` and `d2graph/d2graph.go:SizeToContent`.
The native flat entry point delegates to the shared grid scope sizing, label
spacing, shape insets and fixed-dimension centering, and retains direct grid
edge routing. The 18 pristine graph float-bit controls and 18 end-to-end PNG
comparisons are recorded in `.reference/grid-dimensions-20261006/`; this does
not qualify all nested grid, shape or TALA behavior. D2-derived changes remain
covered by MPL-2.0.

Nongrid cylinder carriers in the shared D2 TALA adapter follow original
`d2layouts/d2talalayout/adapter.go` and `lib/shape/shape_cylinder.go`: the
TALA shape fitter owns the three 24px arcs, so compiler arc insets are not
applied a second time. Grid cylinders retain their separate grid fitting path.
Checkpoint 490 evidence in `.reference/lion-target-20261007/` covers all 40
authored object-box controls plus 26 upstream boxes and 16 routes; remaining
Markdown paint differences are explicitly retained. D2-derived changes remain
covered by MPL-2.0.

Checkpoint502 maps original compileEdge reference ScopeMap/BoardIDA,
IsSequenceDiagramGroup/Note and ExtractSubgraph root ownership to native
declaration-scope metadata, group/note classification and actor grid projection.
Six complete Source group inventories/default fills match;495 affected tests
and25 raster-harness tests pass. Broad149/159 remains unchanged without losses;
nesting_power improves448106→145138pixels but remains an exact failure. The
focused sequence CLI retains all7 exact failures, including a Source error
Native does not yet reproduce. Canonical actor identities/proxy elimination,
full graph/target/routes/errors/pixels and wider PLAN gates remain open.
Evidence: .reference/sequence-reference-groups-20261007/source-comparison.md
and qualification.json. No full-D2 completion claim.

Checkpoint505 maps original d2layouts/d2sequence/sequence_diagram.go placeGroups/adjustGroupLabel to deepest-first adjustment, crossing group/span height extension, minimum endpoint Y for moved messages, and movement of only spans/groups/notes. The permanent sequence-group-label-adjustment-source-v090.json qualifies13complete Source boxes,7message routes and6synthetic lifeline identities/routes from3measured-boundary controls. All91layout tests and25harness tests pass. Fresh broad149/159has no regressions; focused sequence2/7retains the two earlier exact matches and nested difference65150→5208. Renderer group/span role classification, global paint ordering, stale-empty-label HasLabel semantics, error and broader API/sequence/TALA parity remain open. Evidence: .reference/sequence-group-label-adjustment-20261007/source-comparison.md and qualification.json. Plugins are excluded from the active full-original-v0.9.0 scope.

Checkpoint506 maps original d2sequence.adjustGroupLabel HasLabel guard to explicit cleared-label metadata before stale dimensions can reserve a title band. An original-Go private helper overlay confirms all four boxes and both route points remain unchanged; sequence-empty-label-source-v090.json binds the Native regression to that complete state. All92layout tests pass0ignored, retaining checkpoint505 complete geometry controls. Missing label metadata/general direct API callers and message/note placement remain unqualified. Evidence: .reference/sequence-group-render-roles-20261007/source-comparison.md and qualification.json. D2-derived changes retain MPL-2.0 notices; the near-full goal remains active, bundled TALA included and plugins excluded.

Checkpoint507 maps original d2sequence verticalIndices/routeMessages/placeNotes to source-line comparisons and additive consumed-height arithmetic, including self-message height and strict same-line ties. Shared projection and both sequence edge-filter paths remap line keys alongside arena IDs. Source lines, not node IDs or message enumeration, reserve note space. All92layout tests retain their contracts, with the complete measured note-between regression checking four boxes, two message routes and two lifeline identities/routes alongside the three previous geometry controls; integrated Source/official/native controls are recorded in .reference/sequence-note-source-order-20261007/qualification.json. Complete actor/group ordering, transient extraction and general direct APIs remain unqualified. D2-derived changes retain MPL-2.0 notices; existing Go sort translation retains its BSD-3-Clause notices.

Checkpoint508 maps original d2graph.IsSequenceDiagramNote/Group into note classification from edge-expression references and contained object/edge scopes, rather than endpoint incidence. Object-scope ownership can qualify a group without messages; complete placement and invalid-position errors are separate outstanding contracts. Shared projection remaps object scopes and explicit edge-reference state. All92layout tests pass; full55object classification and CLI evidence are recorded in .reference/sequence-reference-note-fill-20261007/qualification.json. Legacy reference-less measured graphs/general APIs remain unqualified. D2-derived changes retain MPL-2.0 notices.

Checkpoint509 maps original newSequenceDiagram source-line sorting and addLifelineEdges last-message selection into fixed_lifeline_routes. It sorts the same filtered messages by earliest eligible declaration line before choosing the final complete route, preserving Source last-message semantics rather than using every route maximum. Six original-Go target inventories qualify33boxes,29message routes and29lifeline identities/routes; the measured regression retains the previous four controls. All92layout tests pass. Evidence: .reference/sequence-lifeline-source-order-20261007/source-comparison.md and qualification.json. Full invalid-position errors, variable spans, arbitrary ordering/transient extraction/API contracts and wider PLAN gates remain open. D2-derived changes retain MPL-2.0 notices; existing Go sort translation retains BSD-3-Clause notices.

Checkpoint510 maps original d2sequence.placeSpans/placeGroup and d2layouts.validateObjectPositions to source first/last endpoint selection, direct child bounds, non-glob note-scope ownership, preservation of unbounded boxes and final nested validation before scope reinjection. Nested sequence subtrees are removed from the enclosing span/note inventory following original LayoutNested extraction. Permanent Source fixtures qualify four accepted variable-control boxes/two messages/two synthetic lifelines, eight complete measured empty group/span boxes, three Source error inventories and the actual private infinity/NaN/Go-format validator controls. All94layout tests pass; full qualification is .reference/sequence-infinite-positions-20261007/qualification.json. Arbitrary ordering, missing-metadata direct APIs, other extraction/errors and broader PLAN gates remain open. D2-derived changes retain MPL-2.0 notices.

Checkpoint511: Temporary projections remap declaration/level provenance and routing carries committed fonts as explicit values. Six original-Go controls qualify all32adapter font/level/cached-label records;95layout tests pass. General reference-less APIs and arbitrary reinjection remain open. Evidence: .reference/sequence-extracted-grid-20261007/source-comparison.md and qualification.json. Full text/style/routing, structured-header and broader PLAN gates remain open. Existing MPL-2.0 and third-party notices are preserved.

Checkpoint605: Original d2graph Connect/initIndex/IsEquivalent/AbsID and d2compiler Compile sorting map to native retained edge indices, graph serialization/projection and WASM source_index/abs_id metadata. Forty repeated source controls and the native/browser API regression fixtures cover the stated identities; complete graph/API parity remains open. See .reference/compiled-edge-index-native-20261009/source-comparison.md and qualification.json. D2-derived changes retain MPL-2.0 notices.

Nested-grid crossing callbacks (2026-10-22 run 4) follow original D2 v0.9.0
`d2layouts/d2layouts.go:330–382`: restore external edges after InjectNested,
then use DefaultRouter for Dagre/ELK, including its edge LabelPosition commit.
The root-grid and ELK object-grid paths now retain that callback at every
nested grid. Reduced witness: tests/regressions/e2e/nested-grid-crossing-label.d2;
complete SVG agrees with pinned Go for all three engines. Scoreboard +3 / 0.

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

Nested-grid routing order (2026-10-23 run 14) follows original
`d2layouts/d2layouts.go:240–246,315–354`: each recursive LayoutNested finishes
child crossing routes and labels before the parent restores its crossings.
TALA now shares the postorder grid traversal used by Dagre/ELK, with any
implicit root last. This preserves existing labels as obstacles for
`d2talalayout/internal/labeling/placement.go:496–547`, rather than reserving
unset child labels and panicking. Target: TALA `real_world/jupyter_aws_eks`,
runtime failure → exact SVG; full scoreboard +1 / 0 regressions.
Witness: `tests/regressions/e2e/tala-nested-grid-routing-order.d2` (complete
normalized SVG equal to pinned Go, target hash 1710228742). MPL-2.0 applies.

## Empty detached-grid label state (2026-10-23 run 16)

Original `d2layouts/d2grid/layout.go:31–35` sets a grid-root label default
only when `HasLabel`; `d2layouts/d2layouts.go:748–753` injects that detached
root state after parent layout. `grid::effective_grid_positions` now discards
provisional parent-engine label defaults on an empty grid root, retaining
`Unset` (Go nil) alongside the root's inside-top-left icon. Reduced from
failing Dagre `stable/grid_icon`: `tests/regressions/e2e/grid-empty-label-icon.d2`,
target 3546686747 → pinned-Go 2599487348, complete normalized SVG equality.
Full upstream delta: +1 exact SVG / zero regressions.

## Master grid measurement observations (2026-10-25, I58)

Pinned D2 master `6a7b48378`, `d2layouts/d2grid/grid_search.go:88–99`
and `layout.go:631–666,811–829`, map to `grid_row_size`,
`grid_row_with_gap` and `grid_cuts_distance`. The last helper's existing
left-to-right gap sum is shared with the input-only native replay; arithmetic
and search order are unchanged. All 110,596 original measurements across
116 sessions in the three `grid_integral_test.go` measurement functions match
complete interval bounds and IEEE-754 bits, including NaN payloads and signed
zero. Original Go cache/prefix assertions remain active, but the native
projection qualifies numerical results only, not Go's private cache layout.
The existing ordered-search corpus remains 443/443. No numerical fix or new
golden match is claimed. Capture/replay: `scripts/grid-measurement-scoreboard.py`;
source hook: `tools/grid-measurement-scoreboard/hook_test.go`; evidence:
`.reference/codex-runs/iso58/measurement.json`. Existing MPL-2.0 notices apply.
