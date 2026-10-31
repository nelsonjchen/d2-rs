# Corresponding source

Lane L85 (2026-10-26), performance phase: graph property scalar/presence/range
queries use the core's source-ordered borrowed unquoted-path lookup. The same
field, scalar and range identities reach the existing lowering logic; arithmetic,
class precedence and output order are unchanged. This removes temporary
`raw_string_box` allocations at the `property_scalar_inherited` hotspot from
the supervisor's `real_world/spyre_encoder` profile. The standalone
`frontend-profile` example reports parse/IR/graph-JSON stage times and verifies
equal results across iterations; it adds no profiling to production builds.

## Paired board-link measurement (Lane L15)

Master `d2graph/d2graph.go:1644–1653` adds 64 px of horizontal shape padding
when both Link and Tooltip survive compilation. The former HTTP(S)-host
predicate in `compiler/graph.rs` incorrectly omitted internal board links.
Measurement now uses the same `node_link_after_board_validation` result as
the native target, following `d2compiler/compile.go:1726–1762`; unresolved
links remain absent. Authored width and existing shape exclusions still
apply. The four named failing artifacts are `pdf-paged-file`,
`pdf-paged-stdout`, `pptx-paged-file`, `pptx-paged-stdout` from Lane F run59's
Architecture corpus; `lang15-witness/internal-link-tooltip.d2` is minimal.

`tests/graph_lowering.rs` exercises actual shape fitting for the paired link,
class inheritance, missing link and authored-width cases. Its older nested
Markdown expectation is corrected against master `d2ir/compile.go:798–835`
and the original Go-rendered `lang15-witness/markdown-scope.d2`: master
collectVariables exposes both `nested.x` and the nested alias `x`.

Lane L14 (2026-10-25), master `6a7b48378`: the compiler's grid validation
now shares `d2_rs_core::grid::grid_capacity` (`d2graph/grid_diagram.go:5–32`)
without changing diagnostic ranges or admission order. The test-only adapter
also captures `d2ir/index_test.go:98–112` through `construct_fields` and
`substitution_cycle_test.go:5–16` through the existing visited-node traversal
(`d2ir/compile.go:747–800`). Cyclic IR requests retain node identities and
edges without JSON recursion; complete field results retain names/ranges.
`master-small.json` preserves all 42 original API results across four test
functions, with every Go assertion active. No ordinary build includes adapters.

Lane L13 (2026-10-25), master `6a7b48378`: optional-map copy admission and
the public `Compiler::reserve_variable_expansion_aliases` follow
`d2ir/expansion.go:163–265`. A missing map is rejected before context polls.
Alias reservations share the compilation budget, charge repeated logical
nodes and distinct scalars sharing mutable AST values, preserve earlier
charges, and propagate typed context causes without changing IR.
`edge_expansion.rs` factors the existing pair/work counters into the raw
reservation boundaries from `d2ir/edge_expansion.go:80–106`; compiler errors
still acquire their source location at the original caller boundary.
The test-only `master_internal.rs` adapter reaches the production primitives
and direct array/edge routines. Its seven original functions are captured
with every Go assertion active. Budget results compare exactly, including
every reservation error; only identical requests use lossless run encoding.
The three cancellation functions compare their original partial-progress and
existential predicates, retaining complete raw states separately. They do
not assert equal poll schedules or partial IR. No adapter/public test
serialization is compiled into ordinary builds.

Lane L12 (2026-10-25), master `6a7b48378`: the native post-Compile
`Compiler::reserve_variable_expansion_copy` follows
`d2ir/expansion.go:180–265`, sharing the original compilation
budget with substitutions and graph lowering. `expansion.rs` owns the shared
per-node reservation and source-ordered copy walk. Failed reservations retain
earlier successful charges; context causes remain typed and IR is untouched.
Read-only usage and node-cost methods expose the same existing accounting
(`expansion.go:267–306`), without changing the units. The trace-only hook
observes the original cumulative-copy, slice-unit and imported-spread tests;
all assertions remain active and the permanent fixture retains every complete
result, including exact counters. This finite capture closes two previously
unobserved post-Compile assertions; it does not qualify every IR budget or
context operation.

Lane L11 (2026-10-25), master `6a7b48378`: graph materialization follows
`d2compiler/compile.go:113–225` and `d2ir/expansion.go:180–265`.
`compiler/graph_materialization.rs` charges every logical IR occurrence before
automatic board copies. `source_graph.rs` shares the original compilation's
budget across nested boards, folder-comparison copies and repeated API calls;
read-only graph views share its counter rather than resetting it. Immutable
arena views preserve source IR and its lexical board identity, while charging
the source's logical copy cost. `graph.rs` propagates the
graph context through board/class/property views and checks it during field,
object and edge materialization. Canceled/deadline operations return typed
graph errors and publish no partial graph. This qualifies the original three
unused-alias board cases and graph cancellation test, not all context polling
or performance contracts. The trace-only hook captures the actual shared
composite identities as source paths and all original full graph/error results.

Lane L10 (2026-10-25), master `6a7b48378`: `compiler/class_index.rs`
translates `d2compiler/compile.go:570–651` (`getClassMap` / `foldClassName`).
One source-ordered index per root keeps the first match, including nil maps,
omits quoted reserved names and uses Go Unicode simple-fold equivalence.
Construction checks context before each member and before publication;
cancellation/deadlines publish no partial index and preserve the typed cause.
`ClassLookup` borrows immutable IR for the session; graph class resolution
uses the same implementation in board-local lowering views. Property-overlay
views share their containing graph session's index. The core exposes its
existing generated fold key rather than duplicating Unicode tables.

The original `TestClassLookupBuildsOneReusableIndex` and
`TestClassLookupIndexConstructionIsCancelable` remain assertion-active in the
trace checkout's env-gated capture. The scoreboard compares all **8,197**
lookup results (complete IR maps/references), retained-root count, context
checks and typed/exact errors. `tests/fixtures/master-class-index.json` is
losslessly dictionary-encoded; tests restore every original result before
comparing. Whole graph materialization cancellation/budgets and performance
complexity beyond these contracts remain unqualified.

Lane L9 (2026-10-25), master `6a7b48378`: `compiler/substitutions.rs`
forwards the active context into the Markdown replacement API and retains
the typed cancellation/deadline cause before halting (`d2ir/compile.go:605–622`).
The text crate maps the checked matcher and traversal; see d2-rs-text SOURCE.md
and its original 13-call master fixture. Other internal compiler contexts
remain independently unqualified.

Lane L8 (2026-10-25), master `6a7b48378`: `compiler/import_compile.rs`
`load_import_ast` follows `d2ir/import.go:226–265`, calling the core's
context-aware AST cloner after reserving the complete AST-copy cost. It keeps
the original typed cause, halts compilation and never returns a partial tree.
The cloner maps `d2ir/import_cache.go:13–382`; see d2-rs-core SOURCE.md.
`master-ast-clone-scoreboard` replays the original two cancellation tests,
preserving every threshold, complete AST and typed/exact errors. The 65-call
fixture derives from env-gated observation of the original tests, whose
assertions all pass. Imported parsing still uses the existing loader and is
not newly qualified by these clone tests.

Lane L7 (2026-10-25), master `6a7b48378`: `compiler/imports.rs`
maps `d2ir/import.go:17–19,21–55`'s 128-file active import-chain bound.
The root source is excluded. Path validation/resolution precedes the depth
check; the depth diagnostic precedes cyclic-chain detection, and rejection
does not mutate the stack. This is independent of total imports and the
compilation-wide copy/work budgets.

`tools/language-master-scoreboard/ir_import_hook_test.go` observes the original
public calls in `d2ir/import_depth_test.go` and the two master-added functions
in `d2ir/import_test.go`, retaining every assertion (including the deep-chain
subprocess). The cancel-on-open filesystem is described without advancing
its context: only its backing MapFS is read to capture inputs. The native
adapter uses the same filesystem-triggered context policy and compares typed
cancellation, full IR/imports and exact errors. These five original requests
and results are retained in `tests/fixtures/master-import-depth.json`.
The call tier does not qualify context checks during imported parsing/copying
or internal clone cancellation; those remain open.

Lane L6 (2026-10-25), master `6a7b48378`: parser bounds and context follow
`d2parser/parse.go:22–24,48–101,220–269,443–460,1601–1617`.
`parser.rs` checks the shared 128-level map/array nesting budget (the file
map is excluded), latches the halt, and suppresses subsequent termination
errors. `parser/containers.rs` retains the source's partial nodes and distinct
map/array end positions while unwinding. Context checks precede readahead
and occur on the first rune attempt and every 256 attempts thereafter.
`parse_with_context` / `parse_stream_with_context` return the typed original
cause ahead of accumulated syntax diagnostics; context-free APIs retain their
existing result type. Streaming BOM/buffer initialization still precedes rune
checks, matching Go. This parser context is separate from compiler contexts.

`tools/language-master-scoreboard/parser_hook_test.go` observes the original
three master-added parser functions, including both depth-overflow subtests.
The four complete partial AST/error results remain in
`.reference/codex-runs/lang6-parser-capture`; the scoreboard compares every
JSON field with the unchanged type-aware language comparator. Go assertions
remain active, and the pre-existing golden hook is preserved exactly.
The two overflow calls improve from mismatches to matches, the original
pre-canceled call gains a native adapter, and the depth-boundary call remains
exact (1 → 4/4). Native tests also cover mixed nesting, large inputs, typed
deadlines, latched cancellation and retained diagnostic accumulators through
both input boundaries. Arbitrary parser contexts, import-parser propagation
and graph-materialization contexts remain outside this finite original-test
measurement.

Lane L5 (2026-10-25), master `6a7b48378`: `CompileContext` and the additive
`compile_ir_with_context` entry point map the context checks and error
precedence in `d2ir/compile.go:108–195` and `d2ir/expansion.go:56–80`.
`Compiler::stopped` now checks the caller's context at the existing work-stop
gates in construction, arrays, imports, fields/edges, lazy replay and
expansion. It latches the original typed cancellation/deadline cause and
publishes its bare diagnostic ahead of collected source errors; the original
context-free entry points keep their options and defaults. Partial IR remains
diagnostic state and cannot be treated as a successful result when errors
are present. Parsing and graph materialization are separate API stages.
The exact original `TestCompileCancellationDuringIRConstruction` request
and result are retained in `tests/fixtures/master-ir-cancellation.json`.
The trace-only adapter records the original context's threshold and initial
check count without calling Err or changing its state. All original Go
assertions remain intact. The new edge suite captures 11 full IR/import/error
calls from eight original functions in `d2ir/edge_expansion_test.go`; all
already match before this port. Its two primitive and two internal
cancellation tests remain explicitly uncovered. Internal array/AST-clone
cancellation, parser and graph contexts still need dedicated adapters;
this does not qualify all compiler cancellation points or exact poll counts.

Lane L4 (2026-10-25), master `6a7b48378`: import work accounting maps
`d2ir/import.go:92–163,191–265` into `compiler/import_compile.rs` and
`imports.rs`. Cached parsed ASTs are reserved before cloning; the same
compilation budget charges every peek, cached selected-field/map copy and
new IR template. `expansion.rs` counts each AST node and copied slice/string
member following `d2ir/expansion.go:111–125,318–380`. Imported array spreads
reserve their logical members before append (`compile.go:2339–2342`).
Imported substitution errors are restored before the stopped check; the
compiler retains the expansion error independently for the fallback at
`compile.go:202–214`. No path opens, tree clones or array appends occur after
a failed reservation. Public ImportState loaders still return independent
copies. The original `TestImportedVariableExpansionUsesCompilationBudget`
and `TestRepeatedImportPeeksUseCompilationBudget` calls now match complete
IR/import/error results. Five original import calls are permanent fixtures
in `tests/fixtures/master-import-expansion.json`; a native unit also retains
the original spread-versus-nonspread 21-unit accounting assertion.
The variable-suite adapter captures 17 calls from 11 of the 16 original
`d2ir/expansion_test.go` functions, preserving all Go assertions. Cancellation,
the five internal-only functions and two post-Compile assertions remain
explicitly unqualified by the call scoreboard.

Lane L3 (2026-10-25), master `6a7b48378`: `compiler/glob_expansion.rs`
maps `d2ir/glob_expansion.go:9–135`: 65,536 work units, an independent
10,000-created-field limit, and prospective path-depth reservations per
active glob. `scheduler.rs` charges keys before duplicate-context rejection
(`d2ir/compile.go:1161–1184`); `fields.rs` reserves candidate visits and
materialization at `d2ir/d2ir.go:1216–1222,1257–1309,1399–1445`.
Placeholder and array reservations follow `compile.go:1043–1049,2284–2304`.
`CompileLimits::max_glob_expansion` defaults and negative-option precedence
follow `compile.go:120–136`. Exhaustion halts traversal and emits one exact
source diagnostic. Existing variable/edge budgets remain independent.
`scripts/language-ir-master-scoreboard.py` preserves the original assertions
in `d2ir/glob_expansion_test.go` through an env-gated trace-only hook.
It compares full IR, imports and errors for 14 original calls from ten test
functions, including the 2,000-field control; the primitive budget test has
a separate native unit and remains explicitly uncovered by that call tier.
Thirteen compact complete results are in `master-glob-expansion.json`;
the larger passing control remains in `lang3-ir-capture`. Both native runners
share the existing complete IR JSON projection in `examples/support/ir_json.rs`.
This closes the public compiler-call tier at 70/70; cancellation, import/copy
budgets and the broader uncovered test inventory remain open.

Lane L2 (2026-10-25), master `6a7b48378`: grid dimension validation maps
`d2compiler/compile.go:164,217–247,1041–1076` and
`d2graph/grid_diagram.go:6–29` into `arrowhead_validation.rs` and
`late_validation.rs`, retaining the later attribute key for capacity errors.
`edge_expansion.rs` maps `d2ir/edge_expansion.go:9–132`; reservations in
`edge_creation.rs` precede self/container/board filtering at
`d2ir/d2ir.go:1652,1797`. Distinct pairs use glob/AST-segment/field identities
and the selector bit; repeated lazy visits still consume examination work.
`CompileLimits` exposes both edge budgets with the upstream defaults.
`config.rs` maps `d2ir/compile.go:381` and
`d2compiler/compile.go:1838–1842` for source isometric config values.
Ordinary unsafe links use the shared text predicate at
`d2compiler/compile.go:1020–1023`; source edge records promote absent labels
from links as at `compile.go:1222–1224`. Imports without an opener now report
`d2ir/import.go:239–240`'s explicit disabled-import diagnostic at the import
node, rather than failing unsupported-language preflight.
Gradient stop admission maps `lib/color/gradient.go:105–143` into
`style_colors.rs`: one or two fields and finite numeric positions.
The `master-compiler-bounds.json` fixture retains 34 complete original Go
compiler-call results alongside L1's 35 variable contracts. Color and edge-key
witness fixtures were independently rerun against master with env-gated hooks
in `tools/language-master-scoreboard/`; their v0.9.0 negatives remain intact.
The remaining glob-work call and the uncovered internal/context APIs do not
pass this compiler-call tier.

Lane L master inline-test work (2026-10-25) maps
`compiler/expansion.rs` to pinned master `6a7b48378`'s
`d2ir/expansion.go:14–380` (work units, repeated aliases and scalar identities),
`compiler/substitutions.rs` to `d2ir/compile.go:270–353,416–848`
(substitution visits, cycle rejection, scalar/array/map reservations, bounded
Markdown variable collection), and `compiler/boards.rs` to
`d2ir/compile.go:216–260,897–916` (class and inherited-board copies).
The additive `compile_ir_with_limits` API retains existing `CompileOptions`
initializers and defaults to the same 65,536-unit substitution budget.
`tests/fixtures/master-variable-expansion.json` retains original inputs and
complete diagnostics from the master compiler tests, dumped only in the
trace checkout by `scripts/language-master-scoreboard.py`. This is finite
coverage; context cancellation, import/AST-copy budgets, graph-materialization
copy accounting, and glob budgets still require their own test adapters
and implementation work.

This is an unpublished development checkout of d2-rs. There is not yet a public
release source URL. Keep this complete checkout, its Git revision and all local
modifications alongside any locally shared binary or WASM artifact. Do not
substitute the archived Weftan repository for the source of d2-rs modifications.

The upstream source and historical Rust input are pinned in
reference-manifest.json. Development references can be reconstructed with
scripts/references.py and docs/REFERENCES.md. Instrumentation patches are tracked
under tools/patches; ignored references must not contain undisclosed changes.

`compiler/graph/source_objects.rs::object` removes a rejected object's link
from the compiler source record as well as the render style. This maps
`d2compiler/compile.go:1334–1374` (`validateBoardLinks`): missing boards,
relative non-board keys and self links are cleared on the original object.
The language scoreboard fixes six complete compiler goldens with this change;
the reduced source-record regression is `x.link: layers.missing`.

`compiler/boards.rs::resolve_board_target` follows pinned
`d2target/d2target.go:108–169` (`Diagram.GetBoard`): optional wrappers,
layers/scenarios/steps search order, terminal current-board names, and
board-only traversal. Missing targets use the joined parsed components as in
`d2cli/main.go:538–540`. This closes the CLI shorthand-target cluster without
allowing ordinary object maps to become render targets.

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

Position-holder validation in compiler/arrowhead_validation.rs maps original
Go D2 v0.9.0 d2compiler/compile.go compileReserved/compilePosition and its class,
board, edge/arrowhead visitation/error deduplication. Label/icon/tooltip maps
validate only unquoted near values, preserve each original position domain,
null resets, array acceptance, synthetic-primary-key guards and source order.
A 567-result pristine fixture covers successful compiles and complete diagnostic
strings, including Unicode/UTF-16 ranges. Full reserved scalar guards and valid
position metadata/rendering remain separately qualified requirements.

## Edge/arrowhead key and reserved value guards (checkpoint481)

Pinned `d2compiler/compile.go` compileField/compileEdgeMap/compileEdgeField/
compileArrowheads/compileReserved: native validation now admits only Source's
unquoted keyword sets, with first-reference edge diagnostics and last-reference
arrowhead/style diagnostics. Class-definition checks run in Source field order;
object arrowheads are rejected. Reserved bare/composite values, scalar shape,
direction and signed int64 dimensions/gaps follow Source syntax and range guards.

The `edge-key-validation-source-master.json` oracle fixture (recipe
`lang-edge-key-validation`, generated by scripts/edge_key_validation_cases.py;
the v0.9.0 capture was retired) checks 1,362 complete compile outcomes, including quoted keys, classes/boards, overrides, Unicode/
UTF-16 and integer extremes. Earlier failures and the incorrect inherited
label-percentage acceptance test's original source are retained. The test now
checks Go's rejection of unsupported explicit fields and retains valid defaults.
General URL/Markdown/security guards, graph metadata/rendering and full language/
API/layout/TALA/resources/cancellation/native/WASM/platform parity remain open.

Authored label-position provenance in compiler/graph.rs follows original
Go d2compiler.compilePosition -> d2graph.SetDimensions ->
d2talalayout/adapter.go's Label.Position assignment and internal/labeling's
PositionFixed guard. Automatic external-label carriers are admitted only for
Unset positions; authored inside/border positions retain fixed provenance.
The raw 421 original-Go position outcomes live in
crates/d2-rs-cli/tests/fixtures/authored-label-position-source-v090.json;
checkpoint483 evidence is .reference/glob-dimensions-20261006/.

Positioned-tooltip metadata handoff (checkpoint486) maps original Go
compilePosition/compileField and exporter.toShape. Native SourceNodeStyle now
retains tooltip.near independently of tooltip text and the target emits it.
Quoted reserved keywords remain ordinary graph fields; board recognition uses
actual unquoted wrapper fields, preserving original quote-insensitive core IR
NodeBoardKind solely as navigation. 227 raw Source compile/export controls and
native regressions compare complete ordered metadata arrays/full diagnostics.
Source SVG controls for480inputs are captured and replayed; native positioned
Markdown box/tail/bounds/font/error parity remains open. Evidence and scope:
.reference/positioned-tooltip-20261006/source-comparison.md.

Checkpoint494 integrates original d2compiler/compile.go compileLabel/compileReserved tooltip raw Markdown validation before label transforms and positional validation. Object/edge primary labels, applied classes, boards and legend scopes retain source traversal/LastPrimaryKey diagnostics/deduplication; object vars bypass compileReserved and later scalar errors remain collected. All38 complete Source diagnostic/acceptance controls match, and106 full language tests pass. Evidence: .reference/markdown-compiler-xml-20261007/source-comparison.md. Broader graph/API/diagnostic parity remains open.

Checkpoint495 retains original root styles/class inheritance for exporter and canvas consumers, excludes unquoted class assignments from graph objects, and follows direct root 3d/double-border style presence/shape/LastPrimaryKey validation in d2compiler.validateKeys. The121 original-Go root metadata/bounds/error controls include direct versus inherited validation and selected child boards. Broader mixed compiler-context errors remain open. Evidence: .reference/root-canvas-style-20261007/source-comparison.md.

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

Checkpoint503 maps original d2graph/d2graph.go Object.EnsureChild and OuterSequenceDiagram plus d2compiler/compile.go compileField/compileMap to ordered canonical graph-field identity and private arena attribute merging. Fourteen original-Go pre-layout controls qualify complete sets of65objects, canonical parents, labels, declared fill/stroke and30edge endpoint/scope records; original lexical IR remains unchanged. Evidence: .reference/sequence-canonical-identity-20261007/source-comparison.md and qualification.json. Ordering, all other graph attributes and full target/API parity remain unqualified. Fresh broad149/159 has no regressions; focused sequence1/7 now includes the exact endpoint control. Plugins remain outside scope.

Checkpoint507 maps original d2compiler.compileField/compileEdge reference contexts and d2sequence.getObjEarliestLineNum/getEdgeEarliestLineNum to complete reference minima. Canonical aliases retain the minimum eligible line; object key-path globs and edge endpoint globs are skipped independently. Ten actual original-Go helper controls qualify35objects and16edges, including repeated labels, same-line statements, glob paths and nested scope; sequence-declaration-lines-source-v090.json binds every inventory. Broader reference/ordering/API contracts remain open. Evidence: .reference/sequence-note-source-order-20261007/source-comparison.md and qualification.json. D2-derived changes retain MPL-2.0 notices.

Checkpoint508 maps original compileField reference MapKey.Edges and ScopeMap/BoardIDA/parent ScopeObj into canonical object edge-reference state and scope ownership. Graph lowering preserves AST quoting/pattern provenance when rejecting unresolved endpoint paths; quoted literal "*" is a concrete ID. Seventeen actual original-compiler controls qualify55object reference/predicate records and one exact diagnostic; complete reference serialization and other graph attributes remain open. Evidence: .reference/sequence-reference-note-fill-20261007/source-comparison.md and sequence-note-references-source-v090.json. D2-derived changes retain MPL-2.0 notices.

Checkpoint510 retains non-glob original Object reference Key/ScopeObj ancestry separately for d2sequence.placeGroup note bounds. General containment metadata remains unfiltered; canonical aliases and projections retain the appropriate ownership. All108language tests pass, including integration controls. Source comparison and strict CLI qualification: .reference/sequence-infinite-positions-20261007/source-comparison.md and qualification.json. Complete reference APIs/arbitrary compilation ordering remain unqualified. D2-derived changes retain MPL-2.0 notices.

Checkpoint511: Lowering records explicit font declaration presence separately from measured font sizes and derives original label levels from canonical parent links, including quoted identifiers. Projection remaps both maps. All108language tests pass; complete32original adapter records and new strict CLI controls qualify the stated extracted ordinary-text contract. Evidence: .reference/sequence-extracted-grid-20261007/source-comparison.md and qualification.json. Full text/style/routing, structured-header and broader PLAN gates remain open. Existing MPL-2.0 and third-party notices are preserved.

Checkpoint545 (2026-10-08): Original d2parser splitLeadingIndent and d2graph Attributes.ApplyTextTransform map to retained raw node-label graph bytes. All64 source byte/error controls are permanent in raw-label-bytes-source-v090.json. Existing full Unicode title/case semantics and other label owners remain unqualified. Evidence: .reference/raw-label-native-20261008/source-comparison.md and progress-bindings.json.

Checkpoint546 (2026-10-08): Original d2compiler compileLabel/compileReserved wraps raw rendered Markdown bytes for XML validation before transforms. Native compiler now uses render_html_bytes/validate_markdown_html_bytes;160 CLI label controls match, recovering32 Markdown diagnostics after545. This does not qualify all byte owners/graph APIs. Evidence: .reference/markdown-bytes-native-20261008/source-comparison.md and api-qualification.json.

Checkpoint605: Original d2graph Connect/initIndex/IsEquivalent/AbsID and d2compiler Compile sorting map to native retained edge indices, graph serialization/projection and WASM source_index/abs_id metadata. Forty repeated source controls and the native/browser API regression fixtures cover the stated identities; complete graph/API parity remains open. See .reference/compiled-edge-index-native-20261009/source-comparison.md and qualification.json. D2-derived changes retain MPL-2.0 notices.

Checkpoint 875: Compile SQLColumn target text defaults/raw constraints, retain name/type dimensions from existing GetLabelSize measurements, and preserve original Go range-copy reference behavior. Source: pinned d2target/sqltable.go,
d2target/d2target.go::Text, d2compiler.compileSQLTable,
d2graph.Object.GetLabelSize/addSQLTableColumnIndices and d2exporter.toShape.

Checkpoint 876: Preserve cloud content-box aspect ratio bits at SizeToContent, before fitting/layout. Extend the existing Go net/url predicate parser with source scheme/host/noncanonical RawPath and D2 toPrettyLink byte truncation, retaining raw bytes through target serialization and Go-compatible presentation. Sources: d2graph.GetDefaultSize/SizeToContent, lib/shape/shape_cloud.go, d2exporter.toPrettyLink; Go1.27 net/url.setPath and gen_encoding_table.go (existing BSD notice).

Checkpoint 876: Carry compiled config data into the measured graph for target hashing. The pinned Go1.27 selects encoding/json v2 (selected files recorded); PrettyLink byte truncation retains per-byte Unicode replacement in hash strings, instead of v1 raw escape tokens.

Run 9 (2026-10-23): `compiler/source_graph.rs` projects the native compiler
result into the pre-SetDimensions d2graph JSON contract. Maps original
`d2compiler/compile.go:87–112,127–211,215–267`: board traversal, scalar boards,
BaseAST materialization (including preserved spread imports), and legend
boxes/routes. `find_board_base_ast` is shared with compiler-resolved editing.
Compiled source labels remain raw (`compileLabel`, lines 473–508); measured
labels retain `d2graph.Attributes.ApplyTextTransform`. Theme override errors
use LastPrimaryKey (lines 1552–1556). Config data belongs to the separate
Config returned by Compile (lines 63–67,1482), rather than the pre-measurement
graph. All nine run-9 compiler failures are retained as complete pinned
results in `tests/fixtures/compiler-source-graph-v090.json`. The language
scoreboard matches all 344 live compiler goldens; 31 historical compiler
goldens remain unbound. This does not qualify editor graph results or full
compiler/API compatibility.

Run 25 (2026-10-23): `compiler/graph.rs` applies the final AST object/edge
sort only to the document graph, following `d2compiler/compile.go:59–60`.
Child boards are compiled directly by `compileBoardsField` (line 199), so
they retain `compileMap` traversal order instead of receiving
`d2graph.SortObjectsByAST` / `SortEdgesByAST` (`d2graph.go:1816–1850`).
The `sequence-layer` and `sequence-spread-layer` CLI cases expose both the
message export order and the independent shape order in target hashes.

## Self-link validation follows Graph.IDA (2026-10-23, Claude)

`d2compiler.validateBoardLinks` (compile.go:1334–1375) drops a link whose
StringIDA equals `obj.Graph.IDA()`. `d2graph.Graph.IDA` (d2graph.go:94–151)
joins `root`, only the innermost collection name, then every board name, so
`layers.x.steps.next` is `root.steps.x.next` and a nested board's link to
itself survives. `go_graph_ida` reproduces that instead of comparing resolved
boards. Witness: upstream `TestAppendix/internal-links` (d2svg appendix tier
5/6 → 6/6); unit test `nested_board_self_links_survive_like_go_graph_ida`.

## Master library LaTeX admission (Lane L26)

`compiler/graph.rs::GraphMeasurer::measure_latex_label_checked_at_size`
preserves `MathError::InputLimit` as the exact diagnostic returned by
`d2renderers/d2latex/latex_common.go:10–30` at 6a7b48378: ValidateInput is
outside the MathJax parse wrapper. Both original object/edge nesting-limit
calls in `d2lib/latex_limits_test.go` are captured with active Go assertions
by `scripts/library-api-scoreboard.py`.

## Graph-session property indexes (Lane L86, master 6a7b48378)

`compiler/graph_fields.rs` accelerates `d2ir/d2ir.go:1106–1150` GetField
for the immutable graph stage. Each folded-key bucket preserves the first
field and first map field, separately for unquoted names: scalar duplicates
are skipped for longer paths, while a nested miss never retries a later map.
Reserved names use Go ToLower rather than EqualFold to determine quoting.
Lowercase ASCII queries borrow their key; other queries use the existing
pinned Unicode fold table. Hash iteration never determines output order.
A stack-scoped session restores its predecessor on early return or unwind;
public IR lookups, board changes and private canonical-property forks never
reuse these indexes. Scalar/presence/range/class/SQL property helpers query
the session; graph text reads the borrowed scalar presentation from core.
