# Corresponding source

Lane L85 (2026-10-26), performance phase: `ir::{equal_fold,fold_string}`
use the existing pinned Go 1.27 Unicode 17 canonical-fold table with an ASCII
fast path and the existing one-invalid-byte RuneError decoder. `StringBox`
exposes callback-scoped scalar bytes instead of copying shared AST storage;
interpolated scalar materialization now copies once. IR field lookup retains
`d2ir/d2ir.go:1106–1150` source order, live public field/name mutations,
reserved-keyword ToLower/quoting and scalar-duplicate traversal behavior.
`get_field_unquoted` accepts borrowed compiler property paths without building
temporary AST nodes. Compiler-owned field indexes retain their existing
mutation/winner rules; no public lookup cache is introduced.

Run 82 (2026-10-25), master `6a7b48378`: `layout_guard::{check,check_raw}`
map `d2layouts/internal/layoutguard/graph.go:12–93`. Cancellation precedes
raw object/edge caps (1,024), endpoint presence, the 256-edge fast path and
the 8,192 edge-interaction bound. The typed adapter and raw optional-endpoint
adapter share one implementation. Original eight test functions / 14 calls
compare complete errors; ordinary tests retain limit/cancellation precedence,
sparse graphs and density undiluted by unrelated objects. Nil graph admission
and arbitrary mid-flight context schedules are not qualified here.

Lane L14 (2026-10-25), master `6a7b48378`: `grid::grid_capacity` and
its axis/cell constants map `d2graph/grid_diagram.go:5–32`. Admission checks
positive axes, then per-axis limits, then an overflow-safe division before
multiplying. The compiler uses this same public API for capacity validation.
The original five `TestGridCapacityLimits` calls compare complete capacities
and error strings; nonpositive-axis precedence has a native contract too.

Lane L10 (2026-10-25), master `6a7b48378`: exports the existing
`ir::fold_string` canonical Unicode simple-fold key for the compiler's shared
class index (`d2compiler/compile.go:609–651`). Its generated Go 1.27.0 Unicode
17 table and IR field-key behavior are unchanged.

Lane L8 (2026-10-25), master `6a7b48378`: `ir/import_cache.rs` maps
`d2ir/import_cache.go:13–382` (`cloneASTMapContext`, `astCloner`) into
`clone_ast_with_context`. Checks precede node/box/slice work; numeric copies
also check after copying the rational. Errors discard the entire working AST,
including partial arrays, while leaving the immutable template intact. The
existing background `clone_ast` uses this same implementation. The two original
cached-import cancellation tests contribute 65 complete AST/error calls:
29 → 65 matches, no regressions. Original source fixture and ownership/cause
tests live in d2-rs-language `master_ast_clone`. Broader import parsing and
internal compiler cancellation APIs remain unqualified.

Lane L3 (2026-10-25), master `6a7b48378`: `ir/pattern.rs` maps
`d2ir/pattern.go:9–203` to visit-aware recursive glob traversal. Candidate
callbacks run before filters (including unnamed fields in full traversal),
and refusal stops the current branch and all enclosing traversals. Lazy
branch helpers skip unnamed fields before the callback, as upstream does.
Existing traversal APIs delegate to the same implementation with an accepting
visitor. Full triple traversal now guards unnamed substitution placeholders
as master's `_tripleGlob` does; the old v0.9.0 nil-name panic is obsolete.
Language tests/scoreboards retain the original master glob contracts,
including unresolved-placeholder diagnostics and work-boundary controls.

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

ARM64 shape-border Cramer arithmetic in shape_geometry.rs follows the pinned
Go1.27compiler's geo.IntersectionPoint FMULD/FMSUBD sequence. The first product
is rounded; the second product/subtraction is fused before the division.
Raw ray/segment/denominator/numerator/ratio/step bits and final port witness
are regenerated by recipe mathtext-core-badge-ray into the oracle-tier
tests/fixtures/badge-ray-source-v090-arm64.json. Its regression
is explicitly ARM64-specific; other-target Source execution remains open.
Probe/assembly/raw failed comparisons are .reference/badge-anchor-20261006/;
normal tracing remains unchanged and the probe compiles only as a test.

Checkpoint495 adds optional authored root-style graph metadata for original d2exporter.Export/applyStyles and buildRoot. It is independent of layout metadata and defaults to absent for existing callers. Scope/evidence: .reference/root-canvas-style-20261007/source-comparison.md.

Checkpoint503 maps original d2graph/d2graph.go Object.EnsureChild and OuterSequenceDiagram plus d2compiler/compile.go compileField/compileMap to ordered canonical graph-field identity and private arena attribute merging. Fourteen original-Go pre-layout controls qualify complete sets of65objects, canonical parents, labels, declared fill/stroke and30edge endpoint/scope records; original lexical IR remains unchanged. Evidence: .reference/sequence-canonical-identity-20261007/source-comparison.md and qualification.json. Ordering, all other graph attributes and full target/API parity remain unqualified. Fresh broad149/159 has no regressions; focused sequence1/7 now includes the exact endpoint control. Plugins remain outside scope.

Checkpoint507 retains original d2graph object/edge reference earliest non-glob MapKey lines as skipped adapter metadata. Missing references keep Go MaxInt32 semantics at the sequence boundary. This metadata is independent of arena identity and graph JSON/export AST ordering; original D2 MPL-2.0 notices remain. Evidence: .reference/sequence-note-source-order-20261007/source-comparison.md.

Checkpoint508 retains original d2graph.Object.hasEdgeRef and reference ScopeObj ownership in skipped graph metadata. Explicit false distinguishes glob-expanded endpoints from edge declarations; object containment follows reference scope ancestry. Legacy measured graphs without declaration metadata retain explicit-edge/child interpretations and remain unqualified as original reference APIs. Source predicates for55objects and48core tests pass. Evidence: .reference/sequence-reference-note-fill-20261007/source-comparison.md and qualification.json. D2-derived changes retain MPL-2.0 notices.

Checkpoint510 adds the original d2layouts.validateObjectPositions diagnostic and non-glob sequence note reference-scope metadata. The error uses existing Go quotation, signed infinity spelling and six-decimal finite coordinates; NaN alone is accepted by this validator. All48core tests pass; actual original private validator controls are checked by layout tests against sequence-invalid-positions-source-v090.json. Evidence: .reference/sequence-infinite-positions-20261007/qualification.json. Other position/error/API contracts remain open. D2-derived changes retain MPL-2.0 notices.

Checkpoint511: Compiler provenance separates explicit font declarations from cached defaults and retains actual Object.Level across extraction. tala_text_font_size resolves ordinary Object.Text defaults at the actual TALA adapter boundary; cached dimensions and legacy measured API font inputs are preserved. Structured headers retain their existing adapter path and are not newly qualified. Evidence: .reference/sequence-extracted-grid-20261007/source-comparison.md and qualification.json. Full text/style/routing, structured-header and broader PLAN gates remain open. Existing MPL-2.0 and third-party notices are preserved.

Checkpoint605: Original d2graph Connect/initIndex/IsEquivalent/AbsID and d2compiler Compile sorting map to native retained edge indices, graph serialization/projection and WASM source_index/abs_id metadata. Forty repeated source controls and the native/browser API regression fixtures cover the stated identities; complete graph/API parity remains open. See .reference/compiled-edge-index-native-20261009/source-comparison.md and qualification.json. D2-derived changes retain MPL-2.0 notices.

Checkpoint 876: Retain original config data independently of parsed TALA seed options; target serialization preserves all original scalar/array values.

## Borrowed scalar text (Lane L86, master 6a7b48378)

`ast/composite.rs::ScalarBox::with_scalar_text` and `scalar_string` implement
`d2ast/d2ast.go:585–608,1492–1517` without copying interpolation bytes through
an intermediate buffer. Read guards bound the borrowed presentation; independent
box slots keep Unbox precedence, strings select only the first interpolation
slot, and invalid UTF-8 retains the existing one-invalid-byte presentation.
Regression tests cover all scalar kinds, malformed bytes, absent/extra
interpolation slots and multi-slot boxes.
