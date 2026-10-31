# Authored forced-hierarchy routing witness — 2026-10-02

[The fixture](/Users/nelson/code/d2-rs/tests/d2/forced-hierarchy-route-trace.d2)
selects TALA's actual `TopDownLeftRight` routing carrier and emits all five
ordered search families for seeds 1 and 42, each repeated twice. Its authored
CLI outputs also pass the existing bounded SVG predicates and eight exact
decoded PNG comparisons against the official D2 v0.9.0 executable. The old
measured Go observer still fails all four strict trace comparisons because it
does not emit the native heap's `work`, `scope_nodes` and `scope_edges` fields.
Those failures remain recorded. This is bounded witness qualification, not
full operation-trace or D2 parity.

The [machine-readable provenance record](/Users/nelson/code/d2-rs/docs/tala-forced-hierarchy-route-witness-20261002.json)
binds the fixtures, binaries, manifests, source snapshots, unchanged helpers,
capture drivers, raw reports, controls and limits with complete SHA-256 hashes.
The current fixture is 547 bytes with SHA-256
`534fb3d9ab6ae324a3d42cde282f40fd43f0c2a9fde7986938f194567182b3f2`.

## Source contract and preserved unsuccessful attempt

The source is pristine D2 v0.9.0 at
`d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda`.
[TestLayoutKeepsForcedHierarchyWithUnrelatedFixedNode](/Users/nelson/code/d2/d2layouts/d2talalayout/fixed_hierarchy_test.go:11)
creates an eight-node DAG under root `shape: hierarchy`, with requested node
dimensions 80 by 50. It additionally creates an unrelated fixed-position node
through the graph API. The authored fixture preserves the eight-node and
seven-edge declaration order, but omits that unrelated node. Its identifier
labels go through ordinary compilation and text measurement; the original
[programmatic constructor](/Users/nelson/code/d2/d2layouts/d2talalayout/adapter_test.go:1234)
supplies empty labels. This fixture therefore does not qualify the original
programmatic fixed-node contract.

The first authored attempt retained the fixed node. Pinned Go compilation
rejected its `top` and `left` properties with `position keywords cannot be used
with shape "hierarchy"`. This follows the
[compiler restriction](/Users/nelson/code/d2/d2compiler/compile.go:1287) and its
[regression case](/Users/nelson/code/d2/d2compiler/compile_test.go:3800). That
attempt made one measurement call and zero layout calls; all four planned rows
remain `capture_failed`. Its source, error and failure report are preserved in
[the first attempt record](/Users/nelson/code/d2-rs/.reference/tala-forced-hierarchy-route-witness-20261002/attempt-1-result.json).
The frozen rejected source has hash
`c3fbe9d49f180042ba66d6c6925baa3ca6708da8c7394163ed3fadce117271b9`.
The tracked fixture was changed only after this failed capture completed.

The [adapter's root flag](/Users/nelson/code/d2/d2layouts/d2talalayout/adapter.go:171)
forces [root hierarchy discovery](/Users/nelson/code/d2/d2layouts/d2talalayout/internal/hierarchy/discovery.go:111).
The [routing coordinator](/Users/nelson/code/d2/d2layouts/d2talalayout/internal/routing/coordinator.go:483)
selects only `TopDownLeftRight` when the first scope node carries a hierarchy.
The corresponding frozen native branch is
[captured here](/Users/nelson/code/d2-rs/.reference/tala-forced-hierarchy-route-witness-attempt-2-20261002/sources/crates/d2-tala-rs/src/internal/routing/search.rs:4475).
The old Go `route-flavor` callback is independently enabled by routing tracing,
as shown by [its recorded patch](/Users/nelson/code/d2-rs/tools/patches/d2-v090-stage-trace.patch:5768).
It is not manufactured by the search-flavor filter. Both measured sides
actually reported the sole flavor `TopDownLeftRight` in all four cases.

## Measured layout and ordered search capture

The unchanged frozen wrapper made one measurement call and twenty layout
calls, using pinned plain Go `fd822918…`, old traced Go `c5f1e4d…`, native
diagnostic `30d629d3…` and feature-off `12d33edc…`. Their complete hashes and
Go provenance are in the JSON record. The native pair is bound to terminal
build report SHA-256
`8a72fc9a7e899e3e5a1cc83c9b594cc8196e6b887e731872eead0e357a79dd66`.

| Seed | Heap | Pop | Neighbor | Cost breakdown | Cost | Actual flavor |
|---|---:|---:|---:|---:|---:|---|
| 1 | 835 | 252 | 944 | 700 | 700 | TopDownLeftRight |
| 42 | 782 | 244 | 908 | 672 | 672 | TopDownLeftRight |

Counts are per run and match both sides; each row was repeated twice. The four
captures contain 13,418 selected search events per side in actual interleaved
order. The public graph comparator returned no difference; observer on/off,
feature-off and full emitted-stream repeatability controls passed. The four
same-level node pairs and increasing level Y values also passed the original
test's applicable subset assertions. The
[independent raw qualification](/Users/nelson/code/d2-rs/.reference/tala-forced-hierarchy-route-witness-attempt-2-20261002/runtime-report.json)
verified 1,199 artifact records without running engines or compilers.

The [strict capture](/Users/nelson/code/d2-rs/.reference/tala-forced-hierarchy-route-capture-attempt-2-20261002/report.json)
remains terminal **FAIL**, with four `divergent` rows. The first supplemental
exact comparison differs at `$[0]`: native emits `scope_edges`, `scope_nodes`
and `work`, which the old Go heap observer lacks. This is an observability gap;
it does not establish an algorithm difference in those absent values. No fields
were removed to change the verdict. Newer parent-owned enriched Go captures
are separate evidence and do not rewrite this older binary's result.

The original comparison uses Python parsed scalar equality, which can equate
signed zero. The supplemental unchanged exact-JSON comparison checks ordered
emitted IEEE number representations and hexadecimal bit strings after the
wrapper's declared transformations. It cannot recover un-emitted values or
NaN payloads. The original stage-seed and adapter-local edge-ID omissions,
un-emitted internal operations and cross-family/global worker scheduling remain
unqualified. There is no epsilon, event sorting, LCS or owner projection in this
capture.

## Official authored CLI and exact pixels

The [authored CLI capture](/Users/nelson/code/d2-rs/.reference/tala-forced-hierarchy-authored-cli-20261002/report.json)
is terminal **PASS** for this fixture. It used the verified official executable
`e8ba501a…` with `--layout=tala`, and the current frozen native CLI diagnostic
`4fa7099b…` and feature-off `84d69647…` with `--layout=tala-rs`. The native source
set is bound to 549 compiled files in
[the terminal build manifest](/Users/nelson/code/d2-rs/docs/tala-cluster-alignment-max-size-tests-build-20261002.json).
The fresh capture copied the binaries, fixture, compiled source, helpers and
source references into its exclusive directory; 562 snapshot pairs stayed
unchanged. It performed 36 CLI calls and no compiler calls: twenty SVG calls,
twelve ordinary PNG calls and four PNG observer controls.

The existing [SVG boundary helper](/Users/nelson/code/d2-rs/scripts/check-dagre-parity.py:64)
was used unchanged. It rounds numbers/path coordinates to six decimals, sorts
the shape inventory and identifies routes by semantic edge. All geometry and
route predicates passed for both native profiles. This does not qualify exact
IEEE geometry, full SVG structure, styling or byte equality with the official
SVG. Official and native SVG serialization actually differ.

| Seed | Decoded PNG dimensions | Exact RGB SHA-256 |
|---|---|---|
| 1 | 904 × 1344 | `cdacd11dd1ddb7e3064f4ca7b86e1327cdbafed1a91f9c9ea6b36bd73d77f9ed` |
| 42 | 844 × 1344 | `4b6b0fdf8c93adc4ab2086c7c4930f769dd92e9cddf3340a1335438e8d8abbd4` |

All eight official/native profile-by-case decoded pixel comparisons have zero
differing pixels. All sixteen PNG outputs and observer controls independently
passed chunk boundaries/order, every CRC, complete zlib-stream and full
scanline/filter validation. They are noninterlaced eight-bit RGB with no `tRNS`,
so alpha is exactly 255. RGBA or transparency would be rejected, not discarded.
Four distinct compressed streams were fully decoded with the unchanged
previously tested [PNG decoder](/Users/nelson/code/d2-rs/scripts/check-render-parity.py).
Compression/chunk metadata are outside the pixel predicate.

Ordinary SVG and PNG repeats are byte-equal on each side. Native diagnostic and
feature-off outputs are byte-equal; enabling grid/final-layout or raster-pixel
observers leaves same-side output bytes unchanged. Enabled diagnostics emit
observer records, while the feature-off profile emits none. The harness clears
all controlled trace prefixes and development/test overrides before each run,
and pins one Go/Rust worker.

The [independent replay](/Users/nelson/code/d2-rs/.reference/tala-forced-hierarchy-authored-cli-20261002/independent-verification.json)
checked 1,785 artifact records, re-extracted all twenty SVG files, validated all
sixteen PNG files and reconstructed the eight exact pixel comparisons without
engine or compiler calls. Its SHA-256 is
`c6136e444753151755013ef80df6a4debb92498a6f4956fb703e5ddd0888d27e`.
These results cover this fixture, two single seeds, these frozen binaries and
their actual renderers. They do not qualify concurrency, cancellation,
transparency, other formats/platforms, full rendering coverage or complete D2
parity. Rust/Go sources, shared helpers, target aliases, global harnesses and
`STATUS.md` were not changed for this witness task.
